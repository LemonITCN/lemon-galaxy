//! 终端里的逐级菜单：方向键或数字选择、回车确认、Esc 返回、Ctrl+C 退出。
//!
//! 只在按键时切到原始模式（Unix 用 termios，Windows 用控制台模式 + 虚拟终端输入），不引第三方库。
//! 终端不支持 ANSI 控制码时（很老的 Windows 控制台、TERM=dumb）退回「打印编号、输入数字回车」的朴素模式。

use std::io::{Read, Write};
use std::sync::OnceLock;

/// 用户按了 Ctrl+C / Ctrl+D：退出整个菜单。
#[derive(Debug)]
pub struct Quit;

impl std::fmt::Display for Quit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("quit")
    }
}

impl std::error::Error for Quit {}

pub fn is_quit(e: &anyhow::Error) -> bool {
    e.downcast_ref::<Quit>().is_some()
}

#[derive(Clone)]
pub struct Item {
    pub key: String,
    pub label: String,
    pub hint: String,
}

impl Item {
    pub fn new(key: impl Into<String>, label: impl Into<String>, hint: impl Into<String>) -> Self {
        Item { key: key.into(), label: label.into(), hint: hint.into() }
    }
}

// ---------------- 颜色 ----------------

static ANSI: OnceLock<bool> = OnceLock::new();

/// 终端是否支持 ANSI 控制码（Windows 上顺带打开虚拟终端处理）。
pub fn ansi() -> bool {
    *ANSI.get_or_init(|| {
        if std::env::var("TERM").map(|t| t == "dumb").unwrap_or(false) {
            return false;
        }
        sys::enable_vt_output()
    })
}

fn wrap(code: &str, s: &str) -> String {
    if ansi() {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

pub fn bold(s: &str) -> String {
    wrap("1", s)
}
pub fn dim(s: &str) -> String {
    wrap("2", s)
}
pub fn green(s: &str) -> String {
    wrap("32", s)
}
pub fn red(s: &str) -> String {
    wrap("31", s)
}
pub fn yellow(s: &str) -> String {
    wrap("33", s)
}
pub fn cyan(s: &str) -> String {
    wrap("36", s)
}

/// 显示宽度：去掉 ANSI 控制码，中日韩全角字符算 2 列。
pub fn display_width(s: &str) -> usize {
    let mut w = 0;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // CSI：ESC [ ... 终止字节 0x40–0x7e
            if chars.peek() == Some(&'[') {
                chars.next();
                for d in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&d) {
                        break;
                    }
                }
            }
            continue;
        }
        w += char_width(c);
    }
    w
}

fn char_width(c: char) -> usize {
    let u = c as u32;
    if u < 0x20 || (0x7f..0xa0).contains(&u) || (0x300..0x370).contains(&u) || (0x200b..=0x200f).contains(&u) || (0xfe00..=0xfe0f).contains(&u) {
        return 0;
    }
    let wide = (0x1100..=0x115f).contains(&u)
        || (0x2e80..=0x303e).contains(&u)
        || (0x3041..=0x33ff).contains(&u)
        || (0x3400..=0x4dbf).contains(&u)
        || (0x4e00..=0x9fff).contains(&u)
        || (0xa000..=0xa4cf).contains(&u)
        || (0xac00..=0xd7a3).contains(&u)
        || (0xf900..=0xfaff).contains(&u)
        || (0xfe30..=0xfe4f).contains(&u)
        || (0xff00..=0xff60).contains(&u)
        || (0xffe0..=0xffe6).contains(&u)
        || (0x1f300..=0x1f64f).contains(&u)
        || (0x1f900..=0x1f9ff).contains(&u)
        || (0x20000..=0x3fffd).contains(&u);
    if wide {
        2
    } else {
        1
    }
}

fn pad_right(s: &str, w: usize) -> String {
    let d = w.saturating_sub(display_width(s));
    format!("{s}{}", " ".repeat(d))
}

/// 一段文本在终端里占几行（长行会折行）。
fn rows_of(lines: &[String], cols: usize) -> usize {
    lines.iter().map(|l| display_width(l).max(1).div_ceil(cols.max(1))).sum()
}

/// 把一次读到的输入拆成单个按键：转义序列（方向键等）算一个键，其余按字符拆。
pub fn split_keys(s: &str) -> Vec<String> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let mut n;
        if b[i] == 0x1b && b.len() - i >= 3 && b[i + 1] == b'[' {
            n = 2;
            while i + n < b.len() && !(0x40..=0x7e).contains(&b[i + n]) {
                n += 1;
            }
            n = (n + 1).min(b.len() - i);
        } else if b[i] == 0x1b && b.len() - i >= 3 && b[i + 1] == b'O' {
            n = 3;
        } else if b[i] == 0x1b {
            n = 1;
        } else {
            n = s[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        }
        out.push(s[i..i + n].to_string());
        i += n;
    }
    out
}

