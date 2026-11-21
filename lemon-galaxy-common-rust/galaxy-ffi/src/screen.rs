//! 终端屏幕模型：给自己绘制终端的界面用（Windows 主控端）。把被控端的终端输出喂进来，取出带颜色的行。
//! Mac / 安卓用各自平台的终端控件，不需要它。

use parking_lot::Mutex;
use std::num::NonZeroU16;
use std::sync::Arc;

fn nz(v: u16) -> NonZeroU16 {
    NonZeroU16::new(v.max(1)).unwrap_or(NonZeroU16::MIN)
}

/// 收集标题与响铃（vt100 用回调上报）。
#[derive(Default)]
struct Events {
    title: String,
    bells: u32,
}

impl vt100::Callbacks for Events {
    fn audible_bell(&mut self, _: &mut vt100::Screen) {
        self.bells = self.bells.wrapping_add(1);
    }
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.title = String::from_utf8_lossy(title).to_string();
    }
}

/// 颜色编码：0 默认色；0x0100_00nn 调色板第 nn 号；0x0200_0000 | RRGGBB 真彩色。
fn color(c: vt100::Color) -> u32 {
    match c {
        vt100::Color::Default => 0,
        vt100::Color::Idx(i) => 0x0100_0000 | i as u32,
        vt100::Color::Rgb(r, g, b) => 0x0200_0000 | (r as u32) << 16 | (g as u32) << 8 | b as u32,
    }
}

pub const SPAN_BOLD: u8 = 1;
pub const SPAN_ITALIC: u8 = 2;
pub const SPAN_UNDERLINE: u8 = 4;
pub const SPAN_INVERSE: u8 = 8;

