param()

$ErrorActionPreference = "Stop"
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$DistDir = Join-Path $RepoRoot "dist\crates"
$PackageTargetDir = Join-Path $RepoRoot "target\package-crates"

function Quote-NativeArgument([string]$Value) {
    if ($null -eq $Value -or $Value.Length -eq 0) { return '""' }
    if ($Value -notmatch '[\s"]') { return $Value }
    return '"' + $Value.Replace('"', '\"') + '"'
}

function Test-NormalizedManifestHasPathDependency([string]$Text) {
    $inDependencySection = $false
    foreach ($line in ($Text -split "`r?`n")) {
        if ($line -match '^\s*\[') {
            $inDependencySection = $line -match '^\s*\[(?:(?:target\..+\.)?(?:dev-|build-)?dependencies)(?:\.|\])'
        }
        if ($inDependencySection -and $line -match '\bpath\s*=') { return $true }
    }
    return $false
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
    if (-not $process.Start()) {
        throw "Failed to start native command: $FilePath $($ArgumentList -join ' ')"
    }
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    $output = ($stdout + $stderr).Trim()
    $exitCode = $process.ExitCode
    $process.Dispose()
    return [PSCustomObject]@{ ExitCode = $exitCode; Output = $output }
}

function Invoke-Checked([string]$FilePath, [string[]]$ArgumentList, [string]$What) {
    $result = Invoke-NativeCapture $FilePath $ArgumentList
    if ($result.Output) { Write-Host $result.Output }
    if ($result.ExitCode -ne 0) {
        throw "$What failed with exit code $($result.ExitCode)"
    }
    return $result.Output
}

$PackageProgressActivity = "Crates.io package validation"
$PackageProgressPhases = 5

function Set-PackageProgress([int]$Phase, [string]$Status, [switch]$PhaseComplete) {
    $boundedPhase = [Math]::Max(1, [Math]::Min($PackageProgressPhases, $Phase))
    if ($PhaseComplete) {
        $percent = [Math]::Floor(($boundedPhase / $PackageProgressPhases) * 100)
    } else {
        $percent = [Math]::Floor((($boundedPhase - 1) / $PackageProgressPhases) * 100)
    }
    Write-Progress -Activity $PackageProgressActivity -Status "[$boundedPhase/$PackageProgressPhases] $Status" -PercentComplete $percent
}

function Invoke-CheckedLive(
    [string]$FilePath,
    [string[]]$ArgumentList,
    [string]$What,
    [int]$Phase
) {
    # Long Cargo commands must inherit the console. Redirecting both streams and
    # calling ReadToEnd() hid all Cargo progress until exit and could deadlock if
    # a redirected pipe filled. This path deliberately streams native output.
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $FilePath
    $startInfo.Arguments = (($ArgumentList | ForEach-Object { Quote-NativeArgument $_ }) -join ' ')
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $false
    $startInfo.RedirectStandardOutput = $false
    $startInfo.RedirectStandardError = $false

    Write-Host "    -> $What"
    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $startInfo
    if (-not $process.Start()) {
        throw "Failed to start native command: $FilePath $($ArgumentList -join ' ')"
    }

    $startedAt = Get-Date
    try {
        while (-not $process.WaitForExit(1000)) {
            $elapsedSeconds = [int]((Get-Date) - $startedAt).TotalSeconds
            Set-PackageProgress $Phase "$What running - ${elapsedSeconds}s elapsed"
        }
        $exitCode = $process.ExitCode
    } finally {
        $process.Dispose()
    }

    if ($exitCode -ne 0) {
        throw "$What failed with exit code $exitCode"
    }
}

