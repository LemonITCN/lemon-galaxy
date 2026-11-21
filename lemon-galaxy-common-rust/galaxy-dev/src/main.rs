//! 开发与集成测试用的命令行主控端。
//!
//! ```text
//! galaxy-dev --data <目录> login <服务器地址> <用户名> <密码> [首次登录的新密码]
//! galaxy-dev --data <目录> devices
//! galaxy-dev --data <目录> exec <设备ID前缀> <命令>
//! galaxy-dev --data <目录> ls <设备> <路径>
//! galaxy-dev --data <目录> put <设备> <本地文件> <远端路径>
//! galaxy-dev --data <目录> get <设备> <远端路径> <本地文件>
//! galaxy-dev --data <目录> term-test <设备>     打开终端、执行命令、断开、接续，验证断线继承
//! galaxy-dev --data <目录> assist <设备码> <验证码> <命令>
//! galaxy-dev --data <目录> vault-test
//! galaxy-dev release-sign <文件>            用环境变量 LEMONGALAXY_RELEASE_SIGNING_KEY（base64 私钥种子）签名，写出 <文件>.sig
//! galaxy-dev release-verify <文件> [签名文件]  用内置发布公钥验签
//! galaxy-dev ssh-test <主机> <端口> <用户名> <私钥文件>   SSH 直连：终端、命令、SFTP、端口转发
//! ```

use galaxy_client::account::{self, DeviceMeta, LoginOutcome};
use galaxy_client::api::Api;
use galaxy_client::connector::Connector;
use galaxy_client::identity::{FileStore, Identity, SecureStore};
use galaxy_client::session::{PeerSession, TerminalSink, TransferProgress};
use galaxy_client::signal::SignalClient;
use galaxy_crypto::AccountSecrets;
use galaxy_net::ServerAddr;
use galaxy_proto::cert::SignedBlob;
use galaxy_proto::peer::{ExecRequest, Origin, TerminalOpen};
use galaxy_proto::{DeviceId, DeviceKind, Platform};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[derive(Serialize, Deserialize)]
struct Saved {
    server: String,
    token: String,
    user_id: u64,
    username: String,
    account_pubkey: Vec<u8>,
    epoch: u32,
    cert: SignedBlob,
    secrets: Vec<u8>,
}

fn meta() -> DeviceMeta {
    DeviceMeta {
        kind: DeviceKind::Control,
        platform: if cfg!(target_os = "macos") { Platform::Macos } else { Platform::Linux },
        os_version: "dev".into(),
        arch: std::env::consts::ARCH.into(),
        hostname: "galaxy-dev".into(),
        app_version: galaxy_proto::APP_VERSION.into(),
    }
}

