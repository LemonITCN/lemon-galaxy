//! 设备身份（设备 ID + Ed25519 设备密钥）与本地安全存储抽象。

use galaxy_crypto::SignKey;
use galaxy_proto::DeviceId;
use std::path::PathBuf;
use std::sync::Arc;

/// 本地安全存储：Apple Keychain / Android Keystore / Windows DPAPI / Linux 0600 文件。
pub trait SecureStore: Send + Sync {
    fn get(&self, key: &str) -> Option<Vec<u8>>;
    fn set(&self, key: &str, value: &[u8]);
    fn remove(&self, key: &str);
}

/// 文件实现（Linux worker 与开发工具用）：每个键一个文件，目录 0700、文件 0600。
pub struct FileStore {
    dir: PathBuf,
}

impl FileStore {
    pub fn new(dir: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
        Ok(Self { dir })
    }
    fn path(&self, key: &str) -> PathBuf {
        let safe: String = key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect();
        self.dir.join(safe)
    }
}

impl SecureStore for FileStore {
    fn get(&self, key: &str) -> Option<Vec<u8>> {
        std::fs::read(self.path(key)).ok()
    }
    fn set(&self, key: &str, value: &[u8]) {
        let p = self.path(key);
        let tmp = p.with_extension("tmp");
        if std::fs::write(&tmp, value).is_ok() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
            }
            let _ = std::fs::rename(&tmp, &p);
        }
    }
    fn remove(&self, key: &str) {
        let _ = std::fs::remove_file(self.path(key));
    }
}

#[derive(Clone)]
pub struct Identity {
    pub device_id: DeviceId,
    pub key: SignKey,
}

impl Identity {
    /// 读取或首次生成设备身份。
    pub fn load_or_create(store: &Arc<dyn SecureStore>) -> Self {
        if let (Some(id), Some(k)) = (store.get("device_id"), store.get("device_key")) {
            if let (Some(device_id), Ok(key)) = (DeviceId::from_slice(&id), SignKey::from_bytes(&k)) {
                return Self { device_id, key };
            }
        }
        let me = Self { device_id: DeviceId::random(), key: SignKey::generate() };
        store.set("device_id", &me.device_id.0);
        store.set("device_key", &me.key.to_bytes());
        me
    }

    pub fn local_wrap_key(&self) -> [u8; 32] {
        galaxy_crypto::local_wrap_key(&self.key)
    }
}
