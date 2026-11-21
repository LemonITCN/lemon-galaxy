# 在本机安装（或更新）LemonGalaxy 虚拟屏驱动，供开发测试；也负责卸载。需要管理员权限。
#   install.ps1 [-Configuration Release] [-Platform x64|ARM64]   安装 / 更新
#   install.ps1 -Remove [-RemoveSigningCert]                     卸载（不需要 EWDK）
# 安装：把 LemonGalaxy 测试证书放进本机受信任的根和受信任的发布者（UMDF 驱动这样就能加载，不用开测试模式），
# 用 EWDK 的 devcon 建根设备 Root\LemonGalaxyDisplay 并装驱动，删掉驱动库里旧版本的包。
# 只碰自己的硬件 ID、INF 名和证书，不影响 LemonLink Display、GameViewer 等其它虚拟屏驱动。
param(
    [string]$Configuration = 'Release',
    [string]$Platform = 'x64',
    [switch]$Remove,
    [switch]$RemoveSigningCert
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$hardwareId = 'Root\LemonGalaxyDisplay'
$infName = 'lemongalaxydisplay.inf'
$subject = 'CN=LemonGalaxy Test Driver Signing'

function Get-OurDevices {
    Get-PnpDevice -ErrorAction SilentlyContinue | Where-Object { $_.HardwareID -contains $hardwareId }
}

function Get-OurPackages {
    Get-WindowsDriver -Online | Where-Object { $_.OriginalFileName -like "*\$infName" }
}

function Find-EwdkTool([string]$name) {
    $iso = if ($env:LG_EWDK_ISO) { $env:LG_EWDK_ISO } else { 'D:\dev\EWDK_28000.iso' }
    $image = Get-DiskImage -ImagePath $iso
    if (-not $image.Attached) { $image = Mount-DiskImage -ImagePath $iso -PassThru }
    $drive = ($image | Get-Volume).DriveLetter
    $arch = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'arm64' } else { 'x64' }
    $tool = Get-ChildItem "${drive}:\Program Files\Windows Kits\10\Tools" -Recurse -Filter $name |
        Where-Object { $_.Directory.Name -eq $arch } | Select-Object -First 1
    if (-not $tool) { throw "$name not found in the EWDK" }
    $tool.FullName
}

if ($Remove) {
    foreach ($device in Get-OurDevices) {
        pnputil /remove-device $device.InstanceId | Out-Host
    }
    foreach ($package in Get-OurPackages) {
        pnputil /delete-driver $package.Driver /uninstall | Out-Host
    }
    foreach ($storeName in 'Root', 'TrustedPublisher') {
        Get-ChildItem "Cert:\LocalMachine\$storeName" | Where-Object Subject -eq $subject | ForEach-Object {
            Write-Host "removing $($_.Thumbprint) from LocalMachine\$storeName"
            Remove-Item $_.PSPath
        }
    }
    if ($RemoveSigningCert) {
        Get-ChildItem Cert:\LocalMachine\My | Where-Object Subject -eq $subject | ForEach-Object {
            Write-Host "removing signing certificate $($_.Thumbprint) from LocalMachine\My"
            Remove-Item $_.PSPath -DeleteKey
        }
    }
    return
}

$package = Join-Path $root "build\$Platform\$Configuration\LemonGalaxyDisplay"
$inf = Join-Path $package 'LemonGalaxyDisplay.inf'
$cer = Join-Path $package 'LemonGalaxyTest.cer'
if (-not (Test-Path $inf) -or -not (Test-Path $cer)) { throw "build and sign first: $package" }

# 信任测试证书
$certificate = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2($cer)
foreach ($storeName in 'Root', 'TrustedPublisher') {
    $store = New-Object System.Security.Cryptography.X509Certificates.X509Store($storeName, 'LocalMachine')
    $store.Open('ReadWrite')
    if (-not ($store.Certificates | Where-Object Thumbprint -eq $certificate.Thumbprint)) { $store.Add($certificate) }
    $store.Close()
}

# 驱动宿主（LOCAL SERVICE）写日志的目录
$logs = Join-Path $env:ProgramData 'LemonGalaxy\Logs'
New-Item -ItemType Directory -Force $logs | Out-Null
icacls $logs /grant '*S-1-5-19:(OI)(CI)M' /q | Out-Null

$devcon = Find-EwdkTool 'devcon.exe'
if (Get-OurDevices) {
    & $devcon update $inf $hardwareId
} else {
    & $devcon install $inf $hardwareId
}
if ($LASTEXITCODE -gt 1) { throw "devcon failed: $LASTEXITCODE" }

# 驱动库里只留当前在用的那份
$device = Get-OurDevices | Select-Object -First 1
$inUse = (Get-PnpDeviceProperty -InstanceId $device.InstanceId -KeyName DEVPKEY_Device_DriverInfPath).Data
foreach ($old in Get-OurPackages | Where-Object { $_.Driver -ne $inUse }) {
    pnputil /delete-driver $old.Driver /force | Out-Null
    Write-Host "deleted old package $($old.Driver) ($($old.Version))"
}
Get-OurDevices | Format-Table Status, Class, FriendlyName, InstanceId -AutoSize | Out-String -Width 200
Get-OurPackages | Format-Table Driver, Version, Date -AutoSize | Out-String -Width 200