// ---------------- 原始模式 ----------------

#[cfg(unix)]
mod sys {
    pub struct Raw(libc::termios);

    impl Raw {
        pub fn enable() -> Option<Raw> {
            unsafe {
                let mut t: libc::termios = std::mem::zeroed();
                if libc::tcgetattr(0, &mut t) != 0 {
                    return None;
                }
                let orig = t;
                // 关掉行缓冲、回显、信号键；保留输出处理（\n 照常换行）
                t.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN);
                t.c_iflag &= !(libc::IXON | libc::ICRNL | libc::BRKINT | libc::INPCK | libc::ISTRIP);
                t.c_cc[libc::VMIN] = 1;
                t.c_cc[libc::VTIME] = 0;
                if libc::tcsetattr(0, libc::TCSANOW, &t) != 0 {
                    return None;
                }
                Some(Raw(orig))
            }
        }
    }

    impl Drop for Raw {
        fn drop(&mut self) {
            unsafe {
                libc::tcsetattr(0, libc::TCSANOW, &self.0);
            }
        }
    }

    pub fn enable_vt_output() -> bool {
        true
    }

    pub fn cols() -> usize {
        unsafe {
            let mut ws: libc::winsize = std::mem::zeroed();
            if libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 {
                return ws.ws_col as usize;
            }
        }
        80
    }
}

#[cfg(windows)]
mod sys {
    use std::ffi::c_void;

    const STD_INPUT_HANDLE: u32 = -10i32 as u32;
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const ENABLE_PROCESSED_INPUT: u32 = 0x1;
    const ENABLE_LINE_INPUT: u32 = 0x2;
    const ENABLE_ECHO_INPUT: u32 = 0x4;
    const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x200;
    const ENABLE_PROCESSED_OUTPUT: u32 = 0x1;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x4;

    #[repr(C)]
    #[derive(Default)]
    struct Coord {
        x: i16,
        y: i16,
    }
    #[repr(C)]
    #[derive(Default)]
    struct SmallRect {
        left: i16,
        top: i16,
        right: i16,
        bottom: i16,
    }
    #[repr(C)]
    #[derive(Default)]
    struct ScreenBufferInfo {
        size: Coord,
        cursor: Coord,
        attrs: u16,
        window: SmallRect,
        max_size: Coord,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(n: u32) -> *mut c_void;
        fn GetConsoleMode(h: *mut c_void, mode: *mut u32) -> i32;
        fn SetConsoleMode(h: *mut c_void, mode: u32) -> i32;
        fn GetConsoleScreenBufferInfo(h: *mut c_void, info: *mut ScreenBufferInfo) -> i32;
    }

    pub struct Raw(u32);

    impl Raw {
        pub fn enable() -> Option<Raw> {
            unsafe {
                let h = GetStdHandle(STD_INPUT_HANDLE);
                let mut mode = 0u32;
                if GetConsoleMode(h, &mut mode) == 0 {
                    return None;
                }
                let raw = (mode & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT)) | ENABLE_VIRTUAL_TERMINAL_INPUT;
                if SetConsoleMode(h, raw) == 0 {
                    return None;
                }
                Some(Raw(mode))
            }
        }
    }

    impl Drop for Raw {
        fn drop(&mut self) {
            unsafe {
                SetConsoleMode(GetStdHandle(STD_INPUT_HANDLE), self.0);
            }
        }
    }

    /// 打开控制台的虚拟终端处理（Windows 10 1511 起）。失败说明不认 ANSI 控制码。
    pub fn enable_vt_output() -> bool {
        unsafe {
            let h = GetStdHandle(STD_OUTPUT_HANDLE);
            let mut mode = 0u32;
            if GetConsoleMode(h, &mut mode) == 0 {
                return false;
            }
            mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0 || SetConsoleMode(h, mode | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0
        }
    }

    pub fn cols() -> usize {
        unsafe {
            let mut info = ScreenBufferInfo::default();
            if GetConsoleScreenBufferInfo(GetStdHandle(STD_OUTPUT_HANDLE), &mut info) != 0 {
                let w = info.window.right - info.window.left + 1;
                if w > 0 {
                    return w as usize;
                }
            }
        }
        80
    }
}

// ---------------- 菜单 ----------------

pub struct Tui {
    /// 键位说明（随语言）
    pub keys_help: String,
    pub keys_help_top: String,
    pub input_help: String,
    pub pause_text: String,
    pub yes: String,
    pub no: String,
    pub plain_prompt: String,
}

fn out() -> std::io::Stdout {
    std::io::stdout()
}

fn flush() {
    let _ = out().flush();
}

