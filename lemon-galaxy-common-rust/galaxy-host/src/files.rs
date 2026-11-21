//! 文件服务。同一套代码既在进程内跑（被控端以目标用户身份运行时），也在降权的子进程里跑（root 服务替普通用户操作时）。

use crate::fio;
use galaxy_proto::peer::{FileEntry, FileKind, FileMsg, FileOp, Origin, PeerError};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWrite, AsyncWriteExt};

const CHUNK: usize = 256 * 1024;

pub struct FileCtx {
    pub home: PathBuf,
    /// AI 改动前的备份目录（为空不备份）
    pub backup_dir: Option<PathBuf>,
}

fn err(code: &str, e: impl std::fmt::Display) -> FileMsg {
    FileMsg::Error(PeerError::new(code, e.to_string()))
}

fn io_err(e: std::io::Error) -> FileMsg {
    let code = match e.kind() {
        std::io::ErrorKind::NotFound => "not_found",
        std::io::ErrorKind::PermissionDenied => "permission_denied",
        std::io::ErrorKind::AlreadyExists => "exists",
        _ => "io",
    };
    let msg = match e.kind() {
        std::io::ErrorKind::NotFound => "文件或目录不存在".to_string(),
        std::io::ErrorKind::PermissionDenied => "没有权限".to_string(),
        std::io::ErrorKind::AlreadyExists => "已存在".to_string(),
        _ => e.to_string(),
    };
    err(code, msg)
}

pub fn resolve(ctx: &FileCtx, p: &str) -> PathBuf {
    let p = p.trim();
    if p.is_empty() || p == "~" {
        return ctx.home.clone();
    }
    if let Some(rest) = p.strip_prefix("~/") {
        return ctx.home.join(rest);
    }
    let pb = PathBuf::from(p);
    if pb.is_absolute() {
        pb
    } else {
        ctx.home.join(pb)
    }
}

#[cfg(unix)]
fn owner_names(uid: u32, gid: u32) -> (String, String) {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static CACHE: std::sync::OnceLock<Mutex<(HashMap<u32, String>, HashMap<u32, String>)>> = std::sync::OnceLock::new();
    let c = CACHE.get_or_init(|| Mutex::new((HashMap::new(), HashMap::new())));
    let mut c = c.lock().unwrap();
    let u = c.0.entry(uid).or_insert_with(|| nix::unistd::User::from_uid(uid.into()).ok().flatten().map(|u| u.name).unwrap_or_else(|| uid.to_string())).clone();
    let g = c.1.entry(gid).or_insert_with(|| nix::unistd::Group::from_gid(gid.into()).ok().flatten().map(|g| g.name).unwrap_or_else(|| gid.to_string())).clone();
    (u, g)
}

pub fn entry_of(path: &Path, name: Option<String>) -> std::io::Result<FileEntry> {
    let lm = std::fs::symlink_metadata(path)?;
    let ft = lm.file_type();
    let (kind, link_target, md) = if ft.is_symlink() {
        let t = std::fs::read_link(path).ok().map(|p| p.to_string_lossy().to_string());
        let target_md = std::fs::metadata(path).ok();
        let k = match &target_md {
            Some(m) if m.is_dir() => FileKind::Dir,
            _ => FileKind::Symlink,
        };
        (k, t, target_md.unwrap_or(lm.clone()))
    } else if ft.is_dir() {
        (FileKind::Dir, None, lm.clone())
    } else if ft.is_file() {
        (FileKind::File, None, lm.clone())
    } else {
        (FileKind::Other, None, lm.clone())
    };
    let modified_at = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0);
    #[cfg(unix)]
    let (mode, owner, group) = {
        use std::os::unix::fs::MetadataExt;
        let (o, g) = owner_names(lm.uid(), lm.gid());
        (lm.mode() & 0o7777, o, g)
    };
    #[cfg(not(unix))]
    let (mode, owner, group) = (if md.permissions().readonly() { 0o444 } else { 0o644 }, String::new(), String::new());
    Ok(FileEntry {
        name: name.unwrap_or_else(|| path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string_lossy().to_string())),
        path: path.to_string_lossy().to_string(),
        kind,
        size: if kind == FileKind::Dir { 0 } else { md.len() },
        mode,
        modified_at,
        owner,
        group,
        link_target,
    })
}

