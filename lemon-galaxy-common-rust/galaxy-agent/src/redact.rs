//! 发给大模型前遮蔽敏感信息（docs/ai-agent.md 第 7 节）。

use regex::Regex;
use std::sync::OnceLock;

const MASK: &str = "[已遮蔽]";

fn patterns() -> &'static Vec<(Regex, &'static str)> {
    static P: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    P.get_or_init(|| {
        let v: Vec<(&str, &str)> = vec![
            // 私钥块
            (r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z0-9 ]*PRIVATE KEY-----", "-----PRIVATE KEY [已遮蔽]-----"),
            // 云厂商访问密钥
            (r"\b(AKIA|ASIA)[0-9A-Z]{16}\b", MASK),
            (r"\bLTAI[0-9A-Za-z]{12,24}\b", MASK),
            (r"\bAKID[0-9A-Za-z]{13,40}\b", MASK),
            // 常见令牌
            (r"\bgh[pousr]_[A-Za-z0-9]{30,}\b", MASK),
            (r"\bglpat-[A-Za-z0-9_\-]{20,}\b", MASK),
            (r"\bxox[abprs]-[A-Za-z0-9\-]{10,}\b", MASK),
            (r"\bsk-[A-Za-z0-9_\-]{20,}\b", MASK),
            (r"\beyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\b", MASK),
            // URL 中的账号密码
            (r"(?i)\b([a-z][a-z0-9+.\-]*://[^\s:/@]+):([^\s@/]+)@", "$1:[已遮蔽]@"),
            // key=value 形式
            (r#"(?i)\b([A-Z0-9_]*(PASSWORD|PASSWD|PWD|SECRET|TOKEN|API_?KEY|ACCESS_?KEY|PRIVATE_?KEY|CREDENTIAL)[A-Z0-9_]*)\s*([=:])\s*("[^"]*"|'[^']*'|[^\s,;]+)"#, "$1$3[已遮蔽]"),
            // Authorization 头
            (r"(?i)(authorization:\s*(bearer|basic|token)\s+)[A-Za-z0-9._~+/=\-]+", "$1[已遮蔽]"),
        ];
        v.into_iter().filter_map(|(p, r)| Regex::new(p).ok().map(|re| (re, r))).collect()
    })
}

/// 替换文本里的敏感信息。返回 (处理后的文本, 遮蔽次数)。
pub fn redact(text: &str) -> (String, usize) {
    let mut out = text.to_string();
    let mut n = 0;
    for (re, rep) in patterns() {
        let c = re.find_iter(&out).count();
        if c > 0 {
            n += c;
            out = re.replace_all(&out, *rep).into_owned();
        }
    }
    (out, n)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn masks() {
        let (o, n) = redact("DB_PASSWORD=hunter2\nurl=postgres://app:s3cret@db:5432/x\nkey AKIAABCDEFGHIJKLMNOP end");
        assert!(n >= 3, "{o}");
        assert!(!o.contains("hunter2") && !o.contains("s3cret") && !o.contains("AKIAABCDEFGHIJKLMNOP"), "{o}");
        let (o, _) = redact("-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----");
        assert!(!o.contains("abc"));
        assert_eq!(redact("nothing here").1, 0);
    }
}