Push-Location $RepoRoot
try {
    $rust = Invoke-Checked "rustc.exe" @("+1.98.0", "--version") "rustc +1.98.0 --version"
    if (-not $rust.StartsWith("rustc 1.98.0 ")) { throw "Expected Rust 1.98.0, got: $rust" }

    Write-Host ""
    Write-Host "==> Locked workspace dependency preflight"
    $metadataProbe = Invoke-NativeCapture "cargo.exe" @("+1.98.0", "metadata", "--locked", "--format-version", "1")
    if ($metadataProbe.ExitCode -ne 0) {
        if ($metadataProbe.Output) { Write-Host $metadataProbe.Output }
        throw "cargo metadata --locked failed with exit code $($metadataProbe.ExitCode)"
    }
    Write-Host "Committed Cargo.lock matches the current workspace graph."

    Set-PackageProgress 1 "Package crates.io payloads"
    Write-Host "==> [1/$PackageProgressPhases] Package publishable crates"
    if (Test-Path -LiteralPath $PackageTargetDir) {
        Remove-Item -LiteralPath $PackageTargetDir -Recurse -Force
    }
    New-Item -ItemType Directory -Force -Path $PackageTargetDir | Out-Null
    $oldCargoTargetDir = $env:CARGO_TARGET_DIR
    $env:CARGO_TARGET_DIR = $PackageTargetDir
    $supportManifest = Join-Path $RepoRoot "external\libp2p-webrtc\Cargo.toml"
    Invoke-CheckedLive "cargo.exe" @(
        "+1.98.0", "package",
        "--manifest-path", $supportManifest,
        "--allow-dirty"
    ) "cargo package p2p-net-webrtc" 1

    # p2p-net's normalized manifest intentionally changes the local path+version
    # dependency into a crates.io dependency. Until the companion is actually
    # published, verify that normalized package with a command-line-only Cargo
    # patch. The override is local to this invocation and cannot be serialized
    # into the .crate payload.
    $supportPatch = "patch.crates-io.p2p-net-webrtc.path='external/libp2p-webrtc'"
    Invoke-CheckedLive "cargo.exe" @(
        "+1.98.0", "package",
        "--package", "p2p-net",
        "--locked", "--allow-dirty",
        "--config", $supportPatch
    ) "cargo package p2p-net with unpublished-companion verification patch" 1
    Set-PackageProgress 1 "Both crates packaged and verified" -PhaseComplete

    Write-Host ""
    Set-PackageProgress 2 "Locate normalized crates.io payloads"
    Write-Host "==> [2/$PackageProgressPhases] Locate normalized crates.io payloads"
    $supportCrate = Join-Path $PackageTargetDir "package\p2p-net-webrtc-0.1.0.crate"
    $rootCrate = Join-Path $PackageTargetDir "package\p2p-net-0.1.0.crate"
    foreach ($crate in @($supportCrate, $rootCrate)) {
        if (-not (Test-Path $crate -PathType Leaf)) { throw "Missing packaged crate: $crate" }
    }
    $env:CARGO_TARGET_DIR = $oldCargoTargetDir
    Set-PackageProgress 2 "Both normalized .crate payloads found" -PhaseComplete

    Set-PackageProgress 3 "Inspect normalized package manifests"
    Write-Host ""
    Write-Host "==> [3/$PackageProgressPhases] Inspect normalized package manifests"

    $rootArchiveEntries = Invoke-Checked "tar.exe" @("-tf", $rootCrate) "inspect packaged p2p-net file list"
    foreach ($forbiddenEntry in @(
        "p2p-net-0.1.0/.cargo/",
        "p2p-net-0.1.0/external/libp2p-dns/",
        "p2p-net-0.1.0/external/libp2p-mdns-placeholder/",
        "p2p-net-0.1.0/external/libp2p-webrtc/"
    )) {
        if (($rootArchiveEntries -split "`r?`n") | Where-Object { $_.StartsWith($forbiddenEntry) }) {
            throw "Packaged p2p-net unexpectedly contains workspace-only entry: $forbiddenEntry"
        }
    }

    $normalizedRoot = Invoke-Checked "tar.exe" @("-xOf", $rootCrate, "p2p-net-0.1.0/Cargo.toml") "inspect packaged p2p-net Cargo.toml"
    if (Test-NormalizedManifestHasPathDependency $normalizedRoot) { throw "Packaged p2p-net still contains a dependency path" }
    if ($normalizedRoot.Contains("[patch.crates-io]")) { throw "Packaged p2p-net still contains [patch.crates-io]" }
    if ($normalizedRoot.Contains("[workspace]")) { throw "Packaged p2p-net unexpectedly retains the repository workspace table" }
    if (-not $normalizedRoot.Contains('package = "p2p-net-webrtc"')) {
        throw "Packaged p2p-net does not depend on registry package p2p-net-webrtc"
    }
    if (-not $normalizedRoot.Contains('version = "0.1.0"')) {
        throw "Packaged p2p-net does not retain the p2p-net-webrtc 0.1.0 registry constraint"
    }

    $normalizedSupport = Invoke-Checked "tar.exe" @("-xOf", $supportCrate, "p2p-net-webrtc-0.1.0/Cargo.toml") "inspect packaged p2p-net-webrtc Cargo.toml"
    if (Test-NormalizedManifestHasPathDependency $normalizedSupport) { throw "Packaged p2p-net-webrtc contains a dependency path" }
    if ($normalizedSupport.Contains("[patch.crates-io]")) { throw "Packaged p2p-net-webrtc contains [patch.crates-io]" }
    Set-PackageProgress 3 "Normalized manifests are registry-self-contained" -PhaseComplete

    Write-Host ""
    Set-PackageProgress 4 "Compile packaged payload as a downstream consumer"
    Write-Host "==> [4/$PackageProgressPhases] Compile packaged payload as a downstream consumer"
    $smokeRoot = Join-Path $env:TEMP ("p2p-net-crates-smoke-" + [Guid]::NewGuid().ToString("N"))
    try {
        $unpacked = Join-Path $smokeRoot "unpacked"
        $consumer = Join-Path $smokeRoot "consumer"
        $consumerSrc = Join-Path $consumer "src"
        New-Item -ItemType Directory -Force -Path $unpacked, $consumerSrc | Out-Null
        Invoke-Checked "tar.exe" @("-xf", $supportCrate, "-C", $unpacked) "extract packaged p2p-net-webrtc" | Out-Null
        Invoke-Checked "tar.exe" @("-xf", $rootCrate, "-C", $unpacked) "extract packaged p2p-net" | Out-Null
        $rootPackageDir = (Join-Path $unpacked "p2p-net-0.1.0").Replace('\', '/')
        $supportPackageDir = (Join-Path $unpacked "p2p-net-webrtc-0.1.0").Replace('\', '/')
        $consumerManifest = @"
[package]
name = "p2p-net-package-consumer-smoke"
version = "0.0.0"
edition = "2021"
publish = false

[dependencies]
p2p-net = { path = "$rootPackageDir" }

[patch.crates-io]
p2p-net-webrtc = { path = "$supportPackageDir" }
"@
        $consumerMain = @"
async fn exercise_public_api() -> Result<(), p2p_net::NetError> {
    let node = p2p_net::start_node(p2p_net::NodeConfig::default()).await?;
    let _peers = node.get_peers().await?;
    node.shutdown().await;
    Ok(())
}

fn main() {
    let _ = exercise_public_api;
}
"@
        [IO.File]::WriteAllText((Join-Path $consumer "Cargo.toml"), $consumerManifest, ([System.Text.UTF8Encoding]::new($false)))
        [IO.File]::WriteAllText((Join-Path $consumerSrc "main.rs"), $consumerMain, ([System.Text.UTF8Encoding]::new($false)))
        $consumerManifestPath = Join-Path $consumer "Cargo.toml"
        Invoke-CheckedLive "cargo.exe" @("+1.98.0", "generate-lockfile", "--manifest-path", $consumerManifestPath) "generate downstream package smoke lockfile" 4
        Invoke-CheckedLive "cargo.exe" @("+1.98.0", "check", "--manifest-path", $consumerManifestPath, "--locked") "compile packaged p2p-net as downstream consumer" 4
        Set-PackageProgress 4 "Downstream consumer compile passed" -PhaseComplete
    } finally {
        if ($smokeRoot -and (Test-Path $smokeRoot)) { Remove-Item -Recurse -Force $smokeRoot }
    }

    Set-PackageProgress 5 "Write crates.io release artifacts"
    Write-Host ""
    Write-Host "==> [5/$PackageProgressPhases] Write crates.io release artifacts"
    New-Item -ItemType Directory -Force -Path $DistDir | Out-Null
    $distSupport = Join-Path $DistDir "p2p-net-webrtc-0.1.0.crate"
    $distRoot = Join-Path $DistDir "p2p-net-0.1.0.crate"
    Copy-Item $supportCrate $distSupport -Force
    Copy-Item $rootCrate $distRoot -Force
    $supportHash = (Get-FileHash $distSupport -Algorithm SHA256).Hash.ToLowerInvariant()
    $rootHash = (Get-FileHash $distRoot -Algorithm SHA256).Hash.ToLowerInvariant()
    @(
        "$supportHash  $(Split-Path $distSupport -Leaf)",
        "$rootHash  $(Split-Path $distRoot -Leaf)"
    ) | Set-Content -Path (Join-Path $DistDir "SHA256SUMS.txt") -Encoding ascii
    @(
        "1. cargo +1.98.0 publish --dry-run --manifest-path external/libp2p-webrtc/Cargo.toml --registry crates-io",
        "2. cargo +1.98.0 publish --manifest-path external/libp2p-webrtc/Cargo.toml --registry crates-io",
        "3. Wait until crates.io/index.crates.io resolves p2p-net-webrtc 0.1.0.",
        "4. cargo +1.98.0 publish --dry-run --package p2p-net --registry crates-io --locked",
        "5. cargo +1.98.0 publish --package p2p-net --registry crates-io --locked"
    ) | Set-Content -Path (Join-Path $DistDir "PUBLISH-ORDER.txt") -Encoding ascii

    Set-PackageProgress 5 "Crates.io release artifacts ready" -PhaseComplete
    Write-Progress -Activity $PackageProgressActivity -Completed

    Write-Host ""
    Write-Host "Crates.io package payloads are normalized and downstream-consumer checked."
    Write-Host "Artifacts: $DistDir"
    Write-Host "Follow $DistDir\PUBLISH-ORDER.txt for the required companion-first publish sequence."
} finally {
    if (Get-Variable -Name oldCargoTargetDir -Scope Local -ErrorAction SilentlyContinue) {
        $env:CARGO_TARGET_DIR = $oldCargoTargetDir
    }
    Write-Progress -Activity $PackageProgressActivity -Completed
    Pop-Location
}
