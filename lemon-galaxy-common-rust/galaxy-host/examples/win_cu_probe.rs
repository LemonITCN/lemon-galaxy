//! 手工验证 Windows 的 AI 应用操作（galaxy_host::win_cu）：按行读请求 JSON 文件逐条执行，
//! 打印返回文字、前台窗口有没有变，截图存到输出目录。要在用户的桌面会话里跑（SSH 会话看不到桌面）。
//!
//!   win_cu_probe.exe requests.jsonl out_dir
//! 请求文件里以 `sleep 毫秒` 开头的行表示等待。请求里可以写占位：
//! - `{{env:名字}}` 换成环境变量（如测试脚本启动的浏览器 pid）；
//! - `{{find:文字1&&文字2}}` 换成最近一次 get_app_state 里第一个同时含这些文字的控件编号，结尾加 `#2` 取第二个（找不到这条请求就跳过）。

#[cfg(windows)]
fn main() {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW};
    let args: Vec<String> = std::env::args().collect();
    let (Some(file), Some(out)) = (args.get(1), args.get(2)) else {
        eprintln!("用法：win_cu_probe requests.jsonl out_dir");
        std::process::exit(2);
    };
    let _ = std::fs::create_dir_all(out);
    let fg = || {
        let mut b = [0u16; 256];
        unsafe {
            let h = GetForegroundWindow();
            let n = GetWindowTextW(h, &mut b);
            format!("{:?}「{}」", h.0, String::from_utf16_lossy(&b[..n.max(0) as usize]))
        }
    };
    let text = std::fs::read_to_string(file).expect("读请求文件");
    println!("开始时的前台窗口：{}", fg());
    let mut last_state = String::new();
    for (i, line) in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).enumerate() {
        if let Some(ms) = line.strip_prefix("sleep ") {
            std::thread::sleep(std::time::Duration::from_millis(ms.trim().parse().unwrap_or(500)));
            continue;
        }
        let Some(line) = fill(line, &last_state) else {
            println!("==== [{i}] {line}\n（占位没找到对应控件，跳过）");
            continue;
        };
        let line = line.as_str();
        let before = fg();
        let t = std::time::Instant::now();
        let r = galaxy_host::win_cu::handle(line);
        let ms = t.elapsed().as_millis();
        let after = fg();
        println!("==== [{i}] {line}  （{ms} ms）");
        if before != after {
            println!("!!!! 前台窗口变了：{before} → {after}");
        }
        // AI 光标窗口的状态
        unsafe {
            use windows::Win32::Foundation::RECT;
            use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowRect, IsWindowVisible};
            let cls: Vec<u16> = "LemonGalaxyAiCursor".encode_utf16().chain(Some(0)).collect();
            if let Ok(h) = FindWindowW(windows::core::PCWSTR(cls.as_ptr()), windows::core::PCWSTR::null()) {
                let mut r = RECT::default();
                let _ = GetWindowRect(h, &mut r);
                println!("（AI 光标：{} 位置 {},{} {}×{}）", if IsWindowVisible(h).as_bool() { "显示" } else { "隐藏" }, r.left, r.top, r.right - r.left, r.bottom - r.top);
            }
        }
        match r {
            None => println!("（不支持）"),
            Some((text, img)) => {
                println!("{text}");
                if line.contains("get_app_state") {
                    last_state = text.clone();
                }
                if let Some(img) = img {
                    let p = format!("{out}/{i:02}.jpg");
                    let _ = std::fs::write(&p, &img.data);
                    println!("截图 {}×{} → {p}", img.width, img.height);
                }
            }
        }
    }
}

/// 换掉请求里的占位；控件找不到时返回 None
#[cfg(windows)]
fn fill(line: &str, state: &str) -> Option<String> {
    let mut s = line.to_string();
    while let Some(a) = s.find("{{") {
        let b = a + s[a..].find("}}")?;
        let key = &s[a + 2..b];
        let val = if let Some(n) = key.strip_prefix("env:") {
            std::env::var(n).unwrap_or_default()
        } else if let Some(q) = key.strip_prefix("find:") {
            // 结尾的 #N 表示第 N 个（从 1 起）
            let (q, nth) = match q.rsplit_once('#').and_then(|(a, n)| Some((a, n.parse::<usize>().ok()?))) {
                Some((a, n)) => (a, n.max(1)),
                None => (q, 1),
            };
            let parts: Vec<&str> = q.split("&&").collect();
            let l = state.lines().filter(|l| parts.iter().all(|p| l.contains(p))).nth(nth - 1)?;
            let t = l.trim_start();
            t[1..t.find(']')?].to_string()
        } else {
            return None;
        };
        s.replace_range(a..b + 2, &val);
    }
    Some(s)
}

#[cfg(not(windows))]
fn main() {
    eprintln!("只在 Windows 上有用");
}
