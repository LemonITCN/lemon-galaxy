//! 进程内共享状态：数据库、密钥、设置、在线设备、中转表、登录中的 OPAQUE 状态、限速。

use crate::db::Db;
use crate::keys::Keys;
use galaxy_proto::ws::WsMsg;
use galaxy_proto::{ConnId, ConnMode, DeviceId, DeviceKind, PathKind};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub server_name: String,
    pub public_host: String,
    pub relay_enabled: bool,
    pub relay_session_kbps: u64,
    pub relay_total_mbps: u64,
    pub login_lockout_threshold: u32,
    pub login_lockout_minutes: u32,
    pub session_idle_days: u32,
    pub audit_retention_days: u32,
    pub admin_ip_allowlist: Vec<String>,
    pub update_source: String,
    pub update_github_repo: String,
    pub update_check_hours: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            server_name: "LemonGalaxy".into(),
            public_host: String::new(),
            relay_enabled: true,
            relay_session_kbps: 0,
            relay_total_mbps: 0,
            login_lockout_threshold: 5,
            login_lockout_minutes: 15,
            session_idle_days: 30,
            audit_retention_days: 180,
            admin_ip_allowlist: vec![],
            update_source: "github".into(),
            update_github_repo: "LemonITCN/lemon-galaxy".into(),
            update_check_hours: 6,
        }
    }
}

pub struct OnlineDevice {
    pub serial: u64,
    pub kind: DeviceKind,
    pub tx: mpsc::UnboundedSender<WsMsg>,
    pub public_ip: IpAddr,
    pub accounts: Vec<u64>,
    pub assist_code: Option<String>,
    pub hostname: String,
    pub connected_at: i64,
}

#[derive(Default)]
pub struct Hub {
    pub devices: RwLock<HashMap<DeviceId, OnlineDevice>>,
    pub assist_codes: RwLock<HashMap<String, DeviceId>>,
    serial: AtomicU64,
}

impl Hub {
    pub fn next_serial(&self) -> u64 {
        self.serial.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn send(&self, id: &DeviceId, msg: WsMsg) -> bool {
        match self.devices.read().get(id) {
            Some(d) => d.tx.send(msg).is_ok(),
            None => false,
        }
    }

    pub fn is_online(&self, id: &DeviceId) -> bool {
        self.devices.read().contains_key(id)
    }

    /// 发给某账号下所有在线设备。
    pub fn notify_user(&self, user_id: u64, msg: WsMsg) {
        for d in self.devices.read().values() {
            if d.accounts.contains(&user_id) {
                let _ = d.tx.send(msg.clone());
            }
        }
    }

    pub fn online_ids(&self) -> Vec<DeviceId> {
        self.devices.read().keys().copied().collect()
    }

    pub fn set_accounts(&self, id: &DeviceId, accounts: Vec<u64>) {
        if let Some(d) = self.devices.write().get_mut(id) {
            d.accounts = accounts;
        }
    }
}

/// 中转一端的去向。
pub enum RelayEnd {
    None,
    Udp(SocketAddr),
    Tcp(mpsc::Sender<bytes::Bytes>),
}

pub struct RelayPair {
    pub conn_id: ConnId,
    pub tokens: [[u8; 16]; 2],
    pub ends: [Mutex<RelayEnd>; 2],
    pub last_activity: AtomicI64,
    pub bytes: AtomicU64,
    /// 限速令牌桶：本秒已转发字节、所在秒
    pub window: Mutex<(i64, u64)>,
}

#[derive(Default)]
pub struct Relay {
    pub by_token: RwLock<HashMap<[u8; 16], (Arc<RelayPair>, usize)>>,
    pub pairs: RwLock<HashMap<ConnId, Arc<RelayPair>>>,
    pub total_bytes: AtomicU64,
}

impl Relay {
    pub fn create(&self, conn_id: ConnId) -> Arc<RelayPair> {
        let tokens = [galaxy_crypto::random_bytes::<16>(), galaxy_crypto::random_bytes::<16>()];
        let pair = Arc::new(RelayPair {
            conn_id,
            tokens,
            ends: [Mutex::new(RelayEnd::None), Mutex::new(RelayEnd::None)],
            last_activity: AtomicI64::new(galaxy_proto::now_ms()),
            bytes: AtomicU64::new(0),
            window: Mutex::new((0, 0)),
        });
        let mut bt = self.by_token.write();
        bt.insert(tokens[0], (pair.clone(), 0));
        bt.insert(tokens[1], (pair.clone(), 1));
        self.pairs.write().insert(conn_id, pair.clone());
        pair
    }

