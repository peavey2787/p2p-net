param(
    [string]$Device = "",
    [string]$AvdName = "P2PNet_API_37",
    [switch]$NoSetup,
    [switch]$NoEmulator,
    [switch]$ColdBoot
)

$ErrorActionPreference = "Stop"
$AndroidRoot = $PSScriptRoot
$RepoRoot = (Resolve-Path (Join-Path $AndroidRoot "..\..")).Path
$Package = "io.github.peavey2787.p2pnet"
$Component = "$Package/.MainActivity"

function Require-ExitCode([string]$What) {
    if ($LASTEXITCODE -ne 0) { throw "$What failed with exit code $LASTEXITCODE" }
}

function Quote-NativeArgument([string]$Value) {
    if ($null -eq $Value -or $Value.Length -eq 0) { return '""' }
    if ($Value -notmatch '[\s"]') { return $Value }
    return '"' + $Value.Replace('"', '\"') + '"'
}

function Invoke-NativeCapture([string]$FilePath, [string[]]$ArgumentList = @()) {
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $FilePath
    $startInfo.Arguments = (($ArgumentList | ForEach-Object { Quote-NativeArgument $_ }) -join ' ')
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true

    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw "Failed to start native command: $FilePath $($ArgumentList -join ' ')" }
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    $output = ($stdout + $stderr).Trim()
    $exitCode = $process.ExitCode
    $process.Dispose()

    return [PSCustomObject]@{
        ExitCode = $exitCode
        Output = $output
    }
}

