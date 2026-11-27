package cn.lemonit.galaxy.core.rust

import cn.lemonit.galaxy.core.bridge.ActivityItem
import cn.lemonit.galaxy.core.bridge.AgentEvent
import cn.lemonit.galaxy.core.bridge.ContextRef
import cn.lemonit.galaxy.core.bridge.ContextRefKind
import cn.lemonit.galaxy.core.bridge.ConversationModel
import cn.lemonit.galaxy.core.bridge.AgentMessage
import cn.lemonit.galaxy.core.bridge.AgentTarget
import cn.lemonit.galaxy.core.bridge.AgentTargetKind
import cn.lemonit.galaxy.core.bridge.AiMode
import cn.lemonit.galaxy.core.bridge.AiModel
import cn.lemonit.galaxy.core.bridge.AiModels
import cn.lemonit.galaxy.core.bridge.ApprovalDecision
import cn.lemonit.galaxy.core.bridge.ClientState
import cn.lemonit.galaxy.core.bridge.ClipboardData
import cn.lemonit.galaxy.core.bridge.ComboKey
import cn.lemonit.galaxy.core.bridge.ConversationConfig
import cn.lemonit.galaxy.core.bridge.ConversationSummary
import cn.lemonit.galaxy.core.bridge.CoreConfig
import cn.lemonit.galaxy.core.bridge.DeviceInfo
import cn.lemonit.galaxy.core.bridge.DeviceKind
import cn.lemonit.galaxy.core.bridge.DisplayInfo
import cn.lemonit.galaxy.core.bridge.DisplayOptions
import cn.lemonit.galaxy.core.bridge.EnvVar
import cn.lemonit.galaxy.core.bridge.ExecRequest
import cn.lemonit.galaxy.core.bridge.ExecResult
import cn.lemonit.galaxy.core.bridge.FileEntry
import cn.lemonit.galaxy.core.bridge.FileKind
import cn.lemonit.galaxy.core.bridge.GalaxyException
import cn.lemonit.galaxy.core.bridge.ImageData
import cn.lemonit.galaxy.core.bridge.InputEvent
import cn.lemonit.galaxy.core.bridge.LoginResult
import cn.lemonit.galaxy.core.bridge.PathInfo
import cn.lemonit.galaxy.core.bridge.PathKind
import cn.lemonit.galaxy.core.bridge.PeerInfo
import cn.lemonit.galaxy.core.bridge.Permissions
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.core.bridge.PointerButton
import cn.lemonit.galaxy.core.bridge.RdpTarget
import cn.lemonit.galaxy.core.bridge.RiskLevel
import cn.lemonit.galaxy.core.bridge.ServerInfo
import cn.lemonit.galaxy.core.bridge.SshAuth
import cn.lemonit.galaxy.core.bridge.RecordingInfo
import cn.lemonit.galaxy.core.bridge.SshInfo
import cn.lemonit.galaxy.core.bridge.TerminalOptions
import cn.lemonit.galaxy.core.bridge.TerminalSessionInfo
import cn.lemonit.galaxy.core.bridge.ToolCallView
import cn.lemonit.galaxy.core.bridge.ToolResultView
import cn.lemonit.galaxy.core.bridge.TotpSetup
import cn.lemonit.galaxy.core.bridge.TouchPhase
import cn.lemonit.galaxy.core.bridge.UpdateInfo
import cn.lemonit.galaxy.core.bridge.UserInfo
import cn.lemonit.galaxy.core.bridge.VaultData
import cn.lemonit.galaxy.core.bridge.VaultEntry
import cn.lemonit.galaxy.core.bridge.VideoCodec
import uniffi.galaxy_ffi.ActivityItem as FActivityItem
import uniffi.galaxy_ffi.AgentEvent as FAgentEvent
import uniffi.galaxy_ffi.ContextRef as FContextRef
import uniffi.galaxy_ffi.ContextRefKind as FContextRefKind
import uniffi.galaxy_ffi.ConversationModel as FConversationModel
import uniffi.galaxy_ffi.AgentMessage as FAgentMessage
import uniffi.galaxy_ffi.AgentTarget as FAgentTarget
import uniffi.galaxy_ffi.AgentTargetKind as FAgentTargetKind
import uniffi.galaxy_ffi.AiMode as FAiMode
import uniffi.galaxy_ffi.AiModels as FAiModels
import uniffi.galaxy_ffi.ApprovalDecision as FApprovalDecision
import uniffi.galaxy_ffi.ClientState as FClientState
import uniffi.galaxy_ffi.ClipboardData as FClipboardData
import uniffi.galaxy_ffi.ComboKey as FComboKey
import uniffi.galaxy_ffi.ConversationConfig as FConversationConfig
import uniffi.galaxy_ffi.ConversationSummary as FConversationSummary
import uniffi.galaxy_ffi.CoreConfig as FCoreConfig
import uniffi.galaxy_ffi.DeviceInfo as FDeviceInfo
import uniffi.galaxy_ffi.DeviceKind as FDeviceKind
import uniffi.galaxy_ffi.DisplayInfo as FDisplayInfo
import uniffi.galaxy_ffi.DisplayOptions as FDisplayOptions
import uniffi.galaxy_ffi.EnvVar as FEnvVar
import uniffi.galaxy_ffi.ExecRequest as FExecRequest
import uniffi.galaxy_ffi.ExecResult as FExecResult
import uniffi.galaxy_ffi.FileEntry as FFileEntry
import uniffi.galaxy_ffi.FileKind as FFileKind
import uniffi.galaxy_ffi.GalaxyException as FGalaxyException
import uniffi.galaxy_ffi.ImageData as FImageData
import uniffi.galaxy_ffi.InputEvent as FInputEvent
import uniffi.galaxy_ffi.LoginResult as FLoginResult
import uniffi.galaxy_ffi.PathInfo as FPathInfo
import uniffi.galaxy_ffi.PathKind as FPathKind
import uniffi.galaxy_ffi.PeerInfo as FPeerInfo
import uniffi.galaxy_ffi.Permissions as FPermissions
import uniffi.galaxy_ffi.Platform as FPlatform
import uniffi.galaxy_ffi.PointerButton as FPointerButton
import uniffi.galaxy_ffi.RdpTarget as FRdpTarget
import uniffi.galaxy_ffi.RecordingInfo as FRecordingInfo
import uniffi.galaxy_ffi.UpdateInfo as FUpdateInfo
import uniffi.galaxy_ffi.RiskLevel as FRiskLevel
import uniffi.galaxy_ffi.ServerInfo as FServerInfo
import uniffi.galaxy_ffi.SshAuth as FSshAuth
import uniffi.galaxy_ffi.SshInfo as FSshInfo
import uniffi.galaxy_ffi.TerminalOptions as FTerminalOptions
import uniffi.galaxy_ffi.TerminalSessionInfo as FTerminalSessionInfo
import uniffi.galaxy_ffi.ToolCallView as FToolCallView
import uniffi.galaxy_ffi.ToolResultView as FToolResultView
import uniffi.galaxy_ffi.TotpSetup as FTotpSetup
import uniffi.galaxy_ffi.TouchPhase as FTouchPhase
import uniffi.galaxy_ffi.UserInfo as FUserInfo
import uniffi.galaxy_ffi.VaultData as FVaultData
import uniffi.galaxy_ffi.VaultEntry as FVaultEntry
import uniffi.galaxy_ffi.VideoCodec as FVideoCodec

