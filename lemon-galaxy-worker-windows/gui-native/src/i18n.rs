//! 中英两套文案（`tr!("中文", "English")`，与 Linux 版同样的写法），文字尽量沿用 WPF 版的语言包。
//!
//! 语言：环境变量 `LEMON_GALAXY_LANG` > 界面设置里选的 > 系统界面语言（中文系统用中文，其余英文）。
//! 设置里换语言时整个窗口重建（`ui::window`），所以这里是可改的全局开关。

use std::sync::atomic::{AtomicBool, Ordering};

static ZH: AtomicBool = AtomicBool::new(false);

pub fn zh() -> bool {
    ZH.load(Ordering::Relaxed)
}

/// `setting`：界面设置里的语言（`system`、`zh-Hans`、`en`）
#[cfg_attr(not(windows), allow(dead_code))]
pub fn apply(setting: &str) {
    let zh = match std::env::var("LEMON_GALAXY_LANG").ok().map(|v| v.trim().to_ascii_lowercase()).filter(|v| !v.is_empty()) {
        Some(v) => v.starts_with("zh"),
        None => match setting {
            "zh-Hans" | "zh" => true,
            "en" => false,
            _ => system_is_chinese(),
        },
    };
    ZH.store(zh, Ordering::Relaxed);
}

#[cfg(windows)]
fn system_is_chinese() -> bool {
    // LANG_CHINESE = 0x04（简体、繁体都算）
    let id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    id & 0x3ff == 0x04
}

#[cfg(not(windows))]
fn system_is_chinese() -> bool {
    std::env::var("LANG").map(|v| v.starts_with("zh")).unwrap_or(false)
}

/// `tr!("中文", "English")`
#[macro_export]
macro_rules! tr {
    ($zh:expr, $en:expr) => {
        if $crate::i18n::zh() {
            $zh
        } else {
            $en
        }
    };
}

/// 服务端用的语言标记（服务回的错误信息用这个语言）
#[cfg_attr(not(windows), allow(dead_code))]
pub fn lang_tag() -> &'static str {
    if zh() {
        "zh-CN"
    } else {
        "en"
    }
}

/// 简单的 `{0}`、`{1}` 替换（文案里带参数的地方用）
#[cfg_attr(not(windows), allow(dead_code))]
pub fn fmt(template: &str, args: &[&str]) -> String {
    let mut s = template.to_string();
    for (i, a) in args.iter().enumerate() {
        s = s.replace(&format!("{{{i}}}"), a);
    }
    s
}
