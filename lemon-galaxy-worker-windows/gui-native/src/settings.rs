//! 界面自己的设置（每个 Windows 用户一份，`%LOCALAPPDATA%\LemonGalaxy\worker-ui\settings.json`，与 WPF 版同一个文件、
//! 同样的键名；不认识的键原样保留）与界面日志（同目录 `ui.log`）。被控服务的设置在服务那边。

use serde_json::{Map, Value};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct AppSettings {
    /// system / zh-Hans / en
    pub language: String,
    /// 登录后在托盘显示图标（安装时写了「所有用户登录后运行」；这里关掉就只对自己不显示）
    pub tray_at_login: bool,
    /// 有人连入时发系统通知
    pub notifications: bool,
    /// 第一次关窗口时提示过「还在托盘里」
    pub told_about_tray: bool,
    /// 窗口大小（96 DPI 下的像素，与 WPF 版的单位相同）
    pub window_width: f64,
    pub window_height: f64,
    rest: Map<String, Value>,
    path: PathBuf,
}

impl AppSettings {
    pub fn load() -> Self {
        let path = crate::setup::ui_data_dir().join("settings.json");
        let mut s = AppSettings { language: "system".into(), tray_at_login: true, notifications: true, told_about_tray: false, window_width: 920.0, window_height: 660.0, rest: Map::new(), path };
        let Ok(text) = std::fs::read_to_string(&s.path) else { return s };
        let Ok(Value::Object(m)) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else {
            log_error("界面设置文件读不懂，按默认值");
            return s;
        };
        let str_ = |k: &str, d: &str| m.get(k).and_then(|v| v.as_str()).unwrap_or(d).to_string();
        let bool_ = |k: &str, d: bool| m.get(k).and_then(|v| v.as_bool()).unwrap_or(d);
        let num = |k: &str, d: f64| m.get(k).and_then(|v| v.as_f64()).unwrap_or(d);
        s.language = str_("language", "system");
        s.tray_at_login = bool_("trayAtLogin", true);
        s.notifications = bool_("notifications", true);
        s.told_about_tray = bool_("toldAboutTray", false);
        s.window_width = num("windowWidth", s.window_width);
        s.window_height = num("windowHeight", s.window_height);
        s.rest = m;
        s
    }

    pub fn save(&self) {
        let mut m = self.rest.clone();
        m.insert("language".into(), self.language.clone().into());
        m.insert("trayAtLogin".into(), self.tray_at_login.into());
        m.insert("notifications".into(), self.notifications.into());
        m.insert("toldAboutTray".into(), self.told_about_tray.into());
        m.insert("windowWidth".into(), self.window_width.into());
        m.insert("windowHeight".into(), self.window_height.into());
        if let Some(d) = self.path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let text = serde_json::to_string_pretty(&Value::Object(m)).unwrap_or_default();
        if let Err(e) = std::fs::write(&self.path, text) {
            log_error(&format!("保存界面设置失败：{e}"));
        }
    }
}

// ---------------- 日志 ----------------

static LOG: Mutex<Option<PathBuf>> = Mutex::new(None);

/// 日志写到 `ui.log`；超过 1 MB 时先把旧的挪成 `ui.old.log`
pub fn init_log() {
    let dir = crate::setup::ui_data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let p = dir.join("ui.log");
    if std::fs::metadata(&p).map(|m| m.len() > 1 << 20).unwrap_or(false) {
        let _ = std::fs::rename(&p, dir.join("ui.old.log"));
    }
    if let Ok(mut l) = LOG.lock() {
        *l = Some(p);
    }
}

fn write_log(level: &str, msg: &str) {
    let Ok(l) = LOG.lock() else { return };
    let Some(p) = l.as_ref() else { return };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        let _ = writeln!(f, "{} [{level}] [{}] {msg}", crate::ui::fmt_time(galaxy_proto::now_ms(), true), std::process::id());
    }
}

pub fn log_info(msg: &str) {
    write_log("info", msg);
}

pub fn log_error(msg: &str) {
    write_log("error", msg);
}