/*
 * uniffi.galaxy_ffi（生成代码）与 core.bridge（界面认的类型）之间的转换。
 * 两边字段一一对应；枚举按名字对应，新增成员时两边都要加（编译器会在 when 上报错提醒）。
 */

// ---------- 错误 ----------

/** 内核错误字段叫 detail（避开 Kotlin 异常自带的 message），文字已按 locale 本地化。 */
internal fun FGalaxyException.toBridge(): GalaxyException = when (this) {
    is FGalaxyException.Network -> GalaxyException.Network(detail)
    is FGalaxyException.Auth -> GalaxyException.Auth(code, detail)
    is FGalaxyException.Permission -> GalaxyException.Permission(detail)
    is FGalaxyException.NotFound -> GalaxyException.NotFound(detail)
    is FGalaxyException.Protocol -> GalaxyException.Protocol(detail)
    is FGalaxyException.Cancelled -> GalaxyException.Cancelled()
    is FGalaxyException.Other -> GalaxyException.Other(detail)
}

/** 把内核调用里抛出的 uniffi 错误换成界面认的 GalaxyException。 */
internal inline fun <T> ffi(block: () -> T): T = try {
    block()
} catch (e: FGalaxyException) {
    throw e.toBridge()
}

// ---------- 枚举 ----------

