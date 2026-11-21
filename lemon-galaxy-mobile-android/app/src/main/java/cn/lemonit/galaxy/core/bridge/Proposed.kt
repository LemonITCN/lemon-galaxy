package cn.lemonit.galaxy.core.bridge

/*
 * 界面需要、但当前 galaxy-ffi 还没有的接口（见报告）。
 *
 * 目前只剩 SSH 直连：ffi.md 第 6 节写了 connect_ssh / SshSession / SecretPrompt，内核还没导出。
 * 模拟内核实现了它；真内核没有时，界面通过 `client as? SshClientInterface` 判断，SSH 主机的终端 / 文件 / AI 入口显示为不可用。
 */

/** 键盘交互认证、私钥口令、首次连接确认主机指纹。内核在后台线程同步等待返回。 */
interface SecretPrompt {
    fun ask(prompt: String, echo: Boolean): String?
}

interface SshSessionInterface {
    suspend fun openTerminal(cols: UShort, rows: UShort, sink: TerminalSink): TerminalChannelInterface
    fun files(): FileClientInterface
    suspend fun exec(request: ExecRequest): ExecResult
    suspend fun forwardLocal(localPort: UShort, remoteHost: String, remotePort: UShort): PortForwardInterface
    fun close()
}

interface SshClientInterface {
    /** vault 里的 ssh_host。 */
    suspend fun connectSsh(hostId: String, secretPrompt: SecretPrompt): SshSessionInterface
}
