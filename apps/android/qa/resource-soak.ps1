param(
    [ValidateRange(1, 10080)][int]$DurationMinutes = 60,
    [ValidateRange(5, 3600)][int]$SampleSeconds = 30,
    [string]$Package = "io.github.peavey2787.p2pnet"
)

$ErrorActionPreference = "Stop"
$EvidenceRoot = Join-Path $PSScriptRoot "evidence"
$Stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$OutDir = Join-Path $EvidenceRoot "resource-$Stamp"
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$Csv = Join-Path $OutDir "samples.csv"
"timestamp,pid,total_pss_kb,total_rss_kb,threads,fds,cpu_percent" | Set-Content $Csv

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

if ((Adb get-state) -ne "device") { throw "adb device is not ready" }
$deadline = (Get-Date).AddMinutes($DurationMinutes)
$sample = 0

while ((Get-Date) -lt $deadline) {
    $pid = (Adb shell pidof $Package).Trim()
    if (-not $pid) { throw "$Package is not running at sample $sample" }
    $pid = ($pid -split '\s+')[0]

    $mem = Adb shell dumpsys meminfo $Package
    $status = Adb shell cat "/proc/$pid/status"
    $cpu = Adb shell dumpsys cpuinfo
    $fdText = (Adb shell "ls /proc/$pid/fd 2>/dev/null | wc -l").Trim()

    $pss = if ($mem -match 'TOTAL PSS:\s+(\d+)') { $Matches[1] } else { "" }
    $rss = if ($mem -match 'TOTAL RSS:\s+(\d+)') { $Matches[1] } else { "" }
    $threads = if ($status -match '(?m)^Threads:\s+(\d+)') { $Matches[1] } else { "" }
    $fds = if ($fdText -match '^\d+$') { $fdText } else { "" }
    $cpuPercent = ""
    foreach ($line in ($cpu -split "`r?`n")) {
        if ($line -match ('^\s*([0-9.]+)%.*' + [regex]::Escape($Package))) {
            $cpuPercent = $Matches[1]
            break
        }
    }

    "$(Get-Date -Format o),$pid,$pss,$rss,$threads,$fds,$cpuPercent" | Add-Content $Csv
    $mem | Set-Content (Join-Path $OutDir ("meminfo-{0:D5}.txt" -f $sample))
    $sample++
    Start-Sleep -Seconds $SampleSeconds
}

$finalPid = (Adb shell pidof $Package).Trim()
if (-not $finalPid) { throw "$Package exited before the soak completed" }
Write-Host "Resource soak completed: $sample samples"
Write-Host "Evidence: $OutDir"
