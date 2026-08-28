param(
    [ValidateSet("Release", "Debug")][string]$Variant = "Release"
)

$ErrorActionPreference = "Stop"
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$AndroidRoot = $PSScriptRoot

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

function Import-VsDevEnvironment([string]$VsDevCmd) {
    $command = '"' + $VsDevCmd + '" -no_logo -arch=amd64 -host_arch=amd64 >nul && set'
    $lines = & cmd.exe /d /s /c $command
    if ($LASTEXITCODE -ne 0) { throw "Visual Studio developer environment initialization failed" }
    foreach ($line in $lines) {
        $index = $line.IndexOf('=')
        if ($index -le 0) { continue }
        $name = $line.Substring(0, $index)
        $value = $line.Substring($index + 1)
        Set-Item -Path "Env:$name" -Value $value
    }
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

function Resolve-AndroidSdkRoot {
    $brokenRoot = Get-KnownBrokenSdkRoot
    foreach ($candidate in @($env:ANDROID_SDK_ROOT, $env:ANDROID_HOME)) {
        if ($candidate -and -not (Test-SamePath $candidate $brokenRoot)) {
            return (Normalize-FullPath $candidate)
        }
    }

    $LocalProperties = Join-Path $AndroidRoot "local.properties"
    if (Test-Path $LocalProperties) {
        $Line = Get-Content $LocalProperties | Where-Object { $_ -like "sdk.dir=*" } | Select-Object -First 1
        if ($Line) {
            $value = $Line.Substring("sdk.dir=".Length).Replace("\:", ":").Replace("\\", "\")
            if ($value -and -not (Test-SamePath $value $brokenRoot)) {
                return (Normalize-FullPath $value)
            }
        }
    }

    $defaultSdk = Normalize-FullPath (Join-Path $env:LOCALAPPDATA "Android\Sdk")
    if (Test-Path $defaultSdk -PathType Container) { return $defaultSdk }
    throw "Set ANDROID_SDK_ROOT/ANDROID_HOME or create apps\\android\\local.properties with sdk.dir=..."
}

function Remove-PoisonedCmakeState {
    $cxxRoot = Join-Path $AndroidRoot "app\.cxx"
    if (-not (Test-Path $cxxRoot -PathType Container)) { return }

    $brokenRoot = Get-KnownBrokenSdkRoot
    $needles = @($brokenRoot, $brokenRoot.Replace('\', '/'))
    $stateFiles = Get-ChildItem $cxxRoot -Recurse -File -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -in @("CMakeCache.txt", "build.ninja") }
    foreach ($stateFile in $stateFiles) {
        $content = Get-Content $stateFile.FullName -Raw -ErrorAction SilentlyContinue
        if (-not $content) { continue }
        foreach ($needle in $needles) {
            if ($content.IndexOf($needle, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
                Write-Warning "Removing stale CMake state that references invalid SDK root: $brokenRoot"
                Remove-Item $cxxRoot -Recurse -Force
                return
            }
        }
    }
}

function Normalize-FutureBuildInputTimestamps {
    # ZIP timestamps do not carry a reliable timezone. An archive produced in UTC
    # and extracted by Windows in a western timezone can therefore make source
    # files appear several hours in the future. Ninja then considers CMake's
    # manifest permanently dirty and re-runs CMake until its 100-try guard fires.
    $now = Get-Date
    $futureCutoff = $now.AddSeconds(5)
    $safeTime = $now.AddSeconds(-2)
    $cxxRoot = Join-Path $AndroidRoot "app\.cxx"
    $cmakeSourceRoot = Join-Path $AndroidRoot "app\src\main\cpp"
    $futureCmakeInput = $false
    $normalized = 0

    $inputs = @()
    foreach ($sourceRoot in @(
        $cmakeSourceRoot,
        (Join-Path $AndroidRoot "app\src\main\java"),
        (Join-Path $AndroidRoot "native\src")
    )) {
        if (Test-Path $sourceRoot -PathType Container) {
            foreach ($file in (Get-ChildItem $sourceRoot -Recurse -File -ErrorAction Stop)) {
                $inputs += $file
            }
        }
    }

    foreach ($sourceFile in @(
        (Join-Path $RepoRoot "Cargo.toml"),
        (Join-Path $RepoRoot "Cargo.lock"),
        (Join-Path $RepoRoot "rust-toolchain.toml"),
        (Join-Path $AndroidRoot "build.gradle.kts"),
        (Join-Path $AndroidRoot "settings.gradle.kts"),
        (Join-Path $AndroidRoot "gradle.properties"),
        (Join-Path $AndroidRoot "app\build.gradle.kts"),
        (Join-Path $AndroidRoot "native\Cargo.toml"),
        (Join-Path $AndroidRoot "native\Cargo.lock")
    )) {
        if (Test-Path $sourceFile -PathType Leaf) {
            $inputs += (Get-Item $sourceFile)
        }
    }

    foreach ($file in $inputs) {
        if ($file.LastWriteTime -le $futureCutoff) { continue }
        if ($file.FullName.StartsWith($cmakeSourceRoot, [StringComparison]::OrdinalIgnoreCase)) {
            $futureCmakeInput = $true
        }
        $file.LastWriteTime = $safeTime
        $normalized++
    }

    if ($normalized -gt 0) {
        Write-Warning "Normalized $normalized future-dated Android build input(s) caused by archive timezone skew."
    }
    if ($futureCmakeInput -and (Test-Path $cxxRoot -PathType Container)) {
        Write-Warning "Removing CMake state after future-dated CMake input recovery."
        Remove-Item $cxxRoot -Recurse -Force
    }
}


$vsDevCmd = Find-VsDevCmd
if (-not $vsDevCmd) {
    throw "Visual Studio C++ Build Tools were not found. Run apps\android\setup-android.cmd first."
}
Import-VsDevEnvironment $vsDevCmd
if (-not (Get-Command link.exe -ErrorAction SilentlyContinue)) {
    throw "Visual Studio developer environment initialized but link.exe is unavailable. Run apps\android\setup-android.cmd to repair the C++ build tools."
}

$env:RUSTUP_TOOLCHAIN = "1.98.0"
$rustVersion = (& rustc +1.98.0 --version)
Require-ExitCode "rustc +1.98.0 --version"
if (-not $rustVersion.StartsWith("rustc 1.98.0 ")) {
    throw "Android builds require rustc 1.98.0, got: $rustVersion"
}

$cargoNdkProbe = Invoke-NativeCapture "cargo.exe" "+1.98.0 ndk --version"
if ($cargoNdkProbe.ExitCode -ne 0) {
    throw "cargo +1.98.0 ndk --version failed with exit code $($cargoNdkProbe.ExitCode): $($cargoNdkProbe.Output)"
}
$cargoNdkVersion = $cargoNdkProbe.Output
if (-not $cargoNdkVersion.Contains("4.1.2")) {
    throw "Android builds require cargo-ndk 4.1.2. Install with: cargo install cargo-ndk --version 4.1.2 --locked"
}

$javaCommand = "java"
if ($env:JAVA_HOME) {
    $javaCandidate = Join-Path $env:JAVA_HOME "bin\java.exe"
    if (Test-Path $javaCandidate -PathType Leaf) { $javaCommand = $javaCandidate }
}
$javaProbe = Invoke-NativeCapture $javaCommand "-version"
if ($javaProbe.ExitCode -ne 0) {
    throw "java -version failed with exit code $($javaProbe.ExitCode): $($javaProbe.Output)"
}
$javaVersion = ($javaProbe.Output -split "`r?`n" | Select-Object -First 1).Trim()
if ($javaVersion -notmatch 'version "17[\._"]') {
    throw "Android builds require JDK 17, got: $javaVersion"
}

$gradleCommand = "gradle"
if ($env:GRADLE_HOME) {
    $gradleCandidate = Join-Path $env:GRADLE_HOME "bin\gradle.bat"
    if (Test-Path $gradleCandidate -PathType Leaf) {
        $candidateProbe = Invoke-NativeCapture $gradleCandidate "--version"
        if ($candidateProbe.ExitCode -eq 0 -and $candidateProbe.Output -match "Gradle 9\.5\.0") {
            $gradleCommand = $gradleCandidate
        }
    }
}
$gradleProbe = Invoke-NativeCapture $gradleCommand "--version"
if ($gradleProbe.ExitCode -ne 0) {
    throw "gradle --version failed with exit code $($gradleProbe.ExitCode): $($gradleProbe.Output)"
}
if ($gradleProbe.Output -notmatch "Gradle 9\.5\.0") {
    throw "Android builds require Gradle 9.5.0"
}

$SdkRoot = Resolve-AndroidSdkRoot
Normalize-FutureBuildInputTimestamps
Remove-PoisonedCmakeState
if (-not (Test-Path (Join-Path $SdkRoot "platforms\android-37.0\android.jar") -PathType Leaf)) {
    throw "Android SDK platform 37.0 is missing under $SdkRoot"
}
if (-not (Test-Path (Join-Path $SdkRoot "ndk\28.2.13676358") -PathType Container)) {
    throw "Android NDK 28.2.13676358 is missing under $SdkRoot"
}
if (-not (Test-Path (Join-Path $SdkRoot "cmake\3.22.1\bin\cmake.exe") -PathType Leaf)) {
    throw "Android SDK CMake 3.22.1 is missing under $SdkRoot"
}

# cargo-ndk does not consume Gradle local.properties. Export the paths we just
# resolved so Cargo and AGP use the same pinned SDK/NDK installation.
$env:ANDROID_SDK_ROOT = $SdkRoot
$env:ANDROID_HOME = $SdkRoot
$env:ANDROID_NDK_HOME = Join-Path $SdkRoot "ndk\28.2.13676358"

$installedTargets = (& rustup target list --installed --toolchain 1.98.0)
Require-ExitCode "rustup target list"
if ($installedTargets -notcontains "aarch64-linux-android") {
    throw "Missing Rust target aarch64-linux-android. Run: rustup target add aarch64-linux-android --toolchain 1.98.0"
}
if ($installedTargets -notcontains "x86_64-linux-android") {
    throw "Missing Rust target x86_64-linux-android. Run: rustup target add x86_64-linux-android --toolchain 1.98.0"
}

Push-Location $RepoRoot
try {
    $metadataProbe = Invoke-NativeCapture "cargo.exe" "+1.98.0 metadata --locked --format-version 1"
    if ($metadataProbe.ExitCode -ne 0) {
        $details = if ($metadataProbe.Output) { "`n$($metadataProbe.Output)" } else { "" }
        throw "cargo metadata --locked failed with exit code $($metadataProbe.ExitCode)$details"
    }
} finally {
    Pop-Location
}

function Test-FilesByteIdentical([string]$Left, [string]$Right) {
    $leftInfo = Get-Item $Left -ErrorAction Stop
    $rightInfo = Get-Item $Right -ErrorAction Stop
    if ($leftInfo.Length -ne $rightInfo.Length) { return $false }

    $leftStream = [IO.File]::OpenRead($Left)
    $rightStream = [IO.File]::OpenRead($Right)
    try {
        $leftBuffer = New-Object byte[] (1024 * 1024)
        $rightBuffer = New-Object byte[] (1024 * 1024)
        while ($true) {
            $leftRead = $leftStream.Read($leftBuffer, 0, $leftBuffer.Length)
            $rightRead = $rightStream.Read($rightBuffer, 0, $rightBuffer.Length)
            if ($leftRead -ne $rightRead) { return $false }
            if ($leftRead -eq 0) { return $true }
            for ($i = 0; $i -lt $leftRead; $i++) {
                if ($leftBuffer[$i] -ne $rightBuffer[$i]) { return $false }
            }
        }
    } finally {
        $leftStream.Dispose()
        $rightStream.Dispose()
    }
}

function Remove-AndroidBuildState {
    foreach ($path in @(
        (Join-Path $AndroidRoot "app\build"),
        (Join-Path $AndroidRoot "app\.cxx"),
        (Join-Path $AndroidRoot "app\src\main\jniLibs")
    )) {
        if (Test-Path $path) { Remove-Item $path -Recurse -Force }
    }
}

function Invoke-OneReleaseBuild([string]$Label, [string]$EvidenceRoot) {
    $buildRoot = Join-Path $EvidenceRoot $Label
    $cargoTarget = Join-Path $buildRoot "cargo-target"
    $artifactRoot = Join-Path $buildRoot "artifacts"
    New-Item -ItemType Directory -Force -Path $cargoTarget, $artifactRoot | Out-Null

    Remove-AndroidBuildState
    $env:CARGO_TARGET_DIR = $cargoTarget
    $env:CARGO_INCREMENTAL = "0"

    Write-Host ""
    Write-Host "==> Android reproducibility build $Label"
    Push-Location $AndroidRoot
    try {
        $savedErrorActionPreference = $ErrorActionPreference
        try {
            # Gradle and the JVM can emit normal diagnostics on stderr. Under
            # Windows PowerShell 5.1, Stop would promote those lines to a
            # NativeCommandError before we can inspect the real process code.
            $ErrorActionPreference = "Continue"
            & $gradleCommand --no-daemon --no-build-cache --no-configuration-cache --rerun-tasks --stacktrace clean :app:assembleRelease :app:bundleRelease | Out-Host
            $gradleExit = $LASTEXITCODE
        } finally {
            $ErrorActionPreference = $savedErrorActionPreference
        }
        if ($gradleExit -ne 0) {
            throw "Android release build $Label failed with exit code $gradleExit"
        }
    } finally {
        Pop-Location
    }

    $apk = Join-Path $AndroidRoot "app\build\outputs\apk\release\app-release-unsigned.apk"
    $aab = Join-Path $AndroidRoot "app\build\outputs\bundle\release\app-release.aab"
    foreach ($artifact in @($apk, $aab)) {
        if (-not (Test-Path $artifact -PathType Leaf)) {
            throw "Android release build $Label did not produce expected artifact: $artifact"
        }
    }

    $savedApk = Join-Path $artifactRoot "app-release-unsigned.apk"
    $savedAab = Join-Path $artifactRoot "app-release.aab"
    Copy-Item $apk $savedApk -Force
    Copy-Item $aab $savedAab -Force
    return [PSCustomObject]@{
        Apk = $savedApk
        Aab = $savedAab
        ApkHash = (Get-FileHash $savedApk -Algorithm SHA256).Hash.ToLowerInvariant()
        AabHash = (Get-FileHash $savedAab -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}

if ($Variant -eq "Release") {
    $epochProbe = Invoke-NativeCapture "git.exe" "-C `"$RepoRoot`" log -1 --format=%ct"
    $sourceEpoch = if ($epochProbe.ExitCode -eq 0 -and $epochProbe.Output -match '^\d+$') {
        $epochProbe.Output.Trim()
    } else {
        "1577836800"
    }
    $env:SOURCE_DATE_EPOCH = $sourceEpoch

    $evidenceRoot = Join-Path $env:TEMP ("p2p-net-android-repro-" + [Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Force -Path $evidenceRoot | Out-Null
    $originalCargoTarget = $env:CARGO_TARGET_DIR
    try {
        $buildA = Invoke-OneReleaseBuild "build-a" $evidenceRoot
        $buildB = Invoke-OneReleaseBuild "build-b" $evidenceRoot

        Write-Host ""
        Write-Host "==> Verify Android byte-for-byte reproducibility"
        Write-Host "APK build A SHA-256: $($buildA.ApkHash)"
        Write-Host "APK build B SHA-256: $($buildB.ApkHash)"
        Write-Host "AAB build A SHA-256: $($buildA.AabHash)"
        Write-Host "AAB build B SHA-256: $($buildB.AabHash)"

        $apkByteIdentical = Test-FilesByteIdentical $buildA.Apk $buildB.Apk
        $aabByteIdentical = Test-FilesByteIdentical $buildA.Aab $buildB.Aab
        Write-Host "APK direct byte comparison: $apkByteIdentical"
        Write-Host "AAB direct byte comparison: $aabByteIdentical"

        if ($buildA.ApkHash -ne $buildB.ApkHash -or $buildA.AabHash -ne $buildB.AabHash -or -not $apkByteIdentical -or -not $aabByteIdentical) {
            $failureDir = Join-Path $RepoRoot "dist\android\reproducibility-failure"
            New-Item -ItemType Directory -Force -Path $failureDir | Out-Null
            Copy-Item $buildA.Apk (Join-Path $failureDir "build-a.apk") -Force
            Copy-Item $buildB.Apk (Join-Path $failureDir "build-b.apk") -Force
            Copy-Item $buildA.Aab (Join-Path $failureDir "build-a.aab") -Force
            Copy-Item $buildB.Aab (Join-Path $failureDir "build-b.aab") -Force
            throw "Android release is not reproducible: clean build hashes or direct byte comparisons differ. Evidence retained at $failureDir"
        }

        $distDir = Join-Path $RepoRoot "dist\android"
        New-Item -ItemType Directory -Force -Path $distDir | Out-Null
        $distApk = Join-Path $distDir "p2p-net-android-0.1.0-release-unsigned.apk"
        $distAab = Join-Path $distDir "p2p-net-android-0.1.0-release.aab"
        Copy-Item $buildB.Apk $distApk -Force
        Copy-Item $buildB.Aab $distAab -Force

        @(
            "$($buildB.ApkHash)  $(Split-Path $distApk -Leaf)",
            "$($buildB.AabHash)  $(Split-Path $distAab -Leaf)"
        ) | Set-Content -Path (Join-Path $distDir "SHA256SUMS.txt") -Encoding ascii
        @(
            "p2p-net Android reproducible release",
            "version=0.1.0",
            "rust=1.98.0",
            "cargo_ndk=4.1.2",
            "gradle=9.5.0",
            "jdk=17",
            "android_sdk=37.0",
            "ndk=28.2.13676358",
            "cmake=3.22.1",
            "abis=arm64-v8a,x86_64",
            "source_date_epoch=$sourceEpoch",
            "build_a_apk_sha256=$($buildA.ApkHash)",
            "build_b_apk_sha256=$($buildB.ApkHash)",
            "build_a_aab_sha256=$($buildA.AabHash)",
            "build_b_aab_sha256=$($buildB.AabHash)",
            "apk_byte_identical=$($apkByteIdentical.ToString().ToLowerInvariant())",
            "aab_byte_identical=$($aabByteIdentical.ToString().ToLowerInvariant())",
            "reproducible=true"
        ) | Set-Content -Path (Join-Path $distDir "BUILD-MANIFEST.txt") -Encoding utf8

        Write-Host "Reproducibility verified: both clean Android builds are byte-for-byte identical."
        Write-Host ""
        Write-Host "Verified unsigned APK: $distApk"
        Write-Host "Verified unsigned AAB: $distAab"
        Write-Host "Checksums: $distDir\SHA256SUMS.txt"
        Write-Host "Manifest: $distDir\BUILD-MANIFEST.txt"
    } finally {
        if ($null -eq $originalCargoTarget) {
            Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
        } else {
            $env:CARGO_TARGET_DIR = $originalCargoTarget
        }
        Remove-Item $evidenceRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
} else {
    Push-Location $AndroidRoot
    try {
        $savedErrorActionPreference = $ErrorActionPreference
        try {
            $ErrorActionPreference = "Continue"
            & $gradleCommand --no-daemon --stacktrace :app:assembleDebug | Out-Host
            $gradleExit = $LASTEXITCODE
        } finally {
            $ErrorActionPreference = $savedErrorActionPreference
        }
        if ($gradleExit -ne 0) { throw "Android debug build failed with exit code $gradleExit" }
    } finally {
        Pop-Location
    }
    Write-Host ""
    Write-Host "Debug APK: $AndroidRoot\app\build\outputs\apk\debug\app-debug.apk"
}
