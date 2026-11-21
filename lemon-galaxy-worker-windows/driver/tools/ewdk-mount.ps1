# Prints the drive letter of the mounted EWDK image, mounting it first when needed. An image whose
# drive letter does not answer (mounted from a logon session that has ended) is mounted again.
param([string]$Image = 'D:\dev\EWDK_28000.iso')
$ErrorActionPreference = 'Stop'
for ($attempt = 1; $attempt -le 3; $attempt++) {
    try {
        $disk = Get-DiskImage -ImagePath $Image
        if ($disk.Attached) {
            $letter = ($disk | Get-Volume).DriveLetter
            if ($letter -and (Test-Path "${letter}:\BuildEnv\SetupBuildEnv.cmd")) { Write-Output $letter; exit 0 }
            Dismount-DiskImage -ImagePath $Image | Out-Null
        }
        $letter = (Mount-DiskImage -ImagePath $Image -PassThru | Get-Volume).DriveLetter
        if ($letter -and (Test-Path "${letter}:\BuildEnv\SetupBuildEnv.cmd")) { Write-Output $letter; exit 0 }
    } catch {
        [Console]::Error.WriteLine("EWDK mount attempt ${attempt}: $($_.Exception.Message)")
    }
    Start-Sleep -Seconds 2
}
exit 1