internal fun Platform.toFfi() = FPlatform.valueOf(name)
internal fun FPlatform.toBridge() = Platform.valueOf(name)
internal fun DeviceKind.toFfi() = FDeviceKind.valueOf(name)
internal fun FDeviceKind.toBridge() = DeviceKind.valueOf(name)
internal fun FClientState.toBridge() = ClientState.valueOf(name)
internal fun FLoginResult.toBridge() = LoginResult.valueOf(name)
internal fun FPathKind.toBridge() = PathKind.valueOf(name)
internal fun FFileKind.toBridge() = FileKind.valueOf(name)
internal fun FVideoCodec.toBridge() = VideoCodec.valueOf(name)
internal fun VideoCodec.toFfi() = FVideoCodec.valueOf(name)
internal fun AiMode.toFfi() = FAiMode.valueOf(name)
internal fun FAiMode.toBridge() = AiMode.valueOf(name)
internal fun FRiskLevel.toBridge() = RiskLevel.valueOf(name)
internal fun AgentTargetKind.toFfi() = FAgentTargetKind.valueOf(name)
internal fun PointerButton.toFfi() = FPointerButton.valueOf(name)
internal fun TouchPhase.toFfi() = FTouchPhase.valueOf(name)
internal fun ComboKey.toFfi() = FComboKey.valueOf(name)

// ---------- 记录 ----------

internal fun CoreConfig.toFfi() = FCoreConfig(dataDir, cacheDir, logLevel, platform.toFfi(), deviceKind.toFfi(), appVersion, osVersion, hostname, locale)
internal fun FUserInfo.toBridge() = UserInfo(id, username, displayName, aiEnabled, totpEnabled)
internal fun FServerInfo.toBridge() = ServerInfo(address, name, version)
internal fun FTotpSetup.toBridge() = TotpSetup(secret, otpauthUri)
internal fun FDeviceInfo.toBridge() = DeviceInfo(id, kind.toBridge(), platform.toBridge(), hostname, osVersion, arch, appVersion, online, lanSeen, lastSeenAt, isSelf, updateVersion)
internal fun FPathInfo.toBridge() = PathInfo(kind.toBridge(), rttMs, remoteAddr)
internal fun FPermissions.toBridge() = Permissions(view, control, files, terminal, clipboard, audio, aiCap.toBridge())
internal fun FPeerInfo.toBridge() = PeerInfo(deviceId, hostname, platform.toBridge(), osVersion, arch, systemUser, shell, permissions.toBridge(), capabilities, path.toBridge())
internal fun FSshInfo.toBridge() = SshInfo(hostId, label, host, port, username, via)
internal fun FRdpTarget.toBridge() = RdpTarget(label, host, port, username, domain, password, forwarded, forwardId)
internal fun FUpdateInfo.toBridge() = UpdateInfo(version, fileName, size, sha256, url, notes)
internal fun UpdateInfo.toFfi() = FUpdateInfo(version, fileName, size, sha256, url, notes)
internal fun TerminalOptions.toFfi() = FTerminalOptions(attachSession, shell, cols, rows, env.map { FEnvVar(it.name, it.value) })
internal fun FTerminalSessionInfo.toBridge() = TerminalSessionInfo(id, title, createdAt, attached)
internal fun FFileEntry.toBridge() = FileEntry(name, path, kind.toBridge(), size, mode, modifiedAt, owner, group, linkTarget)
internal fun FDisplayInfo.toBridge() = DisplayInfo(id, name, width, height, scale, primary, isVirtual)
internal fun FRecordingInfo.toBridge() = RecordingInfo(paths, durationMs, bytes)
internal fun DisplayOptions.toFfi() = FDisplayOptions(displayId, maxFps, bitrateKbps, codecs.map { it.toFfi() }, viewOnly)
internal fun ExecRequest.toFfi() = FExecRequest(command, shell, cwd, timeoutMs, maxOutput)
internal fun FExecResult.toBridge() = ExecResult(code, stdout, stderr, timedOut, truncated, durationMs)
internal fun FImageData.toBridge() = ImageData(mime, data, width, height)
internal fun ImageData.toFfi() = FImageData(mime, data, width, height)
internal fun EnvVar.toFfi() = FEnvVar(name, value)

