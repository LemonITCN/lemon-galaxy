//! 服务器地址：`host:port#sha256-<base64url 指纹>`，指纹可省略（使用正规 CA 证书时）。

use std::net::{SocketAddr, ToSocketAddrs};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerAddr {
    pub host: String,
    pub port: u16,
    pub fingerprint: Option<[u8; 32]>,
}

impl ServerAddr {
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        let s = s.strip_prefix("https://").unwrap_or(s);
        let s = s.strip_prefix("galaxy://").unwrap_or(s);
        let (hp, fp) = match s.split_once('#') {
            Some((a, b)) => (a, Some(b)),
            None => (s, None),
        };
        let hp = hp.trim_end_matches('/');
        let (host, port) = if let Some(rest) = hp.strip_prefix('[') {
            let (h, p) = rest.split_once(']').ok_or("地址格式错误")?;
            let port = p.strip_prefix(':').map(|x| x.parse::<u16>()).transpose().map_err(|_| "端口错误")?;
            (h.to_string(), port.unwrap_or(galaxy_proto::DEFAULT_PORT))
        } else if hp.matches(':').count() > 1 {
            // 裸 IPv6，不带端口
            (hp.to_string(), galaxy_proto::DEFAULT_PORT)
        } else {
            match hp.rsplit_once(':') {
                Some((h, p)) => (h.to_string(), p.parse::<u16>().map_err(|_| "端口错误")?),
                None => (hp.to_string(), galaxy_proto::DEFAULT_PORT),
            }
        };
        if host.is_empty() {
            return Err("缺少服务器主机名".into());
        }
        let fingerprint = match fp {
            Some(f) => {
                let f = f.trim().strip_prefix("sha256-").ok_or("指纹格式错误")?;
                let b = galaxy_proto::b64::decode(f).map_err(|_| "指纹格式错误")?;
                Some(<[u8; 32]>::try_from(b.as_slice()).map_err(|_| "指纹长度错误")?)
            }
            None => None,
        };
        Ok(Self { host, port, fingerprint })
    }

    pub fn host_for_url(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        }
    }

    pub fn base_url(&self) -> String {
        format!("https://{}:{}", self.host_for_url(), self.port)
    }

    pub fn ws_url(&self, path: &str) -> String {
        format!("wss://{}:{}{}", self.host_for_url(), self.port, path)
    }

    /// 规范字符串（存盘、显示）。
    pub fn to_string_full(&self) -> String {
        match &self.fingerprint {
            Some(f) => format!("{}:{}#sha256-{}", self.host_for_url(), self.port, galaxy_proto::b64::encode(f)),
            None => format!("{}:{}", self.host_for_url(), self.port),
        }
    }

    /// 解析服务器 UDP 地址（可能有 IPv4 与 IPv6 两个）。
    pub fn resolve(&self) -> Vec<SocketAddr> {
        (self.host.as_str(), self.port).to_socket_addrs().map(|i| i.collect()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse() {
        let fp = galaxy_proto::b64::encode(&[7u8; 32]);
        let a = ServerAddr::parse(&format!("galaxy.example.com:7443#sha256-{fp}")).unwrap();
        assert_eq!(a.host, "galaxy.example.com");
        assert_eq!(a.port, 7443);
        assert_eq!(a.fingerprint, Some([7u8; 32]));
        let b = ServerAddr::parse("[240e::1]:9000").unwrap();
        assert_eq!(b.host, "240e::1");
        assert_eq!(b.port, 9000);
        assert_eq!(b.base_url(), "https://[240e::1]:9000");
        let c = ServerAddr::parse("1.2.3.4").unwrap();
        assert_eq!(c.port, 7443);
        assert_eq!(ServerAddr::parse(&a.to_string_full()).unwrap(), a);
    }
}