fn store(dir: &Path) -> Arc<dyn SecureStore> {
    Arc::new(FileStore::new(dir.join("keys")).unwrap())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    galaxy_net::init_crypto();
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,quinn=warn".into())).with_writer(std::io::stderr).init();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut dir = PathBuf::from("./.galaxy-dev");
    if args.first().map(|s| s.as_str()) == Some("--data") {
        dir = PathBuf::from(args[1].clone());
        args.drain(0..2);
    }
    let cmd = args.first().cloned().unwrap_or_default();
    // 不需要设备身份的命令（不建数据目录）
    match cmd.as_str() {
        "release-sign" => {
            use base64::Engine;
            let seed = std::env::var("LEMONGALAXY_RELEASE_SIGNING_KEY").map_err(|_| anyhow::anyhow!("缺少环境变量 LEMONGALAXY_RELEASE_SIGNING_KEY"))?;
            let seed = base64::engine::general_purpose::STANDARD.decode(seed.trim())?;
            let key = galaxy_crypto::SignKey::from_bytes(&seed)?;
            anyhow::ensure!(key.public() == galaxy_proto::update::RELEASE_SIGN_PUBKEY, "签名私钥与内置发布公钥不匹配");
            let data = std::fs::read(&args[1])?;
            let sig = key.sign(galaxy_proto::update::RELEASE_SIGN_DOMAIN, &data);
            std::fs::write(format!("{}.sig", args[1]), base64::engine::general_purpose::STANDARD.encode(sig) + "\n")?;
            println!("已签名 {}", args[1]);
            return Ok(());
        }
        "release-verify" => {
            use base64::Engine;
            let data = std::fs::read(&args[1])?;
            let sig_path = args.get(2).cloned().unwrap_or_else(|| format!("{}.sig", args[1]));
            let sig = base64::engine::general_purpose::STANDARD.decode(std::fs::read_to_string(sig_path)?.trim())?;
            galaxy_crypto::verify(&galaxy_proto::update::RELEASE_SIGN_PUBKEY, galaxy_proto::update::RELEASE_SIGN_DOMAIN, &data, &sig).map_err(|_| anyhow::anyhow!("签名无效"))?;
            println!("签名有效");
            return Ok(());
        }
        "ssh-test" => {
            ssh_test(&args[1], args[2].parse()?, &args[3], &std::fs::read_to_string(&args[4])?).await?;
            return Ok(());
        }
        _ => {}
    }
    std::fs::create_dir_all(&dir)?;
    let id = Identity::load_or_create(&store(&dir));
    match cmd.as_str() {
        "login" => {
            let server = ServerAddr::parse(&args[1]).map_err(anyhow::Error::msg)?;
            let api = Api::new(server.clone(), "zh-CN")?;
            let out = account::login(&api, &id, &meta(), &args[2], &args[3], None).await?;
            let l = match out {
                LoginOutcome::Done(l) => l,
                LoginOutcome::MustChangePassword(fl) => {
                    let np = args.get(4).ok_or_else(|| anyhow::anyhow!("首次登录需要提供新密码"))?;
                    account::complete_first_login(&api, &id, &meta(), fl, np).await?
                }
                LoginOutcome::NeedTotp => anyhow::bail!("需要两步验证码"),
            };
            let secrets = l.secrets.clone().unwrap();
            let saved = Saved {
                server: server.to_string_full(),
                token: l.token.clone(),
                user_id: l.user.id,
                username: l.user.username.clone(),
                account_pubkey: l.account_pubkey.to_vec(),
                epoch: l.epoch,
                cert: l.cert.clone(),
                secrets: secrets.wrap_local(&id.local_wrap_key()),
            };
            std::fs::write(dir.join("controller.json"), serde_json::to_vec_pretty(&saved)?)?;
            println!("登录成功：{}（用户 {}）", l.user.username, l.user.id);
        }
        "devices" => {
            let (api, _s, _) = load(&dir, &id)?;
            let resp: galaxy_proto::api::ListResp<galaxy_proto::api::ClientDevice> = api.get("/devices").await?;
            for d in resp.items {
                println!("{}  {:<8} {:<8} {:<20} {}", d.id, d.kind.as_str(), d.platform.as_str(), d.hostname, if d.online { "在线" } else { "离线" });
            }
        }
        "vault-test" => {
            let (api, s, secrets) = load(&dir, &id)?;
            let mut v = galaxy_client::vault::Vault::new(s.user_id, &secrets);
            v.sync(&api).await?;
            let e = v
                .save(&api, galaxy_client::vault::VaultEntry {
                    id: String::new(),
                    folder_id: None,
                    sort: 0,
                    data: galaxy_client::vault::VaultData::Folder { name: "生产服务器".into(), color: None, expanded: true },
                    updated_at: 0,
                    version: 0,
                })
                .await?;
            let mut v2 = galaxy_client::vault::Vault::new(s.user_id, &secrets);
            v2.sync(&api).await?;
            println!("加密库条目 {} 个，刚写入的 {} 能解密：{}", v2.entries.len(), e.id, v2.entries.contains_key(&e.id));
        }
        "assist" => {
            let (session, _c) = connect(&dir, &id, None, Some((&args[1], &args[2]))).await?;
            run_exec(&session, &args[3]).await?;
        }
        _ => {
            let target = args.get(1).cloned().unwrap_or_default();
            let (session, _c) = connect(&dir, &id, Some(&target), None).await?;
            println!("已连接，路径：{:?}，延迟 {} ms", session.path().kind, session.path().rtt_ms);
            match cmd.as_str() {
                "exec" => run_exec(&session, &args[2..].join(" ")).await?,
                "ls" => {
                    let p = args.get(2).cloned().unwrap_or_else(|| "~".into());
                    for e in session.fs_list(&p).await? {
                        println!("{:?}\t{:>10}\t{:o}\t{}", e.kind, e.size, e.mode, e.name);
                    }
                }
                "put" => {
                    struct P;
                    impl TransferProgress for P {
                        fn on_progress(&self, d: u64, t: u64) {
                            eprint!("\r{d}/{t}");
                        }
                        fn on_finished(&self, _e: Option<String>) {}
                    }
                    let (_tx, rx) = tokio::sync::watch::channel(false);
                    session.upload(Path::new(&args[2]), &args[3], Arc::new(P), rx).await?;
                    println!("\n上传完成");
                }
                "get" => {
                    struct P;
                    impl TransferProgress for P {
                        fn on_progress(&self, d: u64, t: u64) {
                            eprint!("\r{d}/{t}");
                        }
                        fn on_finished(&self, _e: Option<String>) {}
                    }
                    let (_tx, rx) = tokio::sync::watch::channel(false);
                    session.download(&args[2], Path::new(&args[3]), Arc::new(P), rx).await?;
                    println!("\n下载完成");
                }
                "term-test" => term_test(&session).await?,
                "ai" => ai_run(&dir, &id, session.clone(), &args[2..]).await?,
                "stat" => println!("{:?}", session.fs_stat(&args[2]).await),
                "watch" => {
                    let mut p = session.path_watch();
                    for _ in 0..20 {
                        println!("路径 {:?} rtt {} ms {}", p.borrow().kind, p.borrow().rtt_ms, p.borrow().remote);
                        let _ = tokio::time::timeout(Duration::from_secs(3), p.changed()).await;
                    }
                }
                _ => println!("未知命令"),
            }
            session.close();
        }
    }
    Ok(())
}

