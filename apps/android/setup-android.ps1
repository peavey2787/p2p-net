param(
    [string]$SdkRoot = "",
    [string]$AvdName = "P2PNet_API_37",
    [switch]$NoEmulator,
    [switch]$NoVsBuildTools
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
$AndroidRoot = $PSScriptRoot
$RepoRoot = (Resolve-Path (Join-Path $AndroidRoot "..\..")).Path
$ToolRoot = Join-Path $env:LOCALAPPDATA "p2p-net\android-tools"
$GradleVersion = "9.5.0"
$CargoNdkVersion = "4.1.2"
$RustVersion = "1.98.0"
$NdkVersion = "28.2.13676358"
$CmakeVersion = "3.22.1"
$CommandLineToolsFile = "commandlinetools-win-15859902_latest.zip"
$CommandLineToolsSha256 = "90ae805d20434428bffcb699c290860f19bb5f66a67e6b330067e3de801fb04a"
$GradleSha256 = "553c78f50dafcd54d65b9a444649057857469edf836431389695608536d6b746"
$HypervisorRebootRequired = $false

function Require-ExitCode([string]$What) {
    if ($LASTEXITCODE -ne 0) { throw "$What failed with exit code $LASTEXITCODE" }
}

function Invoke-NativeCapture([string]$FilePath, [string]$Arguments) {
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $FilePath
    $startInfo.Arguments = $Arguments
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true

    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw "Failed to start native command: $FilePath $Arguments" }
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

function Refresh-ProcessPath {
    $machinePath = [Environment]::GetEnvironmentVariable("Path", "Machine")
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $parts = @($machinePath, $userPath, $env:Path) | Where-Object { $_ }
    $env:Path = ($parts -join ";")
}

function Add-UserPath([string]$PathToAdd) {
    if (-not $PathToAdd -or -not (Test-Path $PathToAdd -PathType Container)) { return }
    $normalized = [IO.Path]::GetFullPath($PathToAdd).TrimEnd('\')
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $entries = @()
    if ($userPath) { $entries = $userPath -split ';' | Where-Object { $_ } }
    if (-not ($entries | Where-Object { $_.TrimEnd('\') -ieq $normalized })) {
        [Environment]::SetEnvironmentVariable("Path", (($entries + $normalized) -join ";"), "User")
    }
    if (-not (($env:Path -split ';') | Where-Object { $_.TrimEnd('\') -ieq $normalized })) {
        $env:Path = "$normalized;$env:Path"
    }
}

function Require-Winget {
    if (-not (Get-Command winget.exe -ErrorAction SilentlyContinue)) {
        throw "Windows Package Manager (winget) is required to install missing JDK/Visual Studio components. Install or update Microsoft App Installer, then rerun this script."
    }
}

function Install-WingetPackage([string]$Id, [string[]]$ExtraArgs = @()) {
    Require-Winget
    $args = @(
        "install", "--id", $Id, "--exact", "--source", "winget",
        "--accept-package-agreements", "--accept-source-agreements", "--silent"
    ) + $ExtraArgs
    & winget.exe @args
    Require-ExitCode "winget install $Id"
    Refresh-ProcessPath
}

function Get-Java17Home {
    $java = Get-Command java.exe -ErrorAction SilentlyContinue
    if ($java) {
        $probe = Invoke-NativeCapture $java.Source "-version"
        $line = ($probe.Output -split "`r?`n" | Select-Object -First 1).Trim()
        if ($probe.ExitCode -eq 0 -and $line -match 'version "17(?:[\._]|\")') {
            return (Split-Path (Split-Path $java.Source -Parent) -Parent)
        }
    }
    $roots = @(
        "${env:ProgramFiles}\Microsoft\jdk-17*",
        "${env:ProgramFiles}\Eclipse Adoptium\jdk-17*",
        "${env:ProgramFiles}\Java\jdk-17*"
    )
    foreach ($pattern in $roots) {
        $candidate = Get-Item $pattern -ErrorAction SilentlyContinue | Sort-Object Name -Descending | Select-Object -First 1
        if ($candidate -and (Test-Path (Join-Path $candidate.FullName "bin\java.exe") -PathType Leaf)) {
            return $candidate.FullName
        }
    }
    return $null
}

function Find-VsDevCmd {
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path $vswhere -PathType Leaf) {
        $install = (& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath | Select-Object -First 1 | Out-String).Trim()
        if ($install) {
            $cmd = Join-Path $install "Common7\Tools\VsDevCmd.bat"
            if (Test-Path $cmd -PathType Leaf) { return $cmd }
        }
    }
    foreach ($year in @("18", "2022")) {
        foreach ($edition in @("BuildTools", "Community", "Professional", "Enterprise")) {
            foreach ($base in @($env:ProgramFiles, ${env:ProgramFiles(x86)})) {
                if (-not $base) { continue }
                $install = Join-Path $base "Microsoft Visual Studio\$year\$edition"
                $cmd = Join-Path $install "Common7\Tools\VsDevCmd.bat"
                $linker = Get-Item (Join-Path $install "VC\Tools\MSVC\*\bin\Hostx64\x64\link.exe") -ErrorAction SilentlyContinue | Select-Object -First 1
                if ((Test-Path $cmd -PathType Leaf) -and $linker) { return $cmd }
            }
        }
    }
    return $null
}

function Download-Verified([string]$Url, [string]$Destination, [string]$ExpectedSha256) {
    if (Test-Path $Destination -PathType Leaf) {
        $existing = (Get-FileHash $Destination -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($existing -eq $ExpectedSha256.ToLowerInvariant()) { return }
        Remove-Item $Destination -Force
    }
    Write-Host "Downloading $Url"
    Invoke-WebRequest -Uri $Url -OutFile $Destination -UseBasicParsing
    $actual = (Get-FileHash $Destination -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $ExpectedSha256.ToLowerInvariant()) {
        Remove-Item $Destination -Force -ErrorAction SilentlyContinue
        throw "SHA-256 mismatch for $Url. Expected $ExpectedSha256, got $actual"
    }
}

function Ensure-EmulatorAcceleration([string]$EmulatorExe) {
    $probe = Invoke-NativeCapture $EmulatorExe "-accel-check"
    $output = $probe.Output
    if ($probe.ExitCode -eq 0) {
        Write-Host "Android Emulator hardware acceleration is ready."
        return
    }

    $feature = Get-CimInstance -ClassName Win32_OptionalFeature -Filter "Name='HypervisorPlatform'" -ErrorAction SilentlyContinue
    if ($feature -and $feature.InstallState -ne 1) {
        Write-Host "Windows Hypervisor Platform is disabled; enabling it requires one UAC elevation."
        $process = Start-Process -FilePath dism.exe -ArgumentList @(
            "/Online", "/Enable-Feature", "/FeatureName:HypervisorPlatform", "/All", "/NoRestart"
        ) -Verb RunAs -Wait -PassThru
        if ($process.ExitCode -ne 0 -and $process.ExitCode -ne 3010) {
            throw "Windows Hypervisor Platform installation failed with DISM exit code $($process.ExitCode). Emulator output before installation:`n$output"
        }
        $script:HypervisorRebootRequired = $true
        return
    }

    throw "Android Emulator hardware acceleration is unavailable. Windows Hypervisor Platform appears enabled (or its state could not be read), so verify CPU virtualization is enabled in UEFI/BIOS. Emulator output:`n$output"
}

function Normalize-FullPath([string]$PathValue) {
    if (-not $PathValue) { return $null }
    try { return [IO.Path]::GetFullPath($PathValue).TrimEnd('\') } catch { return $null }
}

function Test-SamePath([string]$Left, [string]$Right) {
    $leftPath = Normalize-FullPath $Left
    $rightPath = Normalize-FullPath $Right
    return $leftPath -and $rightPath -and $leftPath.Equals($rightPath, [StringComparison]::OrdinalIgnoreCase)
}

function Get-KnownBrokenSdkRoot {
    return (Normalize-FullPath (Join-Path $RepoRoot "-AvdName"))
}

function Repair-KnownBrokenSdkRootState {
    $brokenRoot = Get-KnownBrokenSdkRoot
    $repaired = $false

    foreach ($name in @("ANDROID_SDK_ROOT", "ANDROID_HOME")) {
        $processValue = [Environment]::GetEnvironmentVariable($name, "Process")
        $userValue = [Environment]::GetEnvironmentVariable($name, "User")
        if ((Test-SamePath $processValue $brokenRoot) -or (Test-SamePath $userValue $brokenRoot)) {
            Write-Warning "$name points at the invalid SDK path created by an older p2p-net launcher: $brokenRoot. Clearing it."
            if (Test-SamePath $processValue $brokenRoot) {
                Remove-Item "Env:$name" -ErrorAction SilentlyContinue
            }
            if (Test-SamePath $userValue $brokenRoot) {
                [Environment]::SetEnvironmentVariable($name, $null, "User")
            }
            $repaired = $true
        }
    }

    $brokenPrefix = "$brokenRoot\"
    $processNdk = Normalize-FullPath ([Environment]::GetEnvironmentVariable("ANDROID_NDK_HOME", "Process"))
    $userNdk = Normalize-FullPath ([Environment]::GetEnvironmentVariable("ANDROID_NDK_HOME", "User"))
    $processNdkBroken = $processNdk -and $processNdk.StartsWith($brokenPrefix, [StringComparison]::OrdinalIgnoreCase)
    $userNdkBroken = $userNdk -and $userNdk.StartsWith($brokenPrefix, [StringComparison]::OrdinalIgnoreCase)
    if ($processNdkBroken -or $userNdkBroken) {
        Write-Warning "ANDROID_NDK_HOME points inside the invalid SDK path. Clearing it."
        if ($processNdkBroken) { Remove-Item "Env:ANDROID_NDK_HOME" -ErrorAction SilentlyContinue }
        if ($userNdkBroken) { [Environment]::SetEnvironmentVariable("ANDROID_NDK_HOME", $null, "User") }
        $repaired = $true
    }

    $localProperties = Join-Path $AndroidRoot "local.properties"
    if (Test-Path $localProperties -PathType Leaf) {
        $line = Get-Content $localProperties | Where-Object { $_ -like "sdk.dir=*" } | Select-Object -First 1
        if ($line) {
            $value = $line.Substring("sdk.dir=".Length).Replace("\:", ":").Replace("\\", "\")
            if (Test-SamePath $value $brokenRoot) {
                Write-Warning "Removing local.properties entry for the invalid SDK path: $brokenRoot"
                Remove-Item $localProperties -Force
                $repaired = $true
            }
        }
    }

    # The bad SDK root was created only by the old array-splatting bug. Remove it
    # when it clearly contains Android SDK material so it cannot be selected again.
    if ((Test-Path $brokenRoot -PathType Container) -and (
        (Test-Path (Join-Path $brokenRoot "cmake") -PathType Container) -or
        (Test-Path (Join-Path $brokenRoot "platforms") -PathType Container) -or
        (Test-Path (Join-Path $brokenRoot "cmdline-tools") -PathType Container)
    )) {
        Write-Warning "Removing invalid Android SDK tree created under the repository: $brokenRoot"
        Remove-Item $brokenRoot -Recurse -Force
        $repaired = $true
    }

    if ($repaired) {
        $cxxRoot = Join-Path $AndroidRoot "app\.cxx"
        if (Test-Path $cxxRoot -PathType Container) {
            Write-Host "Removing stale CMake state that referenced the invalid SDK path."
            Remove-Item $cxxRoot -Recurse -Force
        }
    }
}

function Resolve-SdkRoot([string]$Requested) {
    $brokenRoot = Get-KnownBrokenSdkRoot
    if ($Requested) {
        $requestedPath = Normalize-FullPath $Requested
        if (Test-SamePath $requestedPath $brokenRoot) {
            throw "Refusing invalid Android SDK root '$requestedPath'. Rerun without -SdkRoot so setup can use the real SDK."
        }
        return $requestedPath
    }

    foreach ($candidate in @($env:ANDROID_SDK_ROOT, $env:ANDROID_HOME)) {
        if ($candidate -and -not (Test-SamePath $candidate $brokenRoot)) {
            return (Normalize-FullPath $candidate)
        }
    }

    $localProperties = Join-Path $AndroidRoot "local.properties"
    if (Test-Path $localProperties -PathType Leaf) {
        $line = Get-Content $localProperties | Where-Object { $_ -like "sdk.dir=*" } | Select-Object -First 1
        if ($line) {
            $value = $line.Substring("sdk.dir=".Length).Replace("\:", ":").Replace("\\", "\")
            if ($value -and -not (Test-SamePath $value $brokenRoot)) {
                return (Normalize-FullPath $value)
            }
        }
    }
    return (Join-Path $env:LOCALAPPDATA "Android\Sdk")
}

function Install-AndroidPackages(
    [string]$SdkManager,
    [string]$SdkRootPath,
    [string[]]$Packages
) {
    if ($Packages.Count -eq 0) { return }

    Write-Host "Missing Android SDK packages: $($Packages -join ', ')"
    Write-Host "Accepting Android SDK licenses if required."
    Write-Host "Using sdkmanager channel 3 for Android 37.0 preview packages. Its deprecation warning is expected."
    $licenseInput = 1..64 | ForEach-Object { "y" }
    $licenseInput | & $SdkManager "--sdk_root=$SdkRootPath" --licenses | Out-Host
    Require-ExitCode "Android SDK license acceptance"

    & $SdkManager "--sdk_root=$SdkRootPath" --channel=3 @Packages
    Require-ExitCode "Android SDK package installation"
}

Write-Host "p2p-net Android setup"
Write-Host "Repository: $RepoRoot"
Write-Host "Checks are idempotent: components already at the required version are left alone."
New-Item -ItemType Directory -Force -Path $ToolRoot | Out-Null
Refresh-ProcessPath

Write-Host ""
Write-Host "==> Windows C++ build tools"
if (-not $NoVsBuildTools) {
    $vsDevCmd = Find-VsDevCmd
    if (-not $vsDevCmd) {
        Write-Host "Visual Studio C++ Build Tools not found; installing the VCTools workload."
        Install-WingetPackage "Microsoft.VisualStudio.2022.BuildTools" @(
            "--force",
            "--override", "--wait --quiet --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
        )
        $vsDevCmd = Find-VsDevCmd
        if (-not $vsDevCmd) { throw "Visual Studio C++ Build Tools installation completed but VsDevCmd.bat was not found." }
    }
    Write-Host "Visual Studio developer tools: $vsDevCmd"
} else {
    Write-Host "Skipped by -NoVsBuildTools."
}
if (-not $NoVsBuildTools) {
    $ucrt = Get-Item "${env:ProgramFiles(x86)}\Windows Kits\10\Lib\*\ucrt\x64\ucrt.lib" -ErrorAction SilentlyContinue | Sort-Object FullName -Descending | Select-Object -First 1
    if (-not $ucrt) {
        throw "The Visual Studio C++ workload is present but the Windows Universal CRT library was not found. Rerun setup after repairing Visual Studio Build Tools with its recommended components."
    }
    Write-Host "Universal CRT: $($ucrt.FullName)"
}

Write-Host ""
Write-Host "==> Rust $RustVersion"
if (-not (Get-Command rustup.exe -ErrorAction SilentlyContinue)) {
    Write-Host "rustup not found; installing Rustup."
    Install-WingetPackage "Rustlang.Rustup"
}
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
Add-UserPath (Join-Path $env:USERPROFILE ".cargo\bin")
$toolchains = @(& rustup.exe toolchain list | ForEach-Object { ($_ -split '\s+')[0] })
if (-not ($toolchains | Where-Object { $_ -like "$RustVersion-*" -or $_ -eq $RustVersion })) {
    & rustup.exe toolchain install $RustVersion --profile minimal
    Require-ExitCode "rustup toolchain install $RustVersion"
} else {
    Write-Host "Rust toolchain $RustVersion already installed."
}
$components = @(& rustup.exe component list --toolchain $RustVersion --installed)
if (-not ($components | Where-Object { $_ -like "rustfmt-*" -or $_ -eq "rustfmt" })) {
    & rustup.exe component add rustfmt --toolchain $RustVersion
    Require-ExitCode "rustfmt component install"
}
$installedTargets = @(& rustup.exe target list --installed --toolchain $RustVersion)
$missingTargets = @("aarch64-linux-android", "x86_64-linux-android") | Where-Object { $installedTargets -notcontains $_ }
if ($missingTargets.Count -gt 0) {
    & rustup.exe target add @missingTargets --toolchain $RustVersion
    Require-ExitCode "Rust Android targets"
} else {
    Write-Host "Rust Android targets already installed."
}
$rustLine = (& rustc.exe "+$RustVersion" --version | Out-String).Trim()
if (-not $rustLine.StartsWith("rustc $RustVersion ")) { throw "Expected rustc $RustVersion, got: $rustLine" }
Write-Host "$rustLine"

Write-Host ""
Write-Host "==> JDK 17"
$javaHome = Get-Java17Home
if (-not $javaHome) {
    Write-Host "JDK 17 not found; installing Microsoft OpenJDK 17."
    Install-WingetPackage "Microsoft.OpenJDK.17"
    $javaHome = Get-Java17Home
    if (-not $javaHome) { throw "JDK 17 installation completed but a Java 17 home could not be located." }
}
$env:JAVA_HOME = $javaHome
[Environment]::SetEnvironmentVariable("JAVA_HOME", $javaHome, "User")
Add-UserPath (Join-Path $javaHome "bin")
$javaProbe = Invoke-NativeCapture (Join-Path $javaHome "bin\java.exe") "-version"
if ($javaProbe.ExitCode -ne 0) { throw "java -version failed with exit code $($javaProbe.ExitCode): $($javaProbe.Output)" }
$javaLine = ($javaProbe.Output -split "`r?`n" | Select-Object -First 1).Trim()
Write-Host "$javaLine"

Write-Host ""
Write-Host "==> Gradle $GradleVersion"
$gradleHome = Join-Path $ToolRoot "gradle-$GradleVersion"
$gradleExe = Join-Path $gradleHome "bin\gradle.bat"
$gradleOk = $false
$existingGradle = Get-Command gradle.bat -ErrorAction SilentlyContinue
if (-not $existingGradle) { $existingGradle = Get-Command gradle.exe -ErrorAction SilentlyContinue }
if ($existingGradle) {
    $gradleOutput = (& $existingGradle.Source --version | Out-String)
    $gradleOk = $LASTEXITCODE -eq 0 -and $gradleOutput -match "Gradle 9\.5\.0"
}
if (-not $gradleOk -and -not (Test-Path $gradleExe -PathType Leaf)) {
    $gradleZip = Join-Path $ToolRoot "gradle-$GradleVersion-bin.zip"
    Download-Verified "https://services.gradle.org/distributions/gradle-$GradleVersion-bin.zip" $gradleZip $GradleSha256
    $extractRoot = Join-Path $ToolRoot "gradle-extract"
    Remove-Item $extractRoot -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force -Path $extractRoot | Out-Null
    Expand-Archive -Path $gradleZip -DestinationPath $extractRoot -Force
    if (Test-Path $gradleHome) { Remove-Item $gradleHome -Recurse -Force }
    Move-Item (Join-Path $extractRoot "gradle-$GradleVersion") $gradleHome
    Remove-Item $extractRoot -Recurse -Force
}
if (-not $gradleOk) {
    $env:GRADLE_HOME = $gradleHome
    [Environment]::SetEnvironmentVariable("GRADLE_HOME", $gradleHome, "User")
    Add-UserPath (Join-Path $gradleHome "bin")
    $env:Path = "$(Join-Path $gradleHome 'bin');$env:Path"
    $gradleCommand = $gradleExe
} else {
    $gradleCommand = $existingGradle.Source
}
$gradleOutput = (& $gradleCommand --version | Out-String)
Require-ExitCode "Gradle version check"
if ($gradleOutput -notmatch "Gradle 9\.5\.0") { throw "Gradle $GradleVersion is required." }
Write-Host "Gradle $GradleVersion ready: $gradleCommand"

Write-Host ""
Write-Host "==> cargo-ndk $CargoNdkVersion"
$cargoNdkProbe = Invoke-NativeCapture "cargo.exe" "+$RustVersion ndk --version"
$cargoNdkOutput = $cargoNdkProbe.Output
if ($cargoNdkProbe.ExitCode -ne 0 -or -not $cargoNdkOutput.Contains($CargoNdkVersion)) {
    & cargo.exe "+$RustVersion" install cargo-ndk --version $CargoNdkVersion --locked --force
    Require-ExitCode "cargo-ndk install"
    $cargoNdkProbe = Invoke-NativeCapture "cargo.exe" "+$RustVersion ndk --version"
    $cargoNdkOutput = $cargoNdkProbe.Output
}
if (-not $cargoNdkOutput.Contains($CargoNdkVersion)) { throw "cargo-ndk $CargoNdkVersion is required, got: $cargoNdkOutput" }
Write-Host "$cargoNdkOutput"

Write-Host ""
Write-Host "==> Android SDK"
Repair-KnownBrokenSdkRootState
$SdkRoot = Resolve-SdkRoot $SdkRoot
New-Item -ItemType Directory -Force -Path $SdkRoot | Out-Null
$env:ANDROID_SDK_ROOT = $SdkRoot
$env:ANDROID_HOME = $SdkRoot
$env:ANDROID_NDK_HOME = Join-Path $SdkRoot "ndk\$NdkVersion"
[Environment]::SetEnvironmentVariable("ANDROID_SDK_ROOT", $SdkRoot, "User")
[Environment]::SetEnvironmentVariable("ANDROID_HOME", $SdkRoot, "User")
[Environment]::SetEnvironmentVariable("ANDROID_NDK_HOME", $env:ANDROID_NDK_HOME, "User")

$sdkManager = Join-Path $SdkRoot "cmdline-tools\latest\bin\sdkmanager.bat"
$avdManager = Join-Path $SdkRoot "cmdline-tools\latest\bin\avdmanager.bat"
if (-not (Test-Path $sdkManager -PathType Leaf)) {
    $toolsZip = Join-Path $ToolRoot $CommandLineToolsFile
    Download-Verified "https://dl.google.com/android/repository/$CommandLineToolsFile" $toolsZip $CommandLineToolsSha256
    $extractRoot = Join-Path $ToolRoot "cmdline-tools-extract"
    Remove-Item $extractRoot -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force -Path $extractRoot | Out-Null
    Expand-Archive -Path $toolsZip -DestinationPath $extractRoot -Force
    $latest = Join-Path $SdkRoot "cmdline-tools\latest"
    Remove-Item $latest -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force -Path $latest | Out-Null
    Copy-Item (Join-Path $extractRoot "cmdline-tools\*") $latest -Recurse -Force
    Remove-Item $extractRoot -Recurse -Force
}
if (-not (Test-Path $sdkManager -PathType Leaf)) { throw "sdkmanager.bat was not installed at $sdkManager" }

Add-UserPath (Join-Path $SdkRoot "platform-tools")
Add-UserPath (Join-Path $SdkRoot "emulator")
Add-UserPath (Join-Path $SdkRoot "cmdline-tools\latest\bin")

$sdkPackages = @()
if (-not (Test-Path (Join-Path $SdkRoot "platform-tools\adb.exe") -PathType Leaf)) {
    $sdkPackages += "platform-tools"
}
if (-not (Test-Path (Join-Path $SdkRoot "platforms\android-37.0\android.jar") -PathType Leaf)) {
    $sdkPackages += "platforms;android-37.0"
}
if (-not (Test-Path (Join-Path $SdkRoot "build-tools\37.0.0\aapt2.exe") -PathType Leaf)) {
    $sdkPackages += "build-tools;37.0.0"
}
if (-not (Test-Path (Join-Path $SdkRoot "ndk\$NdkVersion") -PathType Container)) {
    $sdkPackages += "ndk;$NdkVersion"
}
if (-not (Test-Path (Join-Path $SdkRoot "cmake\$CmakeVersion\bin\cmake.exe") -PathType Leaf)) {
    $sdkPackages += "cmake;$CmakeVersion"
}

$currentSystemImageDir = Join-Path $SdkRoot "system-images\android-37.0\google_apis_ps16k\x86_64"
$legacySystemImageDir = Join-Path $SdkRoot "system-images\android-37\google_apis\x86_64"
$systemImagePackage = $null
if (Test-Path (Join-Path $currentSystemImageDir "system.img") -PathType Leaf) {
    $systemImagePackage = "system-images;android-37.0;google_apis_ps16k;x86_64"
} elseif (Test-Path (Join-Path $legacySystemImageDir "system.img") -PathType Leaf) {
    $systemImagePackage = "system-images;android-37;google_apis;x86_64"
}

if (-not $NoEmulator) {
    if (-not (Test-Path (Join-Path $SdkRoot "emulator\emulator.exe") -PathType Leaf)) {
        $sdkPackages += "emulator"
    }
    if (-not $systemImagePackage) {
        $sdkPackages += "system-images;android-37.0;google_apis_ps16k;x86_64"
        $systemImagePackage = "system-images;android-37.0;google_apis_ps16k;x86_64"
    }
}

if ($sdkPackages.Count -gt 0) {
    Install-AndroidPackages $sdkManager $SdkRoot $sdkPackages
} else {
    Write-Host "Required Android SDK packages are already installed."
}

# Verify the required minor-versioned platform was actually materialized.
$platformJar = Join-Path $SdkRoot "platforms\android-37.0\android.jar"
if (-not (Test-Path $platformJar -PathType Leaf)) {
    Write-Warning "Android SDK Platform 37.0 is still missing; retrying platforms;android-37.0 with sdkmanager channel 3."
    & $sdkManager "--sdk_root=$SdkRoot" --channel=3 "platforms;android-37.0"
    Require-ExitCode "Android SDK Platform 37.0 installation"
}

foreach ($required in @(
    (Join-Path $SdkRoot "platforms\android-37.0\android.jar"),
    (Join-Path $SdkRoot "platform-tools\adb.exe"),
    (Join-Path $SdkRoot "ndk\$NdkVersion"),
    (Join-Path $SdkRoot "cmake\$CmakeVersion\bin\cmake.exe")
)) {
    if (-not (Test-Path $required)) { throw "Required Android component is missing after installation: $required" }
}

if (-not $NoEmulator) {
    $emulatorExe = Join-Path $SdkRoot "emulator\emulator.exe"
    if (-not (Test-Path $emulatorExe -PathType Leaf)) { throw "emulator.exe is missing: $emulatorExe" }
    Ensure-EmulatorAcceleration $emulatorExe

    if (-not (Test-Path $avdManager -PathType Leaf)) { throw "avdmanager.bat is missing: $avdManager" }
    if (-not $systemImagePackage) {
        if (Test-Path (Join-Path $currentSystemImageDir "system.img") -PathType Leaf) {
            $systemImagePackage = "system-images;android-37.0;google_apis_ps16k;x86_64"
        } elseif (Test-Path (Join-Path $legacySystemImageDir "system.img") -PathType Leaf) {
            $systemImagePackage = "system-images;android-37;google_apis;x86_64"
        } else {
            throw "Android 37 emulator system image is missing after installation."
        }
    }
    $avdList = (& $avdManager list avd | Out-String)
    if ($avdList -notmatch "(?m)^\s*Name:\s*$([regex]::Escape($AvdName))\s*$") {
        Write-Host "Creating Android 37 AVD: $AvdName"
        Write-Host "System image: $systemImagePackage"
        "no" | & $avdManager create avd --force --name $AvdName --package $systemImagePackage
        Require-ExitCode "AVD creation"
    } else {
        Write-Host "AVD already exists: $AvdName"
    }
}

$escapedSdk = $SdkRoot.Replace("\", "\\").Replace(":", "\:")
Set-Content -Path (Join-Path $AndroidRoot "local.properties") -Value "sdk.dir=$escapedSdk" -Encoding ASCII

Write-Host ""
Write-Host "==> Locked dependency preflight"
Push-Location $RepoRoot
try {
    $metadataProbe = Invoke-NativeCapture "cargo.exe" "+$RustVersion metadata --locked --format-version 1"
    if ($metadataProbe.ExitCode -ne 0) {
        $details = if ($metadataProbe.Output) { "`n$($metadataProbe.Output)" } else { "" }
        throw "cargo metadata --locked failed with exit code $($metadataProbe.ExitCode)$details"
    }
} finally {
    Pop-Location
}

if ($HypervisorRebootRequired) {
    throw "Windows Hypervisor Platform was enabled successfully. Restart Windows once, then rerun apps\android\setup-android.cmd (or apps\android\run-android.cmd) to finish the acceleration check."
}

Write-Host ""
Write-Host "Android build/run prerequisites are ready."
Write-Host "SDK: $SdkRoot"
Write-Host "JDK: $javaHome"
Write-Host "Gradle: $GradleVersion"
if (-not $NoEmulator) { Write-Host "AVD: $AvdName" }
Write-Host "Next: .\apps\android\run-android.cmd"
