//! 远程进行中不让会话因闲置而黑屏、锁屏（GNOME Wayland 一锁屏，系统就禁止远程桌面）。
//! 在画面代理里、用会话总线申请：GNOME 用 org.gnome.SessionManager.Inhibit（闲置），
//! 其他桌面用 org.freedesktop.ScreenSaver.Inhibit。抑制跟着总线连接走：连接断了（代理退出）系统自动撤销。

use zbus::blocking::{Connection, Proxy};

pub struct IdleInhibit {
    conn: Connection,
    kind: Kind,
}

enum Kind {
    Gnome(u32),
    ScreenSaver(u32),
}

const REASON: &str = "LemonGalaxy remote session";
/// GNOME SessionManager 的抑制标志：闲置（不黑屏、不锁屏、不休眠）
const GSM_INHIBIT_IDLE: u32 = 8;

impl IdleInhibit {
    pub fn start() -> Result<Self, String> {
        let conn = Connection::session().map_err(|e| e.to_string())?;
        if let Ok(p) = Proxy::new(&conn, "org.gnome.SessionManager", "/org/gnome/SessionManager", "org.gnome.SessionManager") {
            if let Ok(cookie) = p.call::<_, _, u32>("Inhibit", &("lemon-galaxy", 0u32, REASON, GSM_INHIBIT_IDLE)) {
                return Ok(Self { conn, kind: Kind::Gnome(cookie) });
            }
        }
        let p = Proxy::new(&conn, "org.freedesktop.ScreenSaver", "/org/freedesktop/ScreenSaver", "org.freedesktop.ScreenSaver").map_err(|e| e.to_string())?;
        let cookie: u32 = p.call("Inhibit", &("lemon-galaxy", REASON)).map_err(|e| format!("会话里没有可用的闲置抑制接口：{e}"))?;
        Ok(Self { conn, kind: Kind::ScreenSaver(cookie) })
    }
}

impl Drop for IdleInhibit {
    fn drop(&mut self) {
        let _ = match self.kind {
            Kind::Gnome(c) => Proxy::new(&self.conn, "org.gnome.SessionManager", "/org/gnome/SessionManager", "org.gnome.SessionManager").and_then(|p| p.call::<_, _, ()>("Uninhibit", &(c,))),
            Kind::ScreenSaver(c) => Proxy::new(&self.conn, "org.freedesktop.ScreenSaver", "/org/freedesktop/ScreenSaver", "org.freedesktop.ScreenSaver").and_then(|p| p.call::<_, _, ()>("UnInhibit", &(c,))),
        };
    }
}