/// 读一次输入，拆成按键。
fn read_keys() -> anyhow::Result<Vec<String>> {
    let mut buf = [0u8; 512];
    let n = std::io::stdin().lock().read(&mut buf)?;
    if n == 0 {
        return Err(Quit.into());
    }
    Ok(split_keys(&String::from_utf8_lossy(&buf[..n])))
}

/// 朴素模式读一行（不支持 ANSI 或切不了原始模式时）。
fn read_line() -> anyhow::Result<String> {
    let mut s = String::new();
    if std::io::stdin().read_line(&mut s)? == 0 {
        return Err(Quit.into());
    }
    Ok(s.trim().to_string())
}

impl Tui {
    /// 显示菜单，返回选中项的 key；Esc / q / ← 返回 None（回上一级）；Ctrl+C 返回 Quit。
    /// top 为主菜单：Esc 与 ← 不退出（避免误按），只有 q 退出。
    pub fn choose(&self, title: &str, items: &[Item], sel: usize, top: bool) -> anyhow::Result<Option<String>> {
        if items.is_empty() {
            return Ok(None);
        }
        let raw = if ansi() { sys::Raw::enable() } else { None };
        let Some(raw) = raw else { return self.choose_plain(title, items) };
        let mut sel = sel.min(items.len() - 1);
        let label_w = items.iter().map(|i| display_width(&i.label)).max().unwrap_or(0);
        let help = if top { &self.keys_help_top } else { &self.keys_help };
        let mut rows = 0usize;
        print!("\x1b[?25l");
        let draw = |sel: usize, rows: &mut usize| {
            let mut lines: Vec<String> = title.split('\n').map(|s| s.to_string()).collect();
            for (i, it) in items.iter().enumerate() {
                let num = if i < 9 { format!("{}.", i + 1) } else { "  ".to_string() };
                let label = pad_right(&it.label, label_w);
                let hint = if it.hint.is_empty() { String::new() } else { format!("   {}", dim(&it.hint)) };
                if i == sel {
                    lines.push(format!("  \x1b[36m> {num} \x1b[1m{label}\x1b[0m{hint}"));
                } else {
                    lines.push(format!("    {} {label}{hint}", dim(&num)));
                }
            }
            lines.push(dim(&format!("  {help}")));
            let mut b = String::new();
            if *rows > 1 {
                b.push_str(&format!("\x1b[{}A", *rows - 1));
            }
            b.push_str("\r\x1b[J");
            b.push_str(&lines.join("\r\n"));
            print!("{b}");
            flush();
            *rows = rows_of(&lines, sys::cols());
        };
        let clear = |rows: usize| {
            if rows > 1 {
                print!("\x1b[{}A", rows - 1);
            }
            print!("\r\x1b[J\x1b[?25h");
            flush();
        };
        draw(sel, &mut rows);
        loop {
            let ks = match read_keys() {
                Ok(k) => k,
                Err(e) => {
                    clear(rows);
                    return Err(e);
                }
            };
            for k in ks {
                match k.as_str() {
                    "\x03" | "\x04" => {
                        clear(rows);
                        drop(raw);
                        return Err(Quit.into());
                    }
                    "\r" | "\n" | "\x1b[C" | "\x1bOC" => {
                        clear(rows);
                        println!("{} {}\r", cyan(">"), items[sel].label);
                        return Ok(Some(items[sel].key.clone()));
                    }
                    "\x1b[A" | "\x1bOA" | "k" | "\x10" => sel = (sel + items.len() - 1) % items.len(),
                    "\x1b[B" | "\x1bOB" | "j" | "\x0e" | "\t" => sel = (sel + 1) % items.len(),
                    "\x1b[H" | "\x1bOH" | "\x1b[1~" => sel = 0,
                    "\x1b[F" | "\x1bOF" | "\x1b[4~" => sel = items.len() - 1,
                    "\x1b" | "q" | "Q" | "\x1b[D" | "\x1bOD" | "\x7f" | "\x08" => {
                        if !top || k == "q" || k == "Q" {
                            clear(rows);
                            return Ok(None);
                        }
                    }
                    _ => {
                        if let Some(d) = k.chars().next().and_then(|c| c.to_digit(10)).filter(|_| k.len() == 1) {
                            if d >= 1 && (d as usize) <= items.len() {
                                sel = d as usize - 1;
                                clear(rows);
                                println!("{} {}\r", cyan(">"), items[sel].label);
                                return Ok(Some(items[sel].key.clone()));
                            }
                        }
                    }
                }
            }
            draw(sel, &mut rows);
        }
    }