function Normalize-FullPath([string]$PathValue) {
    if (-not $PathValue) { return $null }
    try { return [IO.Path]::GetFullPath($PathValue).TrimEnd('\') } catch { return $null }
}

function Test-KnownBrokenSdkRoot([string]$PathValue) {
    $candidate = Normalize-FullPath $PathValue
    $brokenRoot = Normalize-FullPath (Join-Path $RepoRoot "-AvdName")
    return $candidate -and $brokenRoot -and $candidate.Equals($brokenRoot, [StringComparison]::OrdinalIgnoreCase)
}

function Resolve-AndroidSdkRoot {
    foreach ($candidate in @($env:ANDROID_SDK_ROOT, $env:ANDROID_HOME)) {
        if ($candidate -and -not (Test-KnownBrokenSdkRoot $candidate)) {
            return (Normalize-FullPath $candidate)
        }
    }
    $localProperties = Join-Path $AndroidRoot "local.properties"
    if (Test-Path $localProperties -PathType Leaf) {
        $line = Get-Content $localProperties | Where-Object { $_ -like "sdk.dir=*" } | Select-Object -First 1
        if ($line) {
            $value = $line.Substring("sdk.dir=".Length).Replace("\:", ":").Replace("\\", "\")
            if ($value -and -not (Test-KnownBrokenSdkRoot $value)) {
                return (Normalize-FullPath $value)
            }
        }
    }
    $defaultSdk = Normalize-FullPath (Join-Path $env:LOCALAPPDATA "Android\Sdk")
    if (Test-Path $defaultSdk -PathType Container) { return $defaultSdk }
    throw "Android SDK path is not configured. Run apps\android\setup-android.cmd first."
}

function Require-CapturedSuccess($Result, [string]$What) {
    if ($Result.ExitCode -ne 0) {
        $details = if ($Result.Output) { "`n$($Result.Output)" } else { "" }
        throw "$What failed with exit code $($Result.ExitCode)$details"
    }
}

function Invoke-AdbCapture([string]$Adb, [string[]]$Arguments = @()) {
    return (Invoke-NativeCapture $Adb $Arguments)
}

function Get-ReadyDevices([string]$Adb) {
    $probe = Invoke-AdbCapture $Adb @("devices")
    Require-CapturedSuccess $probe "adb devices"
    $devices = @()
    foreach ($row in ($probe.Output -split "`r?`n")) {
        if ($row -match '^([^\s]+)\s+device$') { $devices += $Matches[1] }
    }
    return @($devices)
}

function Get-AdbDevicesText([string]$Adb) {
    $probe = Invoke-AdbCapture $Adb @("devices")
    Require-CapturedSuccess $probe "adb devices"
    return $probe.Output
}

function Wait-ForBoot([string]$Adb, [string]$Serial, [int]$TimeoutSeconds = 240) {
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        $bootProbe = Invoke-AdbCapture $Adb @("-s", $Serial, "shell", "getprop", "sys.boot_completed")
        if ($bootProbe.ExitCode -eq 0 -and $bootProbe.Output.Trim() -eq "1") {
            $packageProbe = Invoke-AdbCapture $Adb @("-s", $Serial, "shell", "cmd", "package", "list", "packages")
            if ($packageProbe.ExitCode -eq 0) { return }
        }
        Start-Sleep -Seconds 1
    }
    throw "Android device $Serial did not finish booting before the timeout."
}

if (-not $NoSetup) {
    Write-Host "==> Verify/install Android prerequisites"
    $setupParameters = @{ AvdName = $AvdName }
    if ($NoEmulator) { $setupParameters["NoEmulator"] = $true }
    & (Join-Path $AndroidRoot "setup-android.ps1") @setupParameters
    Require-ExitCode "Android prerequisite setup"
}

$SdkRoot = Resolve-AndroidSdkRoot
$env:ANDROID_SDK_ROOT = $SdkRoot
$env:ANDROID_HOME = $SdkRoot
$env:ANDROID_NDK_HOME = Join-Path $SdkRoot "ndk\28.2.13676358"
$adb = Join-Path $SdkRoot "platform-tools\adb.exe"
$emulator = Join-Path $SdkRoot "emulator\emulator.exe"
if (-not (Test-Path $adb -PathType Leaf)) { throw "adb.exe is missing. Run apps\android\setup-android.cmd first." }

Write-Host ""
Write-Host "==> Build Android debug APK"
& (Join-Path $AndroidRoot "build-android.ps1") -Variant Debug
Require-ExitCode "Android debug build"
$apk = Join-Path $AndroidRoot "app\build\outputs\apk\debug\app-debug.apk"
if (-not (Test-Path $apk -PathType Leaf)) { throw "Debug APK was not produced: $apk" }

Write-Host ""
Write-Host "==> Select Android target"
$startServer = Invoke-AdbCapture $adb @("start-server")
Require-CapturedSuccess $startServer "adb start-server"
$ready = @(Get-ReadyDevices $adb)
$serial = $null

if ($Device) {
    if ($ready -notcontains $Device) {
        $all = Get-AdbDevicesText $adb
        throw "Requested device '$Device' is not ready. adb devices:`n$all"
    }
    $serial = $Device
} elseif ($ready.Count -gt 0) {
    $physical = @($ready | Where-Object { $_ -notlike "emulator-*" })
    $serial = if ($physical.Count -gt 0) { $physical[0] } else { $ready[0] }
}

if (-not $serial) {
    if ($NoEmulator) {
        $all = Get-AdbDevicesText $adb
        throw "No ready Android device is connected and emulator startup was disabled. adb devices:`n$all"
    }
    if (-not (Test-Path $emulator -PathType Leaf)) { throw "Android Emulator is missing. Run apps\android\setup-android.cmd first." }
    $accelProbe = Invoke-NativeCapture $emulator "-accel-check"
    $accel = $accelProbe.Output
    if ($accelProbe.ExitCode -ne 0) {
        throw "Android Emulator hardware acceleration is unavailable. Enable CPU virtualization in firmware and Windows Hypervisor Platform (WHPX), reboot if Windows requests it, then rerun. Emulator output:`n$accel"
    }
    $avdProbe = Invoke-NativeCapture $emulator @("-list-avds")
    Require-CapturedSuccess $avdProbe "emulator -list-avds"
    $avds = @($avdProbe.Output -split "`r?`n" | Where-Object { $_ })
    if ($avds -notcontains $AvdName) {
        throw "AVD '$AvdName' does not exist. Run apps\android\setup-android.cmd first."
    }
    Write-Host "Starting emulator: $AvdName"
    $emulatorArgs = @("-avd", $AvdName, "-netdelay", "none", "-netspeed", "full")
    if ($ColdBoot) { $emulatorArgs += "-no-snapshot-load" }
    Start-Process -FilePath $emulator -ArgumentList $emulatorArgs | Out-Null

    $deadline = (Get-Date).AddSeconds(240)
    while ((Get-Date) -lt $deadline -and -not $serial) {
        Start-Sleep -Seconds 1
        $emulators = @(Get-ReadyDevices $adb | Where-Object { $_ -like "emulator-*" })
        if ($emulators.Count -gt 0) { $serial = $emulators[0] }
    }
    if (-not $serial) { throw "The Android emulator did not become visible to adb before the timeout." }
}

Wait-ForBoot $adb $serial
Write-Host "Using device: $serial"
Write-Host ""
Write-Host "==> Install APK"
$install = Invoke-AdbCapture $adb @("-s", $serial, "install", "-r", "-t", "-d", $apk)
if ($install.Output) { Write-Host $install.Output }
Require-CapturedSuccess $install "adb install"

Write-Host ""
Write-Host "==> Launch P2P Node"
$forceStop = Invoke-AdbCapture $adb @("-s", $serial, "shell", "am", "force-stop", $Package)
Require-CapturedSuccess $forceStop "Android app force-stop"
$launch = Invoke-AdbCapture $adb @("-s", $serial, "shell", "am", "start", "-W", "-n", $Component)
if ($launch.Output) { Write-Host $launch.Output }
Require-CapturedSuccess $launch "Android app launch"

Write-Host ""
Write-Host "P2P Node is running on $serial."
Write-Host "APK: $apk"
Write-Host "Logs: `"$adb`" -s $serial logcat"
