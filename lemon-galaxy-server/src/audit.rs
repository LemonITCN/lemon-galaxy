//! 审计日志。

use crate::state::AppState;
use r2d2_sqlite::rusqlite::params;
use serde_json::Value;

pub struct Actor {
    pub kind: &'static str,
    pub id: String,
    pub name: String,
    pub ip: String,
}

impl Actor {
    pub fn system() -> Self {
        Self { kind: "system", id: String::new(), name: "system".into(), ip: String::new() }
    }
}

pub struct Target {
    pub kind: &'static str,
    pub id: String,
    pub name: String,
}

impl Target {
    pub fn none() -> Self {
        Self { kind: "", id: String::new(), name: String::new() }
    }
    pub fn new(kind: &'static str, id: impl ToString, name: impl Into<String>) -> Self {
        Self { kind, id: id.to_string(), name: name.into() }
    }
}

pub async fn log(st: &AppState, actor: &Actor, action: &'static str, target: Target, detail: Value) {
    let now = galaxy_proto::now_ms();
    let (ak, aid, aname, ip) = (actor.kind.to_string(), actor.id.clone(), actor.name.clone(), actor.ip.clone());
    let detail = detail.to_string();
    let r = st
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO audit(at, actor_type, actor_id, actor_name, action, target_type, target_id, target_name, ip, detail)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![now, ak, aid, aname, action, target.kind, target.id, target.name, ip, detail],
            )
        })
        .await;
    if let Err(e) = r {
        tracing::warn!("写审计日志失败: {e}");
    }
}
