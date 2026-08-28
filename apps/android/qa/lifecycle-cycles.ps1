param(
    [ValidateRange(1, 10000)][int]$Cycles = 100,
    [ValidateRange(1, 60)][int]$SettleSeconds = 3,
    [string]$Package = "io.github.peavey2787.p2pnet"
)

$ErrorActionPreference = "Stop"
$Component = "$Package/.MainActivity"
$EvidenceRoot = Join-Path $PSScriptRoot "evidence"
$Stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$OutDir = Join-Path $EvidenceRoot "lifecycle-$Stamp"
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$Log = Join-Path $OutDir "cycles.csv"
"cycle,pid,total_pss_kb,total_rss_kb,threads,fds" | Set-Content $Log

function Quote-NativeArgument([string]$Value) {
    if ($null -eq $Value -or $Value.Length -eq 0) { return '""' }
    if ($Value -notmatch '[\s"]') { return $Value }
    return '"' + $Value.Replace('"', '\"') + '"'
}

function Adb([Parameter(ValueFromRemainingArguments = $true)][string[]]$Args) {
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = "adb.exe"
    $startInfo.Arguments = (($Args | ForEach-Object { Quote-NativeArgument $_ }) -join ' ')
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true

    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw "Failed to start adb $($Args -join ' ')" }
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    $exitCode = $process.ExitCode
    $output = ($stdout + $stderr).Trim()
    $process.Dispose()

    if ($exitCode -ne 0) { throw "adb $($Args -join ' ') failed with exit code ${exitCode}: $output" }
    return $output
}

for ($cycle = 1; $cycle -le $Cycles; $cycle++) {
    Adb shell am force-stop $Package | Out-Null
    Adb shell am start -W -n $Component | Out-Null
    Start-Sleep -Seconds $SettleSeconds
    $pid = (Adb shell pidof $Package).Trim()
    if (-not $pid) { throw "$Package failed to return at cycle $cycle" }
    $pid = ($pid -split '\s+')[0]
    $mem = Adb shell dumpsys meminfo $Package
    $status = Adb shell cat "/proc/$pid/status"
    $pss = if ($mem -match 'TOTAL PSS:\s+(\d+)') { $Matches[1] } else { "" }
    $rss = if ($mem -match 'TOTAL RSS:\s+(\d+)') { $Matches[1] } else { "" }
    $threads = if ($status -match '(?m)^Threads:\s+(\d+)') { $Matches[1] } else { "" }
    $fds = (Adb shell "ls /proc/$pid/fd 2>/dev/null | wc -l").Trim()
    "$cycle,$pid,$pss,$rss,$threads,$fds" | Add-Content $Log
}
Write-Host "Lifecycle cycles completed: $Cycles"
Write-Host "Evidence: $OutDir"
