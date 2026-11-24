# Code Signing Policy

LemonGalaxy is an open-source (MIT) remote control system published from the public repository
[LemonITCN/lemon-galaxy](https://github.com/LemonITCN/lemon-galaxy).

Windows release files are signed through the SignPath Foundation open-source program:

> Free code signing provided by [SignPath.io](https://about.signpath.io/), certificate by [SignPath Foundation](https://signpath.org/).

Until signing is approved and active, Windows release files are published unsigned, and the release notes say so.

## What gets signed

Only files built by GitHub Actions on GitHub-hosted runners, from source in this repository, are submitted for signing:

- Windows worker executables (`lemon-galaxy-worker-windows`, x86_64, aarch64, and the legacy builds for Windows 7 x64, Vista / 7 32-bit and XP 32-bit)
- Windows control client executables inside the release archives (`lemon-galaxy-control-windows`)
- Windows server executables inside the release archives (`lemon-galaxy-server`)
- LemonGalaxy virtual display driver package (`lemon-galaxy-worker-windows/driver`, x64)

Third-party binaries are never signed with this certificate. All bundled dependencies are open-source components
built from source by the same workflow, plus operating-system libraries.

## Team roles

| Role | Members |
|---|---|
| Committers and reviewers | [LemonITCN](https://github.com/LemonITCN) |
| Approvers | [LemonITCN](https://github.com/LemonITCN) |

Every signing request is approved manually by an approver in SignPath. All team members use multi-factor
authentication for GitHub and SignPath.

## Build and release controls

- Releases are built by the workflow in [`.github/workflows/build.yml`](.github/workflows/build.yml) from an exact
  commit on `main`.
- Every release file is published with its SHA-256 and a detached LemonGalaxy release signature (Ed25519). Clients
  verify both before installing an update.

## Privacy policy

LemonGalaxy is self-hosted. The programs connect only to the LemonGalaxy server that the user or their
administrator configures, and to devices the user chooses to connect to. Remote sessions are end-to-end encrypted
between devices.

- The server checks GitHub Releases of this repository for updates. Administrators can switch the update source to manual uploads in the admin console and upload
  updates manually.
- The AI assistant sends conversations and the context the user allows to the AI model provider that the
  server administrator configures. The assistant is not used unless the administrator sets up a provider.
- No telemetry or usage data is sent to the LemonGalaxy authors or any other party.

---

## 中文说明

本页是 LemonGalaxy 的代码签名策略，申请 SignPath Foundation 开源签名时需要。

- Windows 发布文件由 SignPath Foundation 免费签名，签名者显示为 SignPath Foundation。签名开通前，Windows 文件不签名，会在发布说明里写明。
- 只签本仓库源码由 GitHub Actions 构建出来的文件，不签第三方二进制。
- 每次签名都由维护者在 SignPath 上手动批准。所有成员的 GitHub 和 SignPath 账号都开启两步验证。
- 隐私：程序只连接用户自己配置的 LemonGalaxy 服务器，以及用户主动连接的设备，不向作者或任何第三方发送遥测数据。AI 助手只把对话发给管理员配置的模型服务商。
