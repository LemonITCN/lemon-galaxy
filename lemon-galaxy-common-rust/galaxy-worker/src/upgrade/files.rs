//! 换文件、留备份、按备份换回。与系统无关的部分，各系统都能跑单元测试。
//!
//! 换一个文件：先把新内容复制到目标旁边（同一个目录，改名是原子的），再改名换上。Windows 上运行中的程序
//! 不能覆盖、不能删，但能改名：改名失败时把旧的挪成 `*.old.exe`（或 `*.old-N.exe`）再换上，之后清理。

use std::io;
use std::path::{Path, PathBuf};

fn with_suffix(p: &Path, suffix: &str) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// 挪开旧文件用的名字：`a.exe` → `a.old.exe`、`a.old-1.exe`…；`a.json` → `a.json.old`、`a.json.old-1`…
fn old_name(target: &Path, n: u32) -> PathBuf {
    let name = target.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let tag = if n == 0 { "old".to_string() } else { format!("old-{n}") };
    let new = match name.len().checked_sub(4).filter(|&i| name[i..].eq_ignore_ascii_case(".exe")) {
        Some(i) => format!("{}.{tag}{}", &name[..i], &name[i..]),
        None => format!("{name}.{tag}"),
    };
    target.with_file_name(new)
}

/// 是不是挪开的旧文件（清理时删）
pub fn is_old_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains(".old.") || n.ends_with(".old") || n.contains(".old-")
}

fn set_mode(p: &Path, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode))
    }
    #[cfg(not(unix))]
    {
        let _ = (p, mode);
        Ok(())
    }
}

/// 把 `src` 的内容换到 `target`（`mode` 是 Unix 权限，如 0o755）。
pub fn install_file(src: &Path, target: &Path, mode: u32) -> io::Result<()> {
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = with_suffix(target, ".lgnew");
    let _ = std::fs::remove_file(&tmp);
    if let Err(e) = std::fs::copy(src, &tmp).and_then(|_| set_mode(&tmp, mode)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    let first = match std::fs::rename(&tmp, target) {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };
    if !target.exists() {
        let _ = std::fs::remove_file(&tmp);
        return Err(first);
    }
    // 被占用（Windows 上正在运行的程序）：把旧的改名挪开，再换上
    let old = (0..50)
        .map(|n| old_name(target, n))
        .find(|o| !o.exists() || std::fs::remove_file(o).is_ok())
        .ok_or_else(|| io::Error::other(format!("{}：挪不开旧文件", target.display())))?;
    if let Err(e) = std::fs::rename(target, &old) {
        let _ = std::fs::remove_file(&tmp);
        return Err(io::Error::new(e.kind(), format!("{}：{first}；改名也失败：{e}", target.display())));
    }
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::rename(&old, target);
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// 删掉一个文件；正在运行删不掉时改名挪开（回滚时删新版本新加的文件用）。
pub fn remove_file(target: &Path) -> io::Result<()> {
    match std::fs::remove_file(target) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => {
            let old = (0..50).map(|n| old_name(target, n)).find(|o| !o.exists() || std::fs::remove_file(o).is_ok()).ok_or(e)?;
            std::fs::rename(target, old)
        }
    }
}

/// 目录里的文件（相对路径，按名字排序，不含目录本身）
pub fn list_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    fn walk(root: &Path, rel: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(root.join(rel))?.collect::<Result<_, _>>()?;
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let r = rel.join(e.file_name());
            if e.file_type()?.is_dir() {
                walk(root, &r, out)?;
            } else {
                out.push(r);
            }
        }
        Ok(())
    }
    let mut out = vec![];
    walk(root, Path::new(""), &mut out)?;
    Ok(out)
}

/// 删掉目录里挪开的旧文件（`*.old*`），删不掉的（还在运行）留着下次再删。
pub fn clean_old_files(dir: &Path) {
    let Ok(files) = list_files(dir) else { return };
    for rel in files {
        let name = rel.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        if is_old_name(&name) || name.ends_with(".lgnew") {
            let _ = std::fs::remove_file(dir.join(&rel));
        }
    }
}

/// 删掉换 `target` 时挪开的旧文件（`target` 同目录下的 `*.old*`，只认由它的名字派生出来的），删不掉的留着下次再删。
pub fn clean_old_of(target: &Path) {
    for n in 0..50 {
        let o = old_name(target, n);
        if o.exists() {
            let _ = std::fs::remove_file(&o);
        }
    }
    let _ = std::fs::remove_file(with_suffix(target, ".lgnew"));
}