/// 一行里样式相同的一段。`cells` 是占的列数（全角字占 2 列）。
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct TermSpan {
    pub text: String,
    pub cells: u16,
    pub fg: u32,
    pub bg: u32,
    /// 1 粗体、2 斜体、4 下划线、8 反色
    pub flags: u8,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct TermLine {
    pub spans: Vec<TermSpan>,
    /// 这一行是上一行自动折行过来的（复制时不加换行）
    pub wrapped: bool,
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TermMouseMode {
    None,
    Press,
    PressRelease,
    ButtonMotion,
    AnyMotion,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct TermScreenState {
    pub rows: u16,
    pub cols: u16,
    pub lines: Vec<TermLine>,
    pub cursor_row: u16,
    pub cursor_col: u16,
    /// 程序隐藏了光标，或正在往上翻看历史
    pub cursor_visible: bool,
    pub title: String,
    pub alternate_screen: bool,
    pub application_cursor: bool,
    pub application_keypad: bool,
    pub bracketed_paste: bool,
    pub mouse_mode: TermMouseMode,
    /// 鼠标上报用 SGR 编码
    pub mouse_sgr: bool,
    /// 当前往上翻看的行数（0 为最底部）
    pub scroll_offset: u32,
    /// 历史里一共有多少行
    pub scrollback_len: u32,
    /// 响铃计数（变化时界面提示一下）
    pub bell_count: u32,
}

#[derive(uniffi::Object)]
pub struct TerminalScreen {
    parser: Mutex<vt100::Parser<Events>>,
}

#[uniffi::export]
impl TerminalScreen {
    #[uniffi::constructor]
    pub fn new(rows: u16, cols: u16, scrollback: u32) -> Arc<Self> {
        Arc::new(Self { parser: Mutex::new(vt100::Parser::new_with_callbacks(nz(rows.max(2)), nz(cols.max(10)), scrollback.min(100_000) as usize, Events::default())) })
    }

    /// 喂入被控端终端的输出。
    pub fn feed(&self, data: Vec<u8>) {
        self.parser.lock().process(&data);
    }

    pub fn resize(&self, rows: u16, cols: u16) {
        self.parser.lock().set_size(nz(rows.max(2)), nz(cols.max(10)));
    }

    /// 取屏幕内容。`scroll_offset` 为往上翻看的行数，0 为最底部（超出历史长度时按最大值算）。
    pub fn state(&self, scroll_offset: u32) -> TermScreenState {
        let mut p = self.parser.lock();
        let total = p.screen().scrollback_len() as u32;
        let off = scroll_offset.min(total);
        p.screen_mut().set_scrollback(off as usize);
        let (title, bells) = (p.callbacks().title.clone(), p.callbacks().bells);
        let s = p.screen();
        let (rows, cols) = s.size();
        let (rows, cols) = (rows.get(), cols.get());
        let mut lines = Vec::with_capacity(rows as usize);
        for r in 0..rows {
            let mut spans: Vec<TermSpan> = Vec::new();
            for c in 0..cols {
                let Some(cell) = s.cell(r, c) else { continue };
                if cell.is_wide_continuation() {
                    continue;
                }
                let mut flags = 0u8;
                if cell.bold() {
                    flags |= SPAN_BOLD;
                }
                if cell.italic() {
                    flags |= SPAN_ITALIC;
                }
                if cell.underline() {
                    flags |= SPAN_UNDERLINE;
                }
                if cell.inverse() {
                    flags |= SPAN_INVERSE;
                }
                let (fg, bg) = (color(cell.fgcolor()), color(cell.bgcolor()));
                let text = if cell.has_contents() { cell.contents().to_string() } else { " ".to_string() };
                let w: u16 = if cell.is_wide() { 2 } else { 1 };
                match spans.last_mut() {
                    Some(l) if l.fg == fg && l.bg == bg && l.flags == flags => {
                        l.text.push_str(&text);
                        l.cells += w;
                    }
                    _ => spans.push(TermSpan { text, cells: w, fg, bg, flags }),
                }
            }
            // 行尾的默认样式空白去掉，画起来更快
            if let Some(l) = spans.last_mut() {
                if l.fg == 0 && l.bg == 0 && l.flags == 0 {
                    let trimmed = l.text.trim_end_matches(' ');
                    let cut = l.text.len() - trimmed.len();
                    if cut > 0 {
                        l.cells -= cut as u16;
                        l.text.truncate(trimmed.len());
                    }
                    if l.text.is_empty() {
                        spans.pop();
                    }
                }
            }
            lines.push(TermLine { spans, wrapped: r > 0 && s.row_wrapped(r - 1) });
        }
        let (cr, cc) = s.cursor_position();
        let mouse_mode = match s.mouse_protocol_mode() {
            vt100::MouseProtocolMode::None => TermMouseMode::None,
            vt100::MouseProtocolMode::Press => TermMouseMode::Press,
            vt100::MouseProtocolMode::PressRelease => TermMouseMode::PressRelease,
            vt100::MouseProtocolMode::ButtonMotion => TermMouseMode::ButtonMotion,
            vt100::MouseProtocolMode::AnyMotion => TermMouseMode::AnyMotion,
        };
        TermScreenState {
            rows,
            cols,
            lines,
            cursor_row: cr,
            cursor_col: cc,
            cursor_visible: !s.hide_cursor() && off == 0,
            title,
            alternate_screen: s.alternate_screen(),
            application_cursor: s.application_cursor(),
            application_keypad: s.application_keypad(),
            bracketed_paste: s.bracketed_paste(),
            mouse_mode,
            mouse_sgr: matches!(s.mouse_protocol_encoding(), vt100::MouseProtocolEncoding::Sgr),
            scroll_offset: off,
            scrollback_len: total,
            bell_count: bells,
        }
    }

    /// 可见区域里两点之间的文字（选中复制用），行列从 0 开始，含终点。
    pub fn text_between(&self, start_row: u16, start_col: u16, end_row: u16, end_col: u16) -> String {
        self.parser.lock().screen().contents_between(start_row, start_col, end_row, end_col.saturating_add(1))
    }

    /// 可见区域全部文字。
    pub fn text(&self) -> String {
        self.parser.lock().screen().contents()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_and_colors() {
        let t = TerminalScreen::new(5, 20, 100);
        t.feed(b"ab\x1b[31mred\x1b[0m \xe4\xb8\xad\r\n".to_vec());
        let s = t.state(0);
        let l = &s.lines[0];
        assert_eq!(l.spans[0].text, "ab");
        assert_eq!(l.spans[1].text, "red");
        assert_eq!(l.spans[1].fg, 0x0100_0001);
        assert_eq!(l.spans[2].text, " 中");
        assert_eq!(l.spans[2].cells, 3);
        assert_eq!(s.cursor_row, 1);
    }

    #[test]
    fn scrollback() {
        let t = TerminalScreen::new(3, 10, 100);
        for i in 0..10 {
            t.feed(format!("line{i}\r\n").into_bytes());
        }
        let s = t.state(5);
        assert_eq!(s.scroll_offset, 5);
        assert!(s.scrollback_len >= 7);
        assert!(!s.cursor_visible);
        assert_eq!(t.state(0).scroll_offset, 0);
    }
}