/// AI 改动前备份原文件。返回备份路径。
pub fn backup(ctx: &FileCtx, path: &Path, origin: &Origin) -> Option<String> {
    let Origin::Ai { conversation, step } = origin else { return None };
    let dir = ctx.backup_dir.as_ref()?;
    let md = std::fs::symlink_metadata(path).ok()?;
    let safe_conv: String = conversation.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(40).collect();
    let dest_dir = dir.join(safe_conv).join(format!("{}-{step}", galaxy_proto::now_ms()));
    std::fs::create_dir_all(&dest_dir).ok()?;
    let name = path.file_name()?.to_string_lossy().to_string();
    let dest = dest_dir.join(&name);
    if md.is_dir() {
        copy_dir(path, &dest, 0).ok()?;
    } else {
        std::fs::copy(path, &dest).ok()?;
    }
    let _ = std::fs::write(dest_dir.join(".origin"), path.to_string_lossy().as_bytes());
    Some(dest.to_string_lossy().to_string())
}

fn copy_dir(from: &Path, to: &Path, depth: u32) -> std::io::Result<()> {
    if depth > 32 {
        return Ok(());
    }
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let ft = e.file_type()?;
        let dst = to.join(e.file_name());
        if ft.is_dir() {
            copy_dir(&e.path(), &dst, depth + 1)?;
        } else if ft.is_file() {
            std::fs::copy(e.path(), dst)?;
        }
    }
    Ok(())
}

