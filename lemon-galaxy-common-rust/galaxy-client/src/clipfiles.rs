//! 剪贴板里的文件（复制粘贴文件，像 UU 远程）。
//!
//! 本机复制、远端粘贴：把本机文件与文件夹传到被控端账号目录下的暂存区 `~/.lemon-galaxy/clipboard/<编号>/`，
//! 再把被控端剪贴板设成这些文件，对方在资源管理器 / 访达里直接粘贴。暂存区里一天前的内容顺手清掉。
//! 远端复制、本机粘贴：把远端剪贴板里的文件（含文件夹）下载到本机目录，界面再把本机剪贴板设成这些文件。
//! 超过上限时报错，提示改用文件管理传输。

use crate::error::{ClientError, Result};
use crate::session::{PeerSession, TransferProgress};
use galaxy_proto::peer::{ClipboardData, FileKind, FileOp};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::watch;

pub const MAX_BYTES: u64 = 4 << 30;
pub const MAX_FILES: usize = 10_000;
const STAGE_TTL_MS: i64 = 24 * 3600 * 1000;

fn too_big() -> ClientError {
    ClientError::Other("要复制的文件太多或太大（超过 4 GB 或 1 万个），请用文件管理传输".into())
}

fn base_name(p: &str) -> String {
    let t = p.trim_end_matches(['/', '\\']);
    t.rsplit(['/', '\\']).next().unwrap_or(t).to_string()
}

/// 把多个文件的进度汇总成一个总进度。
struct Sum {
    outer: Arc<dyn TransferProgress>,
    base: u64,
    total: u64,
    done: Arc<AtomicU64>,
}

impl TransferProgress for Sum {
    fn on_progress(&self, done: u64, _total: u64) {
        let d = self.base + done;
        self.done.store(d, Ordering::Relaxed);
        self.outer.on_progress(d.min(self.total), self.total);
    }
    fn on_finished(&self, _e: Option<String>) {}
}

/// 本机要传的内容：先建的目录（相对路径）与文件（本机路径、相对路径、大小）。
#[derive(Default, Debug)]
struct LocalPlan {
    dirs: Vec<String>,
    files: Vec<(PathBuf, String, u64)>,
    total: u64,
}

fn plan_local(paths: &[PathBuf]) -> Result<LocalPlan> {
    let mut plan = LocalPlan::default();
    fn walk(p: &Path, rel: String, plan: &mut LocalPlan) -> Result<()> {
        let md = std::fs::metadata(p).map_err(|e| ClientError::Other(format!("读不了 {}：{e}", p.display())))?;
        if md.is_dir() {
            plan.dirs.push(rel.clone());
            let mut entries: Vec<_> = std::fs::read_dir(p).map_err(|e| ClientError::Other(format!("读不了 {}：{e}", p.display())))?.filter_map(|e| e.ok()).collect();
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                let name = e.file_name().to_string_lossy().to_string();
                walk(&e.path(), format!("{rel}/{name}"), plan)?;
            }
        } else if md.is_file() {
            plan.total += md.len();
            plan.files.push((p.to_path_buf(), rel, md.len()));
        }
        if plan.files.len() > MAX_FILES || plan.total > MAX_BYTES {
            return Err(too_big());
        }
        Ok(())
    }
    for p in paths {
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
        walk(p, name, &mut plan)?;
    }
    Ok(plan)
}

/// 把本机文件 / 文件夹放到被控端剪贴板。返回被控端上的路径（协议格式）。
pub async fn push(session: &Arc<PeerSession>, locals: &[PathBuf], progress: Arc<dyn TransferProgress>, cancel: watch::Receiver<bool>) -> Result<Vec<String>> {
    if locals.is_empty() {
        return Err(ClientError::Other("没有要复制的文件".into()));
    }
    let plan = plan_local(locals)?;
    let home = session.fs_home().await?;
    let root = format!("{}/.lemon-galaxy/clipboard", home.trim_end_matches('/'));
    // 清掉一天前的暂存（失败不要紧）
    if let Ok(old) = session.fs_list(&root).await {
        let now = galaxy_proto::now_ms();
        for e in old {
            if now - e.modified_at > STAGE_TTL_MS {
                let _ = session.fs_op(FileOp::Remove { path: e.path, recursive: true, origin: galaxy_proto::peer::Origin::User }).await;
            }
        }
    }
    let stage = format!("{root}/{}-{:08x}", galaxy_proto::now_ms(), rand::random::<u32>());
    session.fs_op(FileOp::Mkdir { path: stage.clone(), parents: true }).await?;
    for d in &plan.dirs {
        session.fs_op(FileOp::Mkdir { path: format!("{stage}/{d}"), parents: true }).await?;
    }
    let done = Arc::new(AtomicU64::new(0));
    let mut base = 0u64;
    progress.on_progress(0, plan.total);
    for (local, rel, size) in &plan.files {
        if *cancel.borrow() {
            return Err(ClientError::Other("已取消".into()));
        }
        let p = Arc::new(Sum { outer: progress.clone(), base, total: plan.total, done: done.clone() });
        session.upload(local, &format!("{stage}/{rel}"), p, cancel.clone()).await?;
        base += size;
    }
    progress.on_progress(plan.total, plan.total);
    let paths: Vec<String> = locals
        .iter()
        .map(|p| format!("{stage}/{}", p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into())))
        .collect();
    session.clipboard_set(ClipboardData::Files(paths.clone())).await?;
    Ok(paths)
}