internal fun FClipboardData.toBridge(): ClipboardData = when (this) {
    is FClipboardData.Text -> ClipboardData.Text(text)
    is FClipboardData.Image -> ClipboardData.Image(image.toBridge())
    is FClipboardData.Files -> ClipboardData.Files(paths)
}

internal fun ClipboardData.toFfi(): FClipboardData = when (this) {
    is ClipboardData.Text -> FClipboardData.Text(text)
    is ClipboardData.Image -> FClipboardData.Image(image.toFfi())
    is ClipboardData.Files -> FClipboardData.Files(paths)
}

internal fun InputEvent.toFfi(): FInputEvent = when (this) {
    is InputEvent.MouseMove -> FInputEvent.MouseMove(x, y)
    is InputEvent.MouseButton -> FInputEvent.MouseButton(button.toFfi(), down, x, y)
    is InputEvent.Wheel -> FInputEvent.Wheel(dx, dy)
    is InputEvent.Key -> FInputEvent.Key(hidUsage, down)
    is InputEvent.Text -> FInputEvent.Text(text)
    is InputEvent.Touch -> FInputEvent.Touch(id, phase.toFfi(), x, y)
    is InputEvent.SpecialCombo -> FInputEvent.SpecialCombo(combo.toFfi())
}

// ---------- 加密库 ----------

internal fun FSshAuth.toBridge(): SshAuth = when (this) {
    is FSshAuth.Password -> SshAuth.Password(password)
    is FSshAuth.PrivateKey -> SshAuth.PrivateKey(credentialId)
    FSshAuth.Agent -> SshAuth.Agent
    FSshAuth.Interactive -> SshAuth.Interactive
}

internal fun SshAuth.toFfi(): FSshAuth = when (this) {
    is SshAuth.Password -> FSshAuth.Password(password)
    is SshAuth.PrivateKey -> FSshAuth.PrivateKey(credentialId)
    SshAuth.Agent -> FSshAuth.Agent
    SshAuth.Interactive -> FSshAuth.Interactive
}

internal fun FVaultData.toBridge(): VaultData = when (this) {
    is FVaultData.Folder -> VaultData.Folder(name, color, expanded)
    is FVaultData.DeviceMeta -> VaultData.DeviceMeta(deviceId, alias, tags, note, favorite)
    is FVaultData.SshHost -> VaultData.SshHost(label, host, port, username, auth.toBridge(), jumpDeviceId, jumpHostId, tags, note, favorite, terminalTheme)
    is FVaultData.RdpHost -> VaultData.RdpHost(label, host, port, username, password, domain, jumpDeviceId, tags, note, favorite)
    is FVaultData.Credential -> VaultData.Credential(label, username, password, privateKey, passphrase)
    is FVaultData.Snippet -> VaultData.Snippet(label, command, tags)
    is FVaultData.KnownHost -> VaultData.KnownHost(host, port, keyType, fingerprint)
    is FVaultData.Preferences -> VaultData.Preferences(json)
    is FVaultData.PortForward -> VaultData.PortForward(label, viaDeviceId, viaSshHostId, localPort, remoteHost, remotePort, autoStart, note)
}

internal fun VaultData.toFfi(): FVaultData = when (this) {
    is VaultData.Folder -> FVaultData.Folder(name, color, expanded)
    is VaultData.DeviceMeta -> FVaultData.DeviceMeta(deviceId, alias, tags, note, favorite)
    is VaultData.SshHost -> FVaultData.SshHost(label, host, port, username, auth.toFfi(), jumpDeviceId, jumpHostId, tags, note, favorite, terminalTheme)
    is VaultData.RdpHost -> FVaultData.RdpHost(label, host, port, username, password, domain, jumpDeviceId, tags, note, favorite)
    is VaultData.Credential -> FVaultData.Credential(label, username, password, privateKey, passphrase)
    is VaultData.Snippet -> FVaultData.Snippet(label, command, tags)
    is VaultData.KnownHost -> FVaultData.KnownHost(host, port, keyType, fingerprint)
    is VaultData.Preferences -> FVaultData.Preferences(json)
    is VaultData.PortForward -> FVaultData.PortForward(label, viaDeviceId, viaSshHostId, localPort, remoteHost, remotePort, autoStart, note)
}