/// 尽量删掉整个目录（Windows 上还在运行的程序删不掉，留着）
pub fn remove_dir_best_effort(dir: &Path) {
    if std::fs::remove_dir_all(dir).is_err() {
        if let Ok(files) = list_files(dir) {
            for f in files {
                let _ = std::fs::remove_file(dir.join(f));
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lg-upd-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn old_names() {
        let p = Path::new("/x/LemonGalaxyWorker.exe");
        assert_eq!(old_name(p, 0), Path::new("/x/LemonGalaxyWorker.old.exe"));
        assert_eq!(old_name(p, 2), Path::new("/x/LemonGalaxyWorker.old-2.exe"));
        assert_eq!(old_name(Path::new("/x/a.EXE"), 0), Path::new("/x/a.old.EXE"));
        assert_eq!(old_name(Path::new("/x/LemonGalaxyWorker.exe.config"), 0), Path::new("/x/LemonGalaxyWorker.exe.config.old"));
        assert_eq!(old_name(Path::new("/x/lemon-galaxy-worker"), 1), Path::new("/x/lemon-galaxy-worker.old-1"));
        for n in ["a.old.exe", "a.old-3.exe", "a.json.old", "a.json.old-1", "LemonGalaxyWorker.OLD.exe"] {
            assert!(is_old_name(n), "{n}");
        }
        for n in ["a.exe", "older.txt", "bold.json", "lemon-galaxy-worker"] {
            assert!(!is_old_name(n), "{n}");
        }
    }

    #[test]
    fn install_and_list() {
        let d = tmp("install");
        let src = d.join("src.bin");
        std::fs::write(&src, b"new").unwrap();
        // 目标目录不存在时建好
        let t = d.join("inst/sub/x.bin");
        install_file(&src, &t, 0o755).unwrap();
        assert_eq!(std::fs::read(&t).unwrap(), b"new");
        assert!(!with_suffix(&t, ".lgnew").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&t).unwrap().permissions().mode() & 0o777, 0o755);
        }
        // 覆盖已有的
        std::fs::write(&src, b"newer").unwrap();
        install_file(&src, &t, 0o644).unwrap();
        assert_eq!(std::fs::read(&t).unwrap(), b"newer");
        std::fs::write(d.join("inst/a.txt"), b"a").unwrap();
        assert_eq!(list_files(&d.join("inst")).unwrap(), vec![PathBuf::from("a.txt"), PathBuf::from("sub").join("x.bin")]);
        // 源文件不存在：目标不动
        assert!(install_file(&d.join("nope"), &t, 0o644).is_err());
        assert_eq!(std::fs::read(&t).unwrap(), b"newer");
        // 清理挪开的旧文件
        std::fs::write(d.join("inst/x.old.exe"), b"o").unwrap();
        std::fs::write(d.join("inst/sub/y.json.old-2"), b"o").unwrap();
        clean_old_files(&d.join("inst"));
        assert_eq!(list_files(&d.join("inst")).unwrap(), vec![PathBuf::from("a.txt"), PathBuf::from("sub").join("x.bin")]);
        std::fs::write(d.join("inst/sub/x.bin.old-3"), b"o").unwrap();
        std::fs::write(d.join("inst/sub/other.old"), b"o").unwrap();
        clean_old_of(&d.join("inst/sub/x.bin"));
        assert!(!d.join("inst/sub/x.bin.old-3").exists());
        // 别的文件挪开的旧文件不动（clean_old_of 只认自己的）
        assert!(d.join("inst/sub/other.old").exists());
        remove_file(&d.join("inst/a.txt")).unwrap();
        std::fs::write(d.join("inst/sub/x.bin.old-3"), b"o").unwrap();
        std::fs::write(d.join("inst/sub/other.old"), b"o").unwrap();
        clean_old_of(&d.join("inst/sub/x.bin"));
        assert!(!d.join("inst/sub/x.bin.old-3").exists());
        // 别的文件挪开的旧文件不动（clean_old_of 只认自己的）
        assert!(d.join("inst/sub/other.old").exists());
        remove_file(&d.join("inst/a.txt")).unwrap();
        assert!(!d.join("inst/a.txt").exists());
        remove_dir_best_effort(&d);
        assert!(!d.exists());
    }

    /// Windows：运行中的程序不能覆盖，换的时候把它挪成 .old.exe（系统允许直接替换时就直接换）
    #[cfg(windows)]
    #[test]
    fn replace_running_exe() {
        let d = tmp("running");
        let exe = d.join("probe.exe");
        std::fs::copy(std::env::var_os("COMSPEC").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows\System32\cmd.exe")), &exe).unwrap();
        let mut child = std::process::Command::new(&exe).args(["/c", "ping -n 5 127.0.0.1 >nul"]).spawn().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let src = d.join("new.exe");
        std::fs::write(&src, b"MZ new").unwrap();
        install_file(&src, &exe, 0o755).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"MZ new");
        let _ = child.kill();
        let _ = child.wait();
        clean_old_files(&d);
        assert!(!d.join("probe.old.exe").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
