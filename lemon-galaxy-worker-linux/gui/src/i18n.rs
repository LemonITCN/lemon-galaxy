//! 中英两套文案，跟随系统语言（LANGUAGE、LC_ALL、LC_MESSAGES、LANG；LEMON_GALAXY_LANG 可强制）。

use std::sync::OnceLock;

static ZH: OnceLock<bool> = OnceLock::new();

pub fn zh() -> bool {
    *ZH.get_or_init(|| {
        for k in ["LEMON_GALAXY_LANG", "LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"] {
            let Ok(v) = std::env::var(k) else { continue };
            let first = v.split(':').next().unwrap_or("").trim().to_string();
            if first.is_empty() || first == "C" || first == "POSIX" || first.starts_with("C.") {
                continue;
            }
            return first.starts_with("zh");
        }
        false
    })
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

/// 服务端用的语言标记
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn lang_tag() -> &'static str {
    if zh() {
        "zh-CN"
    } else {
        "en"
    }
}