internal fun FVaultEntry.toBridge() = VaultEntry(id, folderId, sort, data.toBridge(), updatedAt)
internal fun VaultEntry.toFfi() = FVaultEntry(id, folderId, sort, data.toFfi(), updatedAt)

// ---------- AI ----------

internal fun FAiModels.toBridge() = AiModels(enabled, maxMode.toBridge(), items.map { AiModel(it.providerId, it.providerName, it.id, it.name, it.vision, it.tools, it.contextLength) })
internal fun AgentTarget.toFfi() = FAgentTarget(kind.toFfi(), id)
internal fun ConversationConfig.toFfi() = FConversationConfig(providerId, modelId, targets.map { it.toFfi() }, mode.toFfi(), planFirst, trustMinutes, title)
internal fun FAgentTarget.toBridge() = AgentTarget(AgentTargetKind.valueOf(kind.name), id)
internal fun FConversationConfig.toBridge() = ConversationConfig(providerId, modelId, targets.map { it.toBridge() }, mode.toBridge(), planFirst, trustMinutes, title)
internal fun FConversationSummary.toBridge() = ConversationSummary(id, title, modelId, targetNames, updatedAt, config?.toBridge())
internal fun FConversationModel.toBridge() = ConversationModel(providerId, modelId)

internal fun ContextRefKind.toFfi() = FContextRefKind.valueOf(name)
internal fun FContextRefKind.toBridge() = ContextRefKind.valueOf(name)
internal fun ContextRef.toFfi() = FContextRef(kind.toFfi(), targetId, id, label)
internal fun FContextRef.toBridge() = ContextRef(kind.toBridge(), targetId, id, label)
internal fun FActivityItem.toBridge() = ActivityItem(at, action, actorName, targetType, targetId, targetName, ip, detail)

internal fun ApprovalDecision.toFfi(): FApprovalDecision = when (this) {
    ApprovalDecision.AllowOnce -> FApprovalDecision.AllowOnce
    ApprovalDecision.AllowSimilar -> FApprovalDecision.AllowSimilar
    is ApprovalDecision.Deny -> FApprovalDecision.Deny(reason)
    is ApprovalDecision.EditAndRun -> FApprovalDecision.EditAndRun(command)
}

internal fun FToolCallView.toBridge() = ToolCallView(callId, tool, targetId, targetName, summary, detail, risk.toBridge(), riskReasons, modelReason, image?.toBridge())
internal fun FToolResultView.toBridge() = ToolResultView(ok, summary, output, image?.toBridge(), denied)

internal fun FAgentEvent.toBridge(): AgentEvent = when (this) {
    is FAgentEvent.AssistantDelta -> AgentEvent.AssistantDelta(messageId, text)
    is FAgentEvent.AssistantDone -> AgentEvent.AssistantDone(messageId)
    is FAgentEvent.Thinking -> AgentEvent.Thinking(messageId, text)
    is FAgentEvent.ToolProposed -> AgentEvent.ToolProposed(call.toBridge(), needsApproval)
    is FAgentEvent.ToolStarted -> AgentEvent.ToolStarted(callId)
    is FAgentEvent.ToolOutput -> AgentEvent.ToolOutput(callId, text)
    is FAgentEvent.ToolFinished -> AgentEvent.ToolFinished(callId, result.toBridge())
    is FAgentEvent.AskUser -> AgentEvent.AskUser(questionId, question, options)
    is FAgentEvent.SecretRequested -> AgentEvent.SecretRequested(requestId, prompt, targetId)
    is FAgentEvent.Usage -> AgentEvent.Usage(promptTokens, completionTokens)
    is FAgentEvent.Status -> AgentEvent.Status(running, step)
    is FAgentEvent.ModeChanged -> AgentEvent.ModeChanged(mode.toBridge(), effective.toBridge(), untilMs)
    is FAgentEvent.Error -> AgentEvent.Error(message)
}

internal fun FAgentMessage.toBridge(): AgentMessage = when (this) {
    is FAgentMessage.User -> AgentMessage.User(text, refs.map { it.toBridge() })
    is FAgentMessage.Assistant -> AgentMessage.Assistant(id, text, thinking)
    is FAgentMessage.Tool -> AgentMessage.Tool(call.toBridge(), result?.toBridge())
}