    pub fn lookup(&self, token: &[u8; 16]) -> Option<(Arc<RelayPair>, usize)> {
        self.by_token.read().get(token).cloned()
    }

    pub fn remove(&self, conn_id: &ConnId) {
        if let Some(p) = self.pairs.write().remove(conn_id) {
            let mut bt = self.by_token.write();
            bt.remove(&p.tokens[0]);
            bt.remove(&p.tokens[1]);
        }
    }

    /// 清理长时间无流量的中转。
    pub fn sweep(&self, idle_ms: i64) -> Vec<ConnId> {
        let now = galaxy_proto::now_ms();
        let stale: Vec<ConnId> = self
            .pairs
            .read()
            .iter()
            .filter(|(_, p)| now - p.last_activity.load(Ordering::Relaxed) > idle_ms)
            .map(|(k, _)| *k)
            .collect();
        for c in &stale {
            self.remove(c);
        }
        stale
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ConnInfo {
    pub id: ConnId,
    pub kind: ConnMode,
    pub user_id: u64,
    pub username: String,
    pub from_device_id: DeviceId,
    pub from_device_name: String,
    pub to_device_id: DeviceId,
    pub to_device_name: String,
    pub path: PathKind,
    pub started_at: i64,
}

pub struct PendingLogin {
    pub user_id: Option<u64>,
    pub username: String,
    pub server_state: Vec<u8>,
    pub created: Instant,
}

#[derive(Default)]
pub struct RateLimiter {
    buckets: Mutex<HashMap<String, (Instant, u32)>>,
}

impl RateLimiter {
    /// 每个 key 在 window 秒内最多 limit 次。
    pub fn check(&self, key: &str, limit: u32, window_secs: u64) -> bool {
        let mut b = self.buckets.lock();
        let now = Instant::now();
        if b.len() > 50_000 {
            b.retain(|_, (t, _)| now.duration_since(*t).as_secs() < window_secs);
        }
        let e = b.entry(key.to_string()).or_insert((now, 0));
        if now.duration_since(e.0).as_secs() >= window_secs {
            *e = (now, 0);
        }
        e.1 += 1;
        e.1 <= limit
    }
}

pub struct AppState {
    pub db: Db,
    pub db_path: PathBuf,
    pub data_dir: PathBuf,
    pub keys: Keys,
    pub settings: RwLock<Settings>,
    pub hub: Hub,
    pub relay: Relay,
    pub conns: RwLock<HashMap<ConnId, ConnInfo>>,
    pub logins: Mutex<HashMap<String, PendingLogin>>,
    pub rate: RateLimiter,
    pub started_at: i64,
    pub port: u16,
    pub udp: Arc<tokio::net::UdpSocket>,
    pub http: reqwest::Client,
    pub setup_token: Mutex<Option<String>>,
}

pub type St = Arc<AppState>;

impl AppState {
    pub fn public_host(&self) -> String {
        let s = self.settings.read();
        if s.public_host.is_empty() {
            "127.0.0.1".to_string()
        } else {
            s.public_host.clone()
        }
    }

    /// 客户端填写的完整服务器地址。
    pub fn client_address(&self) -> String {
        let host = self.public_host();
        let host = if host.contains(':') && !host.starts_with('[') { format!("[{host}]") } else { host };
        format!("{}:{}#{}", host, self.port, self.keys.fingerprint)
    }

    pub async fn save_settings(&self) -> anyhow::Result<()> {
        let json = serde_json::to_string(&*self.settings.read())?;
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT INTO settings(key, value) VALUES('settings', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    [json],
                )
            })
            .await?;
        Ok(())
    }
}

pub fn today() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!("{:04}-{:02}-{:02}", now.year(), now.month() as u8, now.day())
}

/// 累加当日统计。
pub async fn bump_stat(st: &AppState, field: &'static str, n: i64) {
    let day = today();
    let sql = format!(
        "INSERT INTO daily_stats(day, {field}) VALUES(?1, ?2) ON CONFLICT(day) DO UPDATE SET {field} = {field} + excluded.{field}"
    );
    let _ = st.db.call(move |c| c.execute(&sql, galaxy_params![day, n])).await;
}

#[macro_export]
macro_rules! galaxy_params {
    ($($p:expr),* $(,)?) => {
        r2d2_sqlite::rusqlite::params![$($p),*]
    };
}
pub use galaxy_params;