/// 处理一次文件操作。`r` / `w` 是这次操作的流（Write 时从 r 读数据块）。
pub async fn serve<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(op: FileOp, r: &mut R, w: &mut W, ctx: &FileCtx) -> anyhow::Result<()> {
    let reply = match op {
        FileOp::Home => FileMsg::Path(ctx.home.to_string_lossy().to_string()),
        FileOp::Realpath { path } => match std::fs::canonicalize(resolve(ctx, &path)) {
            Ok(p) => FileMsg::Path(p.to_string_lossy().to_string()),
            Err(e) => io_err(e),
        },
        FileOp::Stat { path } => match entry_of(&resolve(ctx, &path), None) {
            Ok(e) => FileMsg::Entry(e),
            Err(e) => io_err(e),
        },
        FileOp::List { path } => {
            let p = resolve(ctx, &path);
            match std::fs::read_dir(&p) {
                Ok(rd) => {
                    let mut v: Vec<FileEntry> = rd.filter_map(|e| e.ok()).filter_map(|e| entry_of(&e.path(), Some(e.file_name().to_string_lossy().to_string())).ok()).take(20_000).collect();
                    v.sort_by(|a, b| (b.kind == FileKind::Dir).cmp(&(a.kind == FileKind::Dir)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
                    FileMsg::Entries(v)
                }
                Err(e) => io_err(e),
            }
        }
        FileOp::Mkdir { path, parents } => {
            let p = resolve(ctx, &path);
            let r = if parents { std::fs::create_dir_all(&p) } else { std::fs::create_dir(&p) };
            match r {
                Ok(()) => FileMsg::Ok,
                Err(e) => io_err(e),
            }
        }
        FileOp::Remove { path, recursive, origin } => {
            let p = resolve(ctx, &path);
            let _ = backup(ctx, &p, &origin);
            let r = match std::fs::symlink_metadata(&p) {
                Ok(m) if m.is_dir() => {
                    if recursive {
                        std::fs::remove_dir_all(&p)
                    } else {
                        std::fs::remove_dir(&p)
                    }
                }
                Ok(_) => std::fs::remove_file(&p),
                Err(e) => Err(e),
            };
            match r {
                Ok(()) => FileMsg::Ok,
                Err(e) => io_err(e),
            }
        }
        FileOp::Rename { from, to, overwrite, origin } => {
            let (a, b) = (resolve(ctx, &from), resolve(ctx, &to));
            if !overwrite && b.exists() {
                err("exists", "目标已存在")
            } else {
                if b.exists() {
                    let _ = backup(ctx, &b, &origin);
                }
                let _ = backup(ctx, &a, &origin);
                match std::fs::rename(&a, &b) {
                    Ok(()) => FileMsg::Ok,
                    Err(e) => io_err(e),
                }
            }
        }
        FileOp::Chmod { path, mode } => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                match std::fs::set_permissions(resolve(ctx, &path), std::fs::Permissions::from_mode(mode & 0o7777)) {
                    Ok(()) => FileMsg::Ok,
                    Err(e) => io_err(e),
                }
            }
            #[cfg(not(unix))]
            {
                let _ = (path, mode);
                err("unsupported", "此平台不支持修改权限")
            }
        }
        FileOp::Search { root, pattern, content, max } => {
            let root = resolve(ctx, &root);
            let max = max.clamp(1, 2000) as usize;
            let pat = pattern.to_lowercase();
            let res = tokio::task::spawn_blocking(move || search(&root, &pat, content, max)).await.unwrap_or_default();
            FileMsg::Entries(res)
        }
        FileOp::Read { path, offset, len } => {
            let p = resolve(ctx, &path);
            match tokio::fs::File::open(&p).await {
                Ok(mut f) => {
                    let size = f.metadata().await.map(|m| m.len()).unwrap_or(0);
                    if f.metadata().await.map(|m| m.is_dir()).unwrap_or(false) {
                        err("is_dir", "这是一个目录")
                    } else {
                        fio::write(w, &FileMsg::Size(size)).await?;
                        f.seek(std::io::SeekFrom::Start(offset)).await?;
                        let mut remaining = len.unwrap_or(u64::MAX);
                        let mut buf = vec![0u8; CHUNK];
                        while remaining > 0 {
                            let want = (remaining.min(CHUNK as u64)) as usize;
                            let n = f.read(&mut buf[..want]).await?;
                            if n == 0 {
                                break;
                            }
                            fio::write(w, &FileMsg::Chunk(buf[..n].to_vec())).await?;
                            remaining -= n as u64;
                        }
                        FileMsg::End
                    }
                }
                Err(e) => io_err(e),
            }
        }
        FileOp::Write { path, offset, truncate, mode, expect_sha256, origin } => {
            let p = resolve(ctx, &path);
            let mut backup_path = None;
            let mut mismatch = false;
            if let Some(exp) = &expect_sha256 {
                let cur = std::fs::read(&p).map(|b| galaxy_crypto::sha256(&b).to_vec()).unwrap_or_default();
                if &cur != exp {
                    mismatch = true;
                }
            }
            if mismatch {
                // 读掉对方已发的数据
                while let Some(FileMsg::Chunk(_)) = fio::read::<_, FileMsg>(r).await? {}
                err("conflict", "文件已被其他人修改，请重新读取后再改")
            } else {
                if p.exists() {
                    backup_path = backup(ctx, &p, &origin);
                }
                let mut opts = tokio::fs::OpenOptions::new();
                opts.write(true).create(true);
                if truncate && offset == 0 {
                    opts.truncate(true);
                }
                #[cfg(unix)]
                if let Some(m) = mode {
                    opts.mode(m & 0o777);
                }
                #[cfg(not(unix))]
                let _ = mode;
                match opts.open(&p).await {
                    Ok(mut f) => {
                        if offset > 0 {
                            f.seek(std::io::SeekFrom::Start(offset)).await?;
                        }
                        let mut hasher_data: Vec<u8> = Vec::new();
                        let small = offset == 0;
                        let mut total = offset;
                        loop {
                            match fio::read::<_, FileMsg>(r).await? {
                                Some(FileMsg::Chunk(d)) => {
                                    tracing::debug!("写入块 {} 字节", d.len());
                                    f.write_all(&d).await?;
                                    total += d.len() as u64;
                                    if small && hasher_data.len() < 64 * 1024 * 1024 {
                                        hasher_data.extend_from_slice(&d);
                                    }
                                }
                                Some(FileMsg::End) | None => break,
                                Some(_) => {}
                            }
                        }
                        tracing::debug!("写入结束，共 {total} 字节");
                        f.flush().await?;
                        f.sync_all().await.ok();
                        FileMsg::Written { size: total, sha256: if small { galaxy_crypto::sha256(&hasher_data).to_vec() } else { vec![] }, backup: backup_path }
                    }
                    Err(e) => io_err(e),
                }
            }
        }
    };
    fio::write(w, &reply).await?;
    w.flush().await?;
    Ok(())
}

