//! JSON 里的二进制字段统一用 base64url（无填充）。

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Deserializer, Serializer};

pub fn encode(data: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(data)
}

pub fn decode(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    // 兼容带填充与标准字母表的输入
    let t = s.trim().trim_end_matches('=');
    URL_SAFE_NO_PAD
        .decode(t)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(t))
}

pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
    if s.is_human_readable() {
        s.serialize_str(&encode(v))
    } else {
        s.serialize_bytes(v)
    }
}

pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    if d.is_human_readable() {
        let s = String::deserialize(d)?;
        decode(&s).map_err(serde::de::Error::custom)
    } else {
        let b: serde_bytes_compat::ByteBuf = Deserialize::deserialize(d)?;
        Ok(b.0)
    }
}

/// `Option<Vec<u8>>` 版本。
pub mod opt {
    use super::*;
    pub fn serialize<S: Serializer>(v: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(b) => {
                if s.is_human_readable() {
                    s.serialize_some(&encode(b))
                } else {
                    s.serialize_some(&serde_bytes_compat::Bytes(b))
                }
            }
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        if d.is_human_readable() {
            let s: Option<String> = Option::deserialize(d)?;
            match s {
                Some(s) => decode(&s).map(Some).map_err(serde::de::Error::custom),
                None => Ok(None),
            }
        } else {
            let b: Option<serde_bytes_compat::ByteBuf> = Option::deserialize(d)?;
            Ok(b.map(|b| b.0))
        }
    }
}

/// 不引入 serde_bytes 依赖的最小字节串包装。
pub(crate) mod serde_bytes_compat {
    use serde::de::{SeqAccess, Visitor};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::fmt;

    pub struct Bytes<'a>(pub &'a [u8]);
    impl Serialize for Bytes<'_> {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_bytes(self.0)
        }
    }

    pub struct ByteBuf(pub Vec<u8>);
    impl<'de> Deserialize<'de> for ByteBuf {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct V;
            impl<'de> Visitor<'de> for V {
                type Value = ByteBuf;
                fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                    f.write_str("bytes")
                }
                fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<ByteBuf, E> {
                    Ok(ByteBuf(v.to_vec()))
                }
                fn visit_byte_buf<E: serde::de::Error>(self, v: Vec<u8>) -> Result<ByteBuf, E> {
                    Ok(ByteBuf(v))
                }
                fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<ByteBuf, A::Error> {
                    let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(4096));
                    while let Some(b) = seq.next_element::<u8>()? {
                        out.push(b);
                    }
                    Ok(ByteBuf(out))
                }
            }
            d.deserialize_bytes(V)
        }
    }
}