fn load(dir: &Path, id: &Identity) -> anyhow::Result<(Api, Saved, AccountSecrets)> {
    let s: Saved = serde_json::from_slice(&std::fs::read(dir.join("controller.json"))?)?;
    let api = Api::new(ServerAddr::parse(&s.server).map_err(anyhow::Error::msg)?, "zh-CN")?;
    api.set_token(Some(s.token.clone()));
    let secrets = AccountSecrets::unwrap_local(&id.local_wrap_key(), &s.secrets)?;
    Ok((api, s, secrets))
}

async fn connect(dir: &Path, id: &Identity, target: Option<&str>, assist: Option<(&str, &str)>) -> anyhow::Result<(Arc<PeerSession>, Arc<Connector>)> {
    let (api, s, _secrets) = load(dir, id)?;
    let server = ServerAddr::parse(&s.server).map_err(anyhow::Error::msg)?;
    let signal = SignalClient::start(server.clone(), id.clone(), Some(s.token.clone()), None);
    if !signal.wait_connected(Duration::from_secs(5)).await {
        eprintln!("连不上服务器，尝试局域网离线连接");
    }
    let ms = galaxy_net::MagicSock::bind(0)?;
    let ep = galaxy_net::quic::endpoint(ms.clone(), false)?;
    let pk: [u8; 32] = s.account_pubkey.as_slice().try_into()?;
    let c = Connector::new(ms, ep, signal, id.clone(), server, pk, s.cert.clone());
    let session = match assist {
        Some((dc, code)) => c.connect_assist(dc, code).await?,
        None if target.map(|t| t.len() == 32).unwrap_or(false) => c.connect(DeviceId::from_hex(target.unwrap()).unwrap()).await?,
        None => {
            let resp: galaxy_proto::api::ListResp<galaxy_proto::api::ClientDevice> = api.get("/devices").await?;
            let t = target.unwrap_or("");
            let dev = resp
                .items
                .iter()
                .find(|d| d.id.to_hex().starts_with(t) && d.id != id.device_id && d.kind.can_host())
                .map(|d| d.id)
                .ok_or_else(|| anyhow::anyhow!("找不到设备 {t}"))?;
            c.connect(dev).await?
        }
    };
    let _ = DeviceId::default();
    Ok((session, c))
}

async fn run_exec(session: &PeerSession, cmd: &str) -> anyhow::Result<()> {
    let req = ExecRequest { command: cmd.into(), shell: None, cwd: None, env: vec![], timeout_ms: 60_000, origin: Origin::User, max_output: 1 << 20 };
    let (done, _o, _e) = session
        .exec(
            req,
            |err, d| {
                use std::io::Write;
                if err {
                    let _ = std::io::stderr().write_all(d);
                } else {
                    let _ = std::io::stdout().write_all(d);
                }
            },
            None,
        )
        .await?;
    println!("\n[退出码 {:?}，{} ms]", done.code, done.duration_ms);
    Ok(())
}

