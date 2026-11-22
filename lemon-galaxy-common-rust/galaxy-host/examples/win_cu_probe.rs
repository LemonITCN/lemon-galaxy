//! 手工验证 Windows 的 AI 应用操作（galaxy_host::win_cu）：按行读请求 JSON 文件逐条执行，
//! 打印返回文字、前台窗口有没有变，截图存到输出目录。要在用户的桌面会话里跑（SSH 会话看不到桌面）。
//!
//!   win_cu_probe.exe requests.jsonl out_dir
//! 请求文件里以 `sleep 毫秒` 开头的行表示等待。

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
    for (i, line) in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).enumerate() {
        if let Some(ms) = line.strip_prefix("sleep ") {
            std::thread::sleep(std::time::Duration::from_millis(ms.trim().parse().unwrap_or(500)));
            continue;
        }
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
                if let Some(img) = img {
                    let p = format!("{out}/{i:02}.jpg");
                    let _ = std::fs::write(&p, &img.data);
                    println!("截图 {}×{} → {p}", img.width, img.height);
                }
            }
        }
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("只在 Windows 上有用");
}
