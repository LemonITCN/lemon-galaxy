# 用 LemonGalaxy 自己的测试证书签驱动包（只签 .cat，它覆盖包里的 .inf 和 .dll）。
# 证书放本机 LocalMachine\My（SSH 会话里也能用私钥），没有就生成一张；公钥导出成包里的
# LemonGalaxyTest.cer，install.ps1 把它放进受信任的根和受信任的发布者。正式发布改用 SignPath。
#   sign.ps1 [-Configuration Release] [-Platform x64|ARM64]
# 在 EWDK 环境里运行（build.cmd 会调用），要用到 signtool 和 inf2cat。
param(
    [string]$Configuration = 'Release',
    [string]$Platform = 'x64'
)
$ErrorActionPreference = 'Stop'
$subject = 'CN=LemonGalaxy Test Driver Signing'
$package = Join-Path (Split-Path $PSScriptRoot -Parent) "build\$Platform\$Configuration\LemonGalaxyDisplay"
$inf = Join-Path $package 'LemonGalaxyDisplay.inf'
if (-not (Test-Path $inf)) { throw "build first: $inf is missing" }

$cert = Get-ChildItem Cert:\LocalMachine\My | Where-Object { $_.Subject -eq $subject -and $_.HasPrivateKey -and $_.NotAfter -gt (Get-Date).AddDays(30) } |
    Sort-Object NotAfter -Descending | Select-Object -First 1
if (-not $cert) {
    $cert = New-SelfSignedCertificate -Type CodeSigningCert -Subject $subject -CertStoreLocation Cert:\LocalMachine\My `
        -KeyAlgorithm RSA -KeyLength 3072 -HashAlgorithm SHA256 -KeyExportPolicy NonExportable -NotAfter (Get-Date).AddYears(5)
    Write-Host "created test certificate $($cert.Thumbprint)"
}

$cat = Join-Path $package 'LemonGalaxyDisplay.cat'
if (-not (Test-Path $cat)) {
    $os = if ($Platform -eq 'ARM64') { '10_RS5_ARM64' } else { '10_RS5_X64' }
    & inf2cat /driver:$package /os:$os /uselocaltime | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "inf2cat failed: $LASTEXITCODE" }
}
& signtool sign /q /fd sha256 /sm /s My /sha1 $cert.Thumbprint $cat
if ($LASTEXITCODE -ne 0) { throw "signtool failed: $LASTEXITCODE" }
Export-Certificate -Cert $cert -FilePath (Join-Path $package 'LemonGalaxyTest.cer') -Type CERT | Out-Null
Write-Host "signed $cat with $($cert.Thumbprint)"
Get-ChildItem $package | Format-Table Length, LastWriteTime, Name -AutoSize