struct Collect(Arc<std::sync::Mutex<Vec<u8>>>);
impl TerminalSink for Collect {
    fn on_output(&self, data: Vec<u8>) {
        self.0.lock().unwrap().extend_from_slice(&data);
    }
    fn on_exit(&self, code: Option<i32>) {
        eprintln!("[终端退出 {code:?}]");
    }
}

/// 等输出里出现某段文字（最多 `secs` 秒）。
async fn wait_for(buf: &Arc<std::sync::Mutex<Vec<u8>>>, needle: &str, secs: u64) -> String {
    let end = std::time::Instant::now() + Duration::from_secs(secs);
    loop {
        let s = String::from_utf8_lossy(&buf.lock().unwrap()).to_string();
        if s.contains(needle) || std::time::Instant::now() > end {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn term_test(session: &Arc<PeerSession>) -> anyhow::Result<()> {
    // 按被控端默认 shell 选命令（Windows 默认 PowerShell）
    let shell = session.ack.host.shell.to_ascii_lowercase();
    let (set_cmd, echo_cmd): (&[u8], &[u8]) = if shell.contains("powershell") || shell.contains("pwsh") {
        (b"$env:LG_MARK='persist-ok'; echo \"first-$(40+2)\"\r", b"echo \"again-$env:LG_MARK\"\r")
    } else if shell.ends_with("cmd.exe") || shell.ends_with("cmd") {
        (b"set LG_MARK=persist-ok & set /a 40+2 >nul & echo first-42\r", b"echo again-%LG_MARK%\r")
    } else {
        (b"export LG_MARK=persist-ok; echo first-$((40+2))\r", b"echo again-$LG_MARK\r")
    };
    let buf = Arc::new(std::sync::Mutex::new(Vec::new()));
    let t = session
        .open_terminal(TerminalOpen { attach: None, shell: None, cols: 100, rows: 30, env: vec![], origin: Origin::User }, Arc::new(Collect(buf.clone())))
        .await?;
    println!("终端会话 {}（{}，shell {}）", t.session_id, t.title, session.ack.host.shell);
    tokio::time::sleep(Duration::from_millis(800)).await;
    t.write(set_cmd.to_vec()).await?;
    let out1 = wait_for(&buf, "first-42", 15).await;
    println!("第一次输出包含 first-42：{}", out1.contains("first-42"));
    t.detach().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let list = session.list_terminals().await?;
    println!("被控端保留的会话：{}", list.iter().map(|x| format!("{}{}", x.id, if x.attached { "(已接)" } else { "" })).collect::<Vec<_>>().join(", "));
    let buf2 = Arc::new(std::sync::Mutex::new(Vec::new()));
    let t2 = session
        .open_terminal(TerminalOpen { attach: Some(t.session_id.clone()), shell: None, cols: 100, rows: 30, env: vec![], origin: Origin::User }, Arc::new(Collect(buf2.clone())))
        .await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    t2.write(echo_cmd.to_vec()).await?;
    let out2 = wait_for(&buf2, "again-persist-ok", 15).await;
    println!("接续后回放了之前的输出：{}", out2.contains("first-42"));
    println!("接续后环境变量还在（断线继承）：{}", out2.contains("again-persist-ok"));
    let snap = t2.snapshot().await?;
    println!("屏幕快照（AI 读屏）行数：{}", snap.lines().count());
    t2.kill().await;
    Ok(())
}


struct PrintUi {
    yes: bool,
    agent: std::sync::Mutex<Option<std::sync::Weak<galaxy_agent::agent::Agent>>>,
}

impl galaxy_agent::agent::AgentUi for PrintUi {
    fn event(&self, e: galaxy_agent::agent::AgentEvent) {
        use galaxy_agent::agent::AgentEvent::*;
        use std::io::Write;
        match e {
            AssistantDelta { text, .. } => {
                print!("{text}");
                let _ = std::io::stdout().flush();
            }
            AssistantDone { .. } => println!(),
            Thinking { .. } => {}
            ToolProposed { call, needs_approval } => {
                println!("\n[工具] {} @{} 风险={:?} {}  {}", call.tool, call.target_name, call.risk, call.summary, if needs_approval { if self.yes { "（自动允许）" } else { "（自动拒绝）" } } else { "" });
                if needs_approval {
                    if let Some(a) = self.agent.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
                        let d = if self.yes { galaxy_agent::agent::ApprovalDecision::AllowOnce } else { galaxy_agent::agent::ApprovalDecision::Deny { reason: Some("测试中自动拒绝".into()) } };
                        let id = call.call_id.clone();
                        tokio::spawn(async move { a.decide(&id, d) });
                    }
                }
            }
            ToolFinished { result, .. } => println!("[结果] {} {}", if result.ok { "成功" } else { "失败" }, result.summary),
            Usage { prompt_tokens, completion_tokens } => println!("[用量] 输入 {prompt_tokens} 输出 {completion_tokens}"),
            Error { message } => println!("[错误] {message}"),
            AskUser { question, .. } => println!("[提问] {question}"),
            _ => {}
        }
    }
}

async fn ai_run(dir: &Path, id: &Identity, session: Arc<PeerSession>, rest: &[String]) -> anyhow::Result<()> {
    let (api, _s, _) = load(dir, id)?;
    let models: galaxy_proto::api::AiModelsResp = api.get("/ai/models").await?;
    anyhow::ensure!(models.enabled && !models.items.is_empty(), "账号未开通 AI 或没有可用模型");
    let want = std::env::var("LG_MODEL").unwrap_or_else(|_| "mimo-v2.6-flash".into());
    let m = models.items.iter().find(|m| m.id == want).or(models.items.iter().find(|m| m.tools)).unwrap().clone();
    println!("模型：{}（{}）看图={} 管理员上限={:?}", m.name, m.id, m.vision, models.max_mode);
    let yes = rest.iter().any(|a| a == "--yes");
    let mode = rest.iter().position(|a| a == "--mode").and_then(|i| rest.get(i + 1)).and_then(|s| galaxy_proto::AiMode::parse(s)).unwrap_or(galaxy_proto::AiMode::ConfirmDangerous);
    let prompt: String = rest.iter().filter(|a| !a.starts_with("--") && galaxy_proto::AiMode::parse(a).is_none()).cloned().collect::<Vec<_>>().join(" ");
    let exec = galaxy_client::ai::SessionExecutor::new();
    exec.add("dev1".into(), session.clone(), session.ack.host.hostname.clone());
    let ui = Arc::new(PrintUi { yes, agent: std::sync::Mutex::new(None) });
    let cfg = galaxy_agent::agent::AgentConfig {
        conversation_id: format!("dev-{}", galaxy_proto::now_ms()),
        provider_id: m.provider_id,
        model_id: m.id.clone(),
        vision: m.vision,
        context_length: m.context_length,
        mode,
        admin_cap: models.max_mode,
        plan_first: false,
        max_steps: 20,
        locale: "zh-CN".into(),
        trust_minutes: None,
    };
    let agent = galaxy_agent::agent::Agent::new(cfg, Arc::new(galaxy_client::ai::GatewayLlm { api }), exec, ui.clone());
    *ui.agent.lock().unwrap() = Some(Arc::downgrade(&agent));
    agent.send(prompt).await;
    Ok(())
}

struct AutoYes;
impl galaxy_client::ssh::SshPrompt for AutoYes {
    fn ask(&self, prompt: &str, _echo: bool) -> Option<String> {
        eprintln!("[SSH 询问] {prompt}");
        None
    }
    fn confirm_host_key(&self, host: &str, port: u16, key_type: &str, fingerprint: &str, changed: bool) -> bool {
        println!("主机密钥 {host}:{port} {key_type} {fingerprint}{}", if changed { "（变了）" } else { "" });
        true
    }
}

struct Prog;
impl TransferProgress for Prog {
    fn on_progress(&self, _d: u64, _t: u64) {}
    fn on_finished(&self, _e: Option<String>) {}
}

/// SSH 直连自测：主机密钥、终端、命令、SFTP 上传下载、端口转发。
async fn ssh_test(host: &str, port: u16, user: &str, key: &str) -> anyhow::Result<()> {
    use galaxy_client::ssh::*;
    let t = SshTarget { host: host.into(), port, username: user.into(), auth: SshAuthMethod::PrivateKey { key: key.into(), passphrase: None }, via: SshVia::Direct, known: None };
    let c = SshSession::connect(t, Arc::new(AutoYes)).await?;
    println!("已连接，需要存回加密库的主机密钥：{:?}", c.new_host_key.as_ref().map(|k| &k.fingerprint));
    let s = c.session;
    // 再连一次，带上记录的主机密钥，不应再询问
    let t2 = SshTarget { host: host.into(), port, username: user.into(), auth: SshAuthMethod::PrivateKey { key: key.into(), passphrase: None }, via: SshVia::Direct, known: c.new_host_key.clone() };
    let c2 = SshSession::connect(t2, Arc::new(AutoYes)).await?;
    println!("第二次连接主机密钥无变化：{}", c2.new_host_key.is_none());
    c2.session.disconnect().await;

    let mut out = Vec::new();
    let (code, timed_out, ms) = s.exec("echo ssh-exec-ok; echo err-line 1>&2; exit 3", Duration::from_secs(20), |_, d| out.extend_from_slice(d), None).await?;
    println!("命令输出：{:?} 退出码 {code:?} 超时 {timed_out} {ms} ms", String::from_utf8_lossy(&out).trim());

    let buf = Arc::new(std::sync::Mutex::new(Vec::new()));
    let term = s.open_terminal(100, 30, &[("TERM".into(), "xterm-256color".into())], Arc::new(Collect(buf.clone()))).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    term.write(b"echo term-$((40+2))\r".to_vec());
    let o = wait_for(&buf, "term-42", 10).await;
    println!("终端输出包含 term-42：{}", o.contains("term-42"));
    term.resize(120, 40);
    term.close();

    println!("SFTP 家目录：{}", s.fs_home().await?);
    let dir = format!("lg-ssh-test-{}", std::process::id());
    s.fs_mkdir(&format!("~/{dir}/a/b")).await?;
    let local = std::env::temp_dir().join(format!("lg-ssh-{}.bin", std::process::id()));
    let data: Vec<u8> = (0..3_000_000u32).map(|i| (i * 7 % 251) as u8).collect();
    std::fs::write(&local, &data)?;
    let (_tx, rx) = tokio::sync::watch::channel(false);
    s.upload(&local, &format!("~/{dir}/a/up.bin"), Arc::new(Prog), rx.clone()).await?;
    let back = local.with_extension("back");
    s.download(&format!("~/{dir}/a/up.bin"), &back, Arc::new(Prog), rx).await?;
    println!("SFTP 上传下载一致：{}", std::fs::read(&back)? == data);
    s.fs_write_all(&format!("~/{dir}/t.txt"), b"hello sftp").await?;
    println!("SFTP 读文本：{}", String::from_utf8_lossy(&s.fs_read_all(&format!("~/{dir}/t.txt"), 1024).await?));
    let list = s.fs_list(&format!("~/{dir}")).await?;
    println!("SFTP 列目录：{}", list.iter().map(|e| format!("{}({:?})", e.name, e.kind)).collect::<Vec<_>>().join(" "));
    s.fs_rename(&format!("~/{dir}/t.txt"), &format!("~/{dir}/t2.txt")).await?;
    s.fs_chmod(&format!("~/{dir}/t2.txt"), 0o600).await?;
    println!("SFTP 改名改权限后：{:o}", s.fs_stat(&format!("~/{dir}/t2.txt")).await?.mode);
    s.fs_remove(&format!("~/{dir}"), true).await?;
    println!("SFTP 递归删除后存在：{}", s.fs_exists(&format!("~/{dir}")).await?);
    let _ = std::fs::remove_file(&local);
    let _ = std::fs::remove_file(&back);

    // 端口转发：经 SSH 连回 sshd 自己，读到 SSH 版本行即通
    let (lp, h) = s.forward_local(0, "127.0.0.1".into(), port).await?;
    let mut sock = tokio::net::TcpStream::connect(("127.0.0.1", lp)).await?;
    let mut line = vec![0u8; 64];
    let n = tokio::time::timeout(Duration::from_secs(5), tokio::io::AsyncReadExt::read(&mut sock, &mut line)).await??;
    println!("端口转发读到：{}", String::from_utf8_lossy(&line[..n]).trim());
    h.abort();
    s.disconnect().await;
    println!("SSH 自测完成");
    Ok(())
}