    fn choose_plain(&self, title: &str, items: &[Item]) -> anyhow::Result<Option<String>> {
        loop {
            println!("{title}");
            for (i, it) in items.iter().enumerate() {
                let hint = if it.hint.is_empty() { String::new() } else { format!("   {}", it.hint) };
                println!("  {:>2}. {}{hint}", i + 1, it.label);
            }
            print!("{}", self.plain_prompt);
            flush();
            let s = read_line()?;
            if s.is_empty() || s == "q" || s == "0" {
                return Ok(None);
            }
            if let Ok(n) = s.parse::<usize>() {
                if (1..=items.len()).contains(&n) {
                    return Ok(Some(items[n - 1].key.clone()));
                }
            }
        }
    }

    /// 读一行文字：def 是直接回车时的默认值；Esc 返回 None。mask 为真时输入显示成 *。
    pub fn input(&self, label: &str, def: &str, hint: &str, mask: bool) -> anyhow::Result<Option<String>> {
        if !hint.is_empty() {
            println!("{}", dim(&format!("  {hint}")));
        }
        let raw = if ansi() { sys::Raw::enable() } else { None };
        let Some(raw) = raw else {
            let d = if def.is_empty() { String::new() } else { format!(" [{def}]") };
            print!("  {label}{d}: ");
            flush();
            let s = read_line()?;
            return Ok(Some(if s.is_empty() { def.to_string() } else { s }));
        };
        let mut val: Vec<char> = Vec::new();
        let mut rows = 0usize;
        let draw = |val: &[char], done: bool, rows: &mut usize| {
            let shown: String = if mask { "*".repeat(val.len()) } else { val.iter().collect() };
            let mut line = format!("  {}: ", bold(label));
            if !val.is_empty() {
                line.push_str(&shown);
            } else if !def.is_empty() {
                line.push_str(&if done { def.to_string() } else { dim(def) });
            }
            if !done {
                line.push_str(&format!("  {}", dim(&self.input_help)));
            }
            let mut b = String::new();
            if *rows > 1 {
                b.push_str(&format!("\x1b[{}A", *rows - 1));
            }
            b.push_str("\r\x1b[J");
            b.push_str(&line);
            print!("{b}");
            flush();
            *rows = rows_of(&[line], sys::cols());
        };
        draw(&val, false, &mut rows);
        loop {
            let ks = read_keys()?;
            for k in ks {
                match k.as_str() {
                    "\x03" | "\x04" => {
                        println!("\r");
                        drop(raw);
                        return Err(Quit.into());
                    }
                    "\x1b" => {
                        draw(&[], true, &mut rows);
                        println!("\r");
                        return Ok(None);
                    }
                    "\r" | "\n" => {
                        draw(&val, true, &mut rows);
                        println!("\r");
                        let s: String = val.iter().collect();
                        return Ok(Some(if s.is_empty() { def.to_string() } else { s.trim().to_string() }));
                    }
                    "\x7f" | "\x08" => {
                        val.pop();
                    }
                    "\x15" => val.clear(),
                    k if k.starts_with('\x1b') => {}
                    k => {
                        for c in k.chars() {
                            if c >= ' ' && c != '\x7f' {
                                val.push(c);
                            }
                        }
                    }
                }
            }
            draw(&val, false, &mut rows);
        }
    }

    /// 是 / 否；默认选中「否」。
    pub fn confirm(&self, question: &str) -> anyhow::Result<bool> {
        let items = [Item::new("yes", self.yes.clone(), ""), Item::new("no", self.no.clone(), "")];
        Ok(self.choose(&bold(question), &items, 1, false)?.as_deref() == Some("yes"))
    }

    /// 等用户按回车（或 Esc）再回菜单。
    pub fn pause(&self) -> anyhow::Result<()> {
        print!("{}", dim(&format!("  {}", self.pause_text)));
        flush();
        let raw = if ansi() { sys::Raw::enable() } else { None };
        let Some(_raw) = raw else {
            read_line()?;
            return Ok(());
        };
        loop {
            for k in read_keys()? {
                match k.as_str() {
                    "\x03" | "\x04" => {
                        println!("\r");
                        return Err(Quit.into());
                    }
                    "\r" | "\n" | "\x1b" | "q" | " " => {
                        print!("\r\x1b[K");
                        flush();
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_split() {
        assert_eq!(split_keys("\x1b[A\x1b[B12"), vec!["\x1b[A", "\x1b[B", "1", "2"]);
        assert_eq!(split_keys("\x1b"), vec!["\x1b"]);
        assert_eq!(split_keys("\x1bOA中"), vec!["\x1bOA", "中"]);
        assert_eq!(split_keys("\x1b[1~\r"), vec!["\x1b[1~", "\r"]);
    }

    #[test]
    fn widths() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("中文"), 4);
        assert_eq!(display_width("\x1b[1m中\x1b[0ma"), 3);
        assert_eq!(rows_of(&["中".repeat(50)], 80), 2);
        assert_eq!(rows_of(&[String::new()], 80), 1);
    }
}