fn search(root: &Path, pat: &str, content: bool, max: usize) -> Vec<FileEntry> {
    let mut out = vec![];
    let mut stack = vec![(root.to_path_buf(), 0u32)];
    let mut visited = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        if out.len() >= max || visited > 200_000 {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            visited += 1;
            let name = e.file_name().to_string_lossy().to_string();
            let path = e.path();
            let Ok(ft) = e.file_type() else { continue };
            let hit = name.to_lowercase().contains(pat)
                || (content && ft.is_file() && e.metadata().map(|m| m.len() < 2 * 1024 * 1024).unwrap_or(false) && std::fs::read(&path).map(|b| String::from_utf8_lossy(&b).to_lowercase().contains(pat)).unwrap_or(false));
            if hit {
                if let Ok(en) = entry_of(&path, Some(name.clone())) {
                    out.push(en);
                    if out.len() >= max {
                        break;
                    }
                }
            }
            if ft.is_dir() && depth < 12 && !name.starts_with('.') && name != "node_modules" && name != "proc" && name != "sys" {
                stack.push((path, depth + 1));
            }
        }
    }
    out
}

/// 降权子进程入口：`<程序> __lg-file-helper <uid> <gid> <home> [backup_dir]`。
/// 先切换到目标用户，再从标准输入读一个 FileOp 并处理。
pub fn helper_main(args: &[String]) -> i32 {
    #[cfg(unix)]
    {
        let (Some(uid), Some(gid), Some(home)) = (args.first().and_then(|s| s.parse::<u32>().ok()), args.get(1).and_then(|s| s.parse::<u32>().ok()), args.get(2)) else {
            return 2;
        };
        let backup = args.get(3).filter(|s| !s.is_empty()).map(PathBuf::from);
        if nix::unistd::getuid().as_raw() != uid {
            let user = nix::unistd::User::from_uid(uid.into()).ok().flatten();
            let groups: Vec<nix::unistd::Gid> = user
                .as_ref()
                .and_then(|u| {
                    #[cfg(not(target_os = "macos"))]
                    {
                        std::ffi::CString::new(u.name.clone()).ok().and_then(|c| nix::unistd::getgrouplist(&c, u.gid).ok())
                    }
                    #[cfg(target_os = "macos")]
                    {
                        let _ = u;
                        None
                    }
                })
                .unwrap_or_else(|| vec![gid.into()]);
            #[cfg(not(target_os = "macos"))]
            if nix::unistd::setgroups(&groups).is_err() {
                return 3;
            }
            #[cfg(target_os = "macos")]
            let _ = groups;
            if nix::unistd::setgid(gid.into()).is_err() || nix::unistd::setuid(uid.into()).is_err() {
                return 3;
            }
        }
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build();
        let Ok(rt) = rt else { return 4 };
        rt.block_on(async move {
            let mut stdin = tokio::io::stdin();
            let mut stdout = tokio::io::stdout();
            let ctx = FileCtx { home: PathBuf::from(home), backup_dir: backup };
            let Ok(Some(op)) = fio::read::<_, FileOp>(&mut stdin).await else { return 5 };
            match serve(op, &mut stdin, &mut stdout, &ctx).await {
                Ok(()) => 0,
                Err(_) => 6,
            }
        })
    }
    #[cfg(not(unix))]
    {
        let _ = args;
        1
    }
}
