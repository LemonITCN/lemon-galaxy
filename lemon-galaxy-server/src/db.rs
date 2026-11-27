//! SQLite 单文件数据库（WAL）。所有访问经由阻塞线程池，避免卡住异步运行时。

use r2d2_sqlite::rusqlite::{self, Connection};
use r2d2_sqlite::SqliteConnectionManager;
use std::path::Path;

pub type Pool = r2d2::Pool<SqliteConnectionManager>;

/// 老数据库补列（新建的库在 SCHEMA 里已经有）。
fn migrate(c: &Connection) -> rusqlite::Result<()> {
    let has = |table: &str, col: &str| -> rusqlite::Result<bool> {
        let mut st = c.prepare(&format!("PRAGMA table_info({table})"))?;
        let names = st.query_map([], |r| r.get::<_, String>(1))?.collect::<Result<Vec<_>, _>>()?;
        Ok(names.iter().any(|n| n == col))
    };
    if !has("devices", "macs")? {
        c.execute_batch("ALTER TABLE devices ADD COLUMN macs TEXT NOT NULL DEFAULT ''")?;
    }
    if !has("devices", "update_channel")? {
        c.execute_batch("ALTER TABLE devices ADD COLUMN update_channel TEXT NOT NULL DEFAULT ''")?;
    }
    Ok(())
}

