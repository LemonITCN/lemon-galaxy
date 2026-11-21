// 建议内核补充、但当前 galaxy-ffi 还没有的接口。
// 界面用 `client as? GalaxyClientSshProposed` 判断：模拟内核实现了，真内核没实现时对应按钮自动置灰。
// 不属于 uniffi 生成代码；内核加上同名接口后，删掉这里的定义、给生成的类补一个协议遵循扩展即可。
import Foundation

/// SSH 直连时向用户要密码、私钥口令、键盘交互答案（spec 3.3）。内核在自己的线程上同步调用。
public protocol SecretPrompt: AnyObject {
    func ask(prompt: String, echo: Bool) -> String?
}

/// SSH 直连会话（spec 3.3：终端、SFTP、命令执行、端口转发）。
public protocol SshSessionProposed: AnyObject {
    func openTerminal(cols: UInt16, rows: UInt16, sink: TerminalSink) async throws -> TerminalChannel
    func files() -> FileClient
    func exec(request: ExecRequest) async throws -> ExecResult
    func forwardLocal(localPort: UInt16, remoteHost: String, remotePort: UInt16) async throws -> PortForward
    func disconnect()
}

public protocol GalaxyClientSshProposed: AnyObject {
    /// 连接加密库里的 ssh_host 条目（可经由某台设备跳转）
    func connectSsh(hostId: String, secretPrompt: SecretPrompt) async throws -> SshSessionProposed
}
