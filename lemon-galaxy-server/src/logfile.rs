//! 日志文件：系统服务没有控制台，日志写到数据目录里。超过上限就轮换（server.log → server.log.1 …），只留几份。

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// 单个文件的上限。
const MAX_BYTES: u64 = 10 * 1024 * 1024;
/// 轮换后保留的旧文件个数。
const KEEP: usize = 3;

pub struct RotatingFile {
    path: PathBuf,
    file: Option<File>,
    size: u64,
    max: u64,
    keep: usize,
}

impl RotatingFile {
    pub fn open(path: &Path) -> io::Result<Self> {
        Self::with_limits(path, MAX_BYTES, KEEP)
    }

    fn with_limits(path: &Path, max: u64, keep: usize) -> io::Result<Self> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let size = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(Self { path: path.to_path_buf(), file: Some(file), size, max, keep })
    }

    fn rotated(&self, n: usize) -> PathBuf {
        let mut s = self.path.clone().into_os_string();
        s.push(format!(".{n}"));
        PathBuf::from(s)
    }

    fn rotate(&mut self) -> io::Result<()> {
        // Windows 上打开着的文件不能改名，先关掉
        self.file = None;
        let _ = std::fs::remove_file(self.rotated(self.keep));
        for n in (1..self.keep).rev() {
            let _ = std::fs::rename(self.rotated(n), self.rotated(n + 1));
        }
        if self.keep > 0 {
            let _ = std::fs::rename(&self.path, self.rotated(1));
        }
        self.file = Some(OpenOptions::new().create(true).write(true).truncate(true).open(&self.path)?);
        self.size = 0;
        Ok(())
    }
}

impl Write for RotatingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.size > 0 && self.size + buf.len() as u64 > self.max {
            self.rotate()?;
        }
        let f = match self.file.as_mut() {
            Some(f) => f,
            None => return Ok(buf.len()),
        };
        let n = f.write(buf)?;
        self.size += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.file.as_mut() {
            Some(f) => f.flush(),
            None => Ok(()),
        }
    }
}

/// 读日志文件最后几行（安装服务失败时给用户看）。
pub fn tail(path: &Path, lines: usize) -> String {
    let Ok(text) = std::fs::read(path) else { return String::new() };
    let text = String::from_utf8_lossy(&text);
    let v: Vec<&str> = text.lines().collect();
    v[v.len().saturating_sub(lines)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_and_keeps_limited_files() {
        let dir = std::env::temp_dir().join(format!("lg-logfile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("logs").join("server.log");
        let mut f = RotatingFile::with_limits(&path, 100, 2).unwrap();
        for i in 0..20 {
            f.write_all(format!("line {i:02} ..............................\n").as_bytes()).unwrap();
        }
        f.flush().unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() <= 100);
        assert!(path.with_file_name("server.log.1").exists());
        assert!(path.with_file_name("server.log.2").exists());
        assert!(!path.with_file_name("server.log.3").exists());
        // 最新一行在当前文件里
        assert!(tail(&path, 1).contains("line 19"));
        // 重新打开接着写，不清空
        drop(f);
        let before = std::fs::metadata(&path).unwrap().len();
        let mut f = RotatingFile::with_limits(&path, 1000, 2).unwrap();
        f.write_all(b"again\n").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), before + 6);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