#[derive(Clone)]
pub struct Db {
    pool: Pool,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE TABLE IF NOT EXISTS admins (
  id INTEGER PRIMARY KEY,
  username TEXT NOT NULL UNIQUE COLLATE NOCASE,
  password_hash TEXT NOT NULL,
  totp_secret BLOB,
  totp_enabled INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  last_login_at INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS admin_sessions (
  token_hash BLOB PRIMARY KEY,
  admin_id INTEGER NOT NULL,
  csrf TEXT NOT NULL,
  stage TEXT NOT NULL,
  ip TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS users (
  id INTEGER PRIMARY KEY,
  username TEXT NOT NULL UNIQUE COLLATE NOCASE,
  display_name TEXT NOT NULL DEFAULT '',
  note TEXT NOT NULL DEFAULT '',
  disabled INTEGER NOT NULL DEFAULT 0,
  must_change_password INTEGER NOT NULL DEFAULT 1,
  opaque_record BLOB NOT NULL,
  totp_secret BLOB,
  totp_enabled INTEGER NOT NULL DEFAULT 0,
  require_totp INTEGER NOT NULL DEFAULT 0,
  account_epoch INTEGER NOT NULL DEFAULT 1,
  account_pubkey BLOB,
  wrapped_secrets BLOB,
  wrapped_nonce BLOB,
  revocations BLOB,
  revocations_sig BLOB,
  ai_enabled INTEGER NOT NULL DEFAULT 1,
  ai_max_mode TEXT NOT NULL DEFAULT 'confirm_dangerous',
  ai_daily_token_limit INTEGER NOT NULL DEFAULT 0,
  ai_monthly_token_limit INTEGER NOT NULL DEFAULT 0,
  failed_logins INTEGER NOT NULL DEFAULT 0,
  locked_until INTEGER NOT NULL DEFAULT 0,
  vault_version INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  last_login_at INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS devices (
  id BLOB PRIMARY KEY,
  pubkey BLOB NOT NULL,
  kind TEXT NOT NULL,
  platform TEXT NOT NULL,
  os_version TEXT NOT NULL DEFAULT '',
  arch TEXT NOT NULL DEFAULT '',
  hostname TEXT NOT NULL DEFAULT '',
  app_version TEXT NOT NULL DEFAULT '',
  last_seen_at INTEGER NOT NULL DEFAULT 0,
  last_ip TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  macs TEXT NOT NULL DEFAULT '',
  -- 更新通道「组件/平台/架构」，被控端每次连上信令时报
  update_channel TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS device_accounts (
  device_id BLOB NOT NULL,
  user_id INTEGER NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending',
  cert BLOB,
  cert_sig BLOB,
  added_at INTEGER NOT NULL,
  PRIMARY KEY (device_id, user_id)
);
CREATE INDEX IF NOT EXISTS device_accounts_user ON device_accounts(user_id);

CREATE TABLE IF NOT EXISTS sessions (
  token_hash BLOB PRIMARY KEY,
  user_id INTEGER NOT NULL,
  device_id BLOB NOT NULL,
  restricted INTEGER NOT NULL DEFAULT 0,
  ip TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  last_used_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS sessions_user ON sessions(user_id);

CREATE TABLE IF NOT EXISTS vault_items (
  user_id INTEGER NOT NULL,
  id TEXT NOT NULL,
  kind TEXT NOT NULL,
  ciphertext BLOB NOT NULL,
  nonce BLOB NOT NULL,
  version INTEGER NOT NULL,
  deleted INTEGER NOT NULL DEFAULT 0,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (user_id, id)
);
CREATE INDEX IF NOT EXISTS vault_items_ver ON vault_items(user_id, version);

CREATE TABLE IF NOT EXISTS ai_providers (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  kind TEXT NOT NULL,
  base_url TEXT NOT NULL,
  api_key_enc BLOB,
  enabled INTEGER NOT NULL DEFAULT 1,
  models_json TEXT NOT NULL DEFAULT '[]',
  models_updated_at INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS ai_grants (
  user_id INTEGER NOT NULL,
  provider_id INTEGER NOT NULL,
  models_json TEXT NOT NULL,
  PRIMARY KEY (user_id, provider_id)
);

CREATE TABLE IF NOT EXISTS ai_usage (
  id INTEGER PRIMARY KEY,
  at INTEGER NOT NULL,
  user_id INTEGER NOT NULL,
  device_id TEXT NOT NULL DEFAULT '',
  provider_id INTEGER NOT NULL,
  model TEXT NOT NULL,
  prompt_tokens INTEGER NOT NULL,
  completion_tokens INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ai_usage_at ON ai_usage(at);
CREATE INDEX IF NOT EXISTS ai_usage_user ON ai_usage(user_id, at);

CREATE TABLE IF NOT EXISTS audit (
  id INTEGER PRIMARY KEY,
  at INTEGER NOT NULL,
  actor_type TEXT NOT NULL,
  actor_id TEXT NOT NULL DEFAULT '',
  actor_name TEXT NOT NULL DEFAULT '',
  action TEXT NOT NULL,
  target_type TEXT NOT NULL DEFAULT '',
  target_id TEXT NOT NULL DEFAULT '',
  target_name TEXT NOT NULL DEFAULT '',
  ip TEXT NOT NULL DEFAULT '',
  detail TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS audit_at ON audit(at);

CREATE TABLE IF NOT EXISTS daily_stats (
  day TEXT PRIMARY KEY,
  connections INTEGER NOT NULL DEFAULT 0,
  relay_bytes INTEGER NOT NULL DEFAULT 0,
  ai_tokens INTEGER NOT NULL DEFAULT 0,
  logins INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE TABLE IF NOT EXISTS update_packages (
  id INTEGER PRIMARY KEY,
  component TEXT NOT NULL,
  platform TEXT NOT NULL,
  arch TEXT NOT NULL,
  version TEXT NOT NULL,
  file_name TEXT NOT NULL,
  size INTEGER NOT NULL,
  sha256 TEXT NOT NULL,
  signature TEXT NOT NULL DEFAULT '',
  signature_ok INTEGER NOT NULL DEFAULT 0,
  source TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
"#;

impl Db {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        // 先用单个连接切到 WAL 并建表，避免连接池并发初始化时互相锁住
        {
            let c = Connection::open(path)?;
            c.execute_batch("PRAGMA busy_timeout = 5000; PRAGMA journal_mode = WAL;")?;
            c.execute_batch(SCHEMA)?;
            migrate(&c)?;
        }
        let manager = SqliteConnectionManager::file(path).with_init(|c| {
            c.execute_batch(
                "PRAGMA busy_timeout = 5000;
                 PRAGMA synchronous = NORMAL;
                 PRAGMA foreign_keys = ON;
                 PRAGMA temp_store = MEMORY;",
            )
        });
        let pool = r2d2::Pool::builder().max_size(8).build(manager)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(Self { pool })
    }

    /// 在阻塞线程池里执行数据库操作。
    pub async fn call<F, T>(&self, f: F) -> anyhow::Result<T>
    where
        F: FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<T> {
            let mut c = pool.get()?;
            Ok(f(&mut c)?)
        })
        .await?
    }

    /// 同步版本（启动时、命令行用）。
    pub fn call_sync<F, T>(&self, f: F) -> anyhow::Result<T>
    where
        F: FnOnce(&mut Connection) -> rusqlite::Result<T>,
    {
        let mut c = self.pool.get()?;
        Ok(f(&mut c)?)
    }

    pub fn file_size(path: &Path) -> u64 {
        let mut total = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        for ext in ["-wal", "-shm"] {
            let p = format!("{}{}", path.display(), ext);
            total += std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        }
        total
    }
}

