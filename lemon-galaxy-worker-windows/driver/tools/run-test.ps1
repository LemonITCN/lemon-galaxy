# 在控制台会话里跑 LemonGalaxyDisplayTest（SSH 会话看不到桌面），等它结束后打印结果。
# 用一次性的计划任务：以当前登录控制台的用户、最高权限、仅交互方式运行，跑完删掉任务。
#   run-test.ps1 [-Configuration Release] [-Platform x64] [-Arguments '--info'] [-TimeoutSeconds 120]
param(
    [string]$Configuration = 'Release',
    [string]$Platform = 'x64',
    [string]$Arguments = '',
    [int]$TimeoutSeconds = 120
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$exe = Join-Path $root "build\$Platform\$Configuration\LemonGalaxyDisplayTest.exe"
if (-not (Test-Path $exe)) { throw "build first: $exe is missing" }
$out = Join-Path $root 'test-output.txt'
Remove-Item $out -ErrorAction SilentlyContinue

$user = (Get-CimInstance Win32_ComputerSystem).UserName
if (-not $user) { throw 'nobody is signed in at the console' }
if (Get-Process LogonUI -ErrorAction SilentlyContinue) { Write-Host 'warning: the console may be locked (LogonUI is running)' }

$task = 'LemonGalaxyDisplayTest'
$command = "`"$exe`" --out `"$out`" $Arguments".Trim()
schtasks /create /tn $task /tr $command /sc once /st 23:59 /ru $user /it /rl highest /f | Out-Null
try {
    schtasks /run /tn $task | Out-Null
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    Start-Sleep -Seconds 2
    while ((Get-Date) -lt $deadline) {
        $state = (Get-ScheduledTask -TaskName $task).State
        if ($state -ne 'Running') { break }
        Start-Sleep -Seconds 1
    }
    $info = Get-ScheduledTaskInfo -TaskName $task
    Write-Host "task state $((Get-ScheduledTask -TaskName $task).State), last result $($info.LastTaskResult)"
} finally {
    schtasks /delete /tn $task /f | Out-Null
}
if (Test-Path $out) { Get-Content $out -Encoding UTF8 } else { Write-Host 'no output file' }