/// 把被控端剪贴板里的文件（含文件夹）下载到本机 `local_dir`。返回本机上的顶层路径。
pub async fn pull(session: &Arc<PeerSession>, remotes: &[String], local_dir: &Path, progress: Arc<dyn TransferProgress>, cancel: watch::Receiver<bool>) -> Result<Vec<PathBuf>> {
    if remotes.is_empty() {
        return Err(ClientError::Other("远端剪贴板里没有文件".into()));
    }
    // 先列出全部内容，确认没超上限
    let mut dirs: Vec<PathBuf> = vec![];
    let mut files: Vec<(String, PathBuf, u64)> = vec![];
    let mut total = 0u64;
    let mut tops = vec![];
    let mut stack: Vec<(String, PathBuf)> = vec![];
    for r in remotes {
        let local = local_dir.join(base_name(r));
        tops.push(local.clone());
        stack.push((r.clone(), local));
    }
    while let Some((remote, local)) = stack.pop() {
        let st = session.fs_stat(&remote).await?;
        match st.kind {
            FileKind::Dir => {
                dirs.push(local.clone());
                for e in session.fs_list(&remote).await? {
                    let l = local.join(&e.name);
                    match e.kind {
                        FileKind::Dir => stack.push((e.path, l)),
                        FileKind::File => {
                            total += e.size;
                            files.push((e.path, l, e.size));
                        }
                        _ => {}
                    }
                    if files.len() > MAX_FILES || total > MAX_BYTES {
                        return Err(too_big());
                    }
                }
            }
            FileKind::File => {
                total += st.size;
                files.push((remote, local, st.size));
            }
            _ => {}
        }
        if files.len() > MAX_FILES || total > MAX_BYTES {
            return Err(too_big());
        }
    }
    std::fs::create_dir_all(local_dir).map_err(|e| ClientError::Other(format!("无法创建 {}：{e}", local_dir.display())))?;
    for d in &dirs {
        std::fs::create_dir_all(d).map_err(|e| ClientError::Other(format!("无法创建 {}：{e}", d.display())))?;
    }
    let done = Arc::new(AtomicU64::new(0));
    let mut base = 0u64;
    progress.on_progress(0, total);
    for (remote, local, size) in &files {
        if *cancel.borrow() {
            return Err(ClientError::Other("已取消".into()));
        }
        if let Some(parent) = local.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let p = Arc::new(Sum { outer: progress.clone(), base, total, done: done.clone() });
        session.download(remote, local, p, cancel.clone()).await?;
        base += size;
    }
    progress.on_progress(total, total);
    Ok(tops)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(base_name("/C:/Users/a/报告.docx"), "报告.docx");
        assert_eq!(base_name("/home/a/dir/"), "dir");
        assert_eq!(base_name("C:\\x\\y.txt"), "y.txt");
    }

    #[test]
    fn plan_walks_dirs_and_limits() {
        let d = std::env::temp_dir().join(format!("lg-clip-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("a/b")).unwrap();
        std::fs::write(d.join("a/1.txt"), b"hello").unwrap();
        std::fs::write(d.join("a/b/2.txt"), b"world!").unwrap();
        std::fs::write(d.join("top.txt"), b"x").unwrap();
        let p = plan_local(&[d.join("a"), d.join("top.txt")]).unwrap();
        assert_eq!(p.dirs, vec!["a".to_string(), "a/b".to_string()]);
        let rels: Vec<_> = p.files.iter().map(|f| f.1.clone()).collect();
        assert_eq!(rels, vec!["a/1.txt", "a/b/2.txt", "top.txt"]);
        assert_eq!(p.total, 12);
        let _ = std::fs::remove_dir_all(&d);
    }
}
