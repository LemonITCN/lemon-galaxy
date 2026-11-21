use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

macro_rules! id16 {
    ($name:ident) => {
        /// 16 字节标识，JSON 里是 32 位小写十六进制，postcard 里是原始字节。
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
        pub struct $name(pub [u8; 16]);

        impl $name {
            pub fn random() -> Self {
                Self(rand::random())
            }
            pub fn to_hex(&self) -> String {
                hex::encode(self.0)
            }
            pub fn from_hex(s: &str) -> Option<Self> {
                let v = hex::decode(s.trim()).ok()?;
                let a: [u8; 16] = v.try_into().ok()?;
                Some(Self(a))
            }
            pub fn from_slice(b: &[u8]) -> Option<Self> {
                let a: [u8; 16] = b.try_into().ok()?;
                Some(Self(a))
            }
            pub fn is_zero(&self) -> bool {
                self.0 == [0u8; 16]
            }
            /// 界面上显示用的短形式。
            pub fn short(&self) -> String {
                self.to_hex()[..8].to_string()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.to_hex())
            }
        }
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.short())
            }
        }
        impl FromStr for $name {
            type Err = &'static str;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::from_hex(s).ok_or("invalid id")
            }
        }
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                if s.is_human_readable() {
                    s.serialize_str(&self.to_hex())
                } else {
                    self.0.serialize(s)
                }
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                if d.is_human_readable() {
                    let s = String::deserialize(d)?;
                    Self::from_hex(&s).ok_or_else(|| serde::de::Error::custom("invalid id"))
                } else {
                    Ok(Self(<[u8; 16]>::deserialize(d)?))
                }
            }
        }
    };
}

id16!(DeviceId);
id16!(ConnId);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        let id = DeviceId::random();
        let j = serde_json::to_string(&id).unwrap();
        assert_eq!(j.len(), 34);
        let back: DeviceId = serde_json::from_str(&j).unwrap();
        assert_eq!(back, id);
        let p = postcard::to_stdvec(&id).unwrap();
        assert_eq!(p.len(), 16);
        let back: DeviceId = postcard::from_bytes(&p).unwrap();
        assert_eq!(back, id);
    }
}
