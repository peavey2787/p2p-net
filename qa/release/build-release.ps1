Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$NoInstallTools = $false
$NoPause = $false
$ForceValidation = $false
foreach ($arg in $args) {
    switch ($arg) {
        "--no-install-tools" { $NoInstallTools = $true }
        "--no-pause" { $NoPause = $true }
        "--force-validation" { $ForceValidation = $true }
        default { throw "Unknown argument: $arg" }
    }
}

$Root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$ReproRoot = Join-Path $Root "target\reproducible-release"
$RootParent = Split-Path -Parent $Root
$RootName = Split-Path -Leaf $Root
$WorktreeRoot = Join-Path $RootParent ("." + $RootName + "-release-worktrees")
$ValidationWorktree = Join-Path $WorktreeRoot "validation-source"
$WorktreeA = Join-Path $WorktreeRoot "source-a"
$WorktreeB = Join-Path $WorktreeRoot "source-b"
$TargetA = Join-Path $ReproRoot "target-a"
$TargetB = Join-Path $ReproRoot "target-b"
$script:ExitCode = 0

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)][string]$Command,
        [Parameter(Mandatory = $true)][string[]]$CommandArgs
    )

    & $Command @CommandArgs
    if ($LASTEXITCODE -ne 0) {
        throw "$Command failed with exit code $LASTEXITCODE"
    }
}

function Remove-ReleaseWorktree {
    param([Parameter(Mandatory = $true)][string]$Path)

    & git worktree remove --force $Path *> $null
    if (Test-Path -LiteralPath $Path) {
        Remove-Item -LiteralPath $Path -Recurse -Force
    }
}

function Cleanup-ReleaseWorktrees {
    $oldPreference = $ErrorActionPreference
    $ErrorActionPreference = "SilentlyContinue"
    Remove-ReleaseWorktree $ValidationWorktree
    Remove-ReleaseWorktree $WorktreeA
    Remove-ReleaseWorktree $WorktreeB
    & git worktree prune *> $null
    if (Test-Path -LiteralPath $WorktreeRoot) {
        Remove-Item -LiteralPath $WorktreeRoot -Recurse -Force
    }
    $ErrorActionPreference = $oldPreference
}

function Get-WorkingTreeStatus {
    $status = @(& git status --porcelain=v1 --untracked-files=all)
    if ($LASTEXITCODE -ne 0) {
        throw "git status failed with exit code $LASTEXITCODE"
    }
    return $status
}

function Get-SourceFingerprint {
    $scriptPath = Join-Path $Root "qa\evidence\source-fingerprint.ps1"
    $json = & $scriptPath
    if ($LASTEXITCODE -ne 0) {
        throw "source fingerprint generation failed with exit code $LASTEXITCODE"
    }
    return ($json | ConvertFrom-Json)
}

function Read-EvidenceManifest {
    param([Parameter(Mandatory = $true)][string]$Path)

    $values = @{}
    foreach ($line in Get-Content -LiteralPath $Path) {
        if (-not $line -or $line.StartsWith("#")) { continue }
        $separator = $line.IndexOf("=")
        if ($separator -gt 0) {
            $values[$line.Substring(0, $separator)] = $line.Substring($separator + 1)
        }
    }
    return $values
}

function Find-ValidationEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$ReleaseInputSha256,
        [string]$SearchRoot = $Root
    )

    $roots = @(
        (Join-Path $SearchRoot "qa\evidence\runs"),
        (Join-Path $SearchRoot "qa\evidence\recovered"),
        (Join-Path $SearchRoot "qa\evidence\attestations")
    )
    foreach ($evidenceRoot in $roots) {
        if (-not (Test-Path -LiteralPath $evidenceRoot)) {
            continue
        }
        $candidates = @(
            Get-ChildItem -LiteralPath $evidenceRoot -Filter "manifest.txt" -File -Recurse |
                Sort-Object LastWriteTimeUtc -Descending
        )
        foreach ($candidate in $candidates) {
            $values = Read-EvidenceManifest $candidate.FullName
            if ($values["result"] -eq "pass" -and
                $values["mode"] -eq "full" -and
                $values["release_input_sha256"] -eq $ReleaseInputSha256) {
                return [pscustomobject]@{
                    Path = $candidate.FullName
                    Values = $values
                    Sha256 = (Get-FileHash -LiteralPath $candidate.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
                }
            }
        }
    }
    return $null
}

function New-SourceSnapshotCommit {
    param(
        [Parameter(Mandatory = $true)][string]$Tree,
        [Parameter(Mandatory = $true)][string]$BaseCommit,
        [Parameter(Mandatory = $true)][string]$SourceEpoch
    )

    $names = @("GIT_AUTHOR_NAME", "GIT_AUTHOR_EMAIL", "GIT_AUTHOR_DATE", "GIT_COMMITTER_NAME", "GIT_COMMITTER_EMAIL", "GIT_COMMITTER_DATE")
    $previous = @{}
    foreach ($name in $names) {
        $previous[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
    }
    try {
        $env:GIT_AUTHOR_NAME = "p2p-net release snapshot"
        $env:GIT_AUTHOR_EMAIL = "release-snapshot@invalid"
        $env:GIT_AUTHOR_DATE = "$SourceEpoch +0000"
        $env:GIT_COMMITTER_NAME = "p2p-net release snapshot"
        $env:GIT_COMMITTER_EMAIL = "release-snapshot@invalid"
        $env:GIT_COMMITTER_DATE = "$SourceEpoch +0000"
        $snapshotCommit = ("p2p-net reproducible release snapshot`n" | & git commit-tree $Tree -p $BaseCommit).Trim()
        if ($LASTEXITCODE -ne 0 -or -not $snapshotCommit) {
            throw "git commit-tree failed while creating the release snapshot"
        }
        return $snapshotCommit
    }
    finally {
        foreach ($name in $names) {
            [Environment]::SetEnvironmentVariable($name, $previous[$name], "Process")
        }
    }
}

function Initialize-MsvcEnvironment {
    param([Parameter(Mandatory = $true)][string]$HostTarget)

    if ($HostTarget -notlike "*-pc-windows-msvc") {
        throw "Windows release runner requires an MSVC Rust host target (found: $HostTarget)"
    }

    $vswhereCandidates = @(
        (Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"),
        (Join-Path $env:ProgramFiles "Microsoft Visual Studio\Installer\vswhere.exe")
    )
    $vswhere = $vswhereCandidates | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Select-Object -First 1
    if (-not $vswhere) {
        throw "vswhere.exe was not found; install Visual Studio Build Tools with the MSVC and Windows SDK components"
    }

    $installLines = @(& $vswhere -latest -products "*" -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)
    if ($LASTEXITCODE -ne 0 -or $installLines.Count -eq 0) {
        throw "Visual Studio Build Tools with the x64 MSVC component were not found"
    }

    $installPath = $installLines[0].Trim()
    $vsDevCmd = Join-Path $installPath "Common7\Tools\VsDevCmd.bat"
    if (-not (Test-Path -LiteralPath $vsDevCmd)) {
        throw "VsDevCmd.bat was not found at $vsDevCmd"
    }

    $commandLine = "call `"$vsDevCmd`" -no_logo -arch=amd64 -host_arch=amd64 >nul && set"
    $environmentLines = @(& $env:ComSpec /d /s /c $commandLine)
    if ($LASTEXITCODE -ne 0) {
        throw "VsDevCmd.bat failed to initialize the MSVC release environment"
    }

    foreach ($line in $environmentLines) {
        $separator = $line.IndexOf("=")
        if ($separator -gt 0) {
            $name = $line.Substring(0, $separator)
            $value = $line.Substring($separator + 1)
            [Environment]::SetEnvironmentVariable($name, $value, "Process")
        }
    }

    if (-not (Get-Command link.exe -ErrorAction SilentlyContinue)) {
        throw "MSVC link.exe is unavailable after VsDevCmd initialization"
    }
}

function Invoke-ReproducibleBuild {
    param(
        [Parameter(Mandatory = $true)][string]$SourceRoot,
        [Parameter(Mandatory = $true)][string]$TargetDir,
        [Parameter(Mandatory = $true)][string]$SourceEpoch
    )

    if (Test-Path -LiteralPath $TargetDir) {
        Remove-Item -LiteralPath $TargetDir -Recurse -Force
    }
    New-Item -ItemType Directory -Path $TargetDir -Force | Out-Null

    $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE ".cargo" }
    $rustupHome = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $env:USERPROFILE ".rustup" }
    $encodedSeparator = [string][char]0x1f
    $rustFlags = @(
        "--remap-path-prefix=$SourceRoot=/src/p2p-net",
        "--remap-path-prefix=$TargetDir=/build/target",
        "--remap-path-prefix=$cargoHome=/cargo-home",
        "--remap-path-prefix=$rustupHome=/rustup-home",
        "-C",
        "linker=link.exe",
        "-C",
        "link-arg=/Brepro"
    )

    $previousLocation = Get-Location
    $controlledEnvNames = @(
        "RUSTFLAGS", "RUSTC", "RUSTDOC", "RUSTDOCFLAGS", "RUSTC_BOOTSTRAP",
        "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_TARGET_DIR", "CARGO_INCREMENTAL", "CARGO_BUILD_JOBS", "CARGO_TERM_COLOR",
        "CARGO_NET_OFFLINE", "SOURCE_DATE_EPOCH", "TZ", "LANG", "LC_ALL",
        "CARGO_BUILD_RUSTC", "CARGO_BUILD_RUSTC_WRAPPER",
        "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_RUSTFLAGS",
        "CARGO_BUILD_TARGET", "CARGO_BUILD_TARGET_DIR",
        "CARGO_PROFILE_RELEASE_OPT_LEVEL", "CARGO_PROFILE_RELEASE_DEBUG",
        "CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS", "CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS",
        "CARGO_PROFILE_RELEASE_LTO", "CARGO_PROFILE_RELEASE_PANIC",
        "CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "CARGO_PROFILE_RELEASE_RPATH",
        "CARGO_PROFILE_RELEASE_STRIP", "CARGO_PROFILE_RELEASE_INCREMENTAL",
        "CARGO_PROFILE_RELEASE_SPLIT_DEBUGINFO",
        "CC", "CXX", "AR", "CFLAGS", "CPPFLAGS", "CXXFLAGS", "LDFLAGS"
    )
    $previousEnvironment = @{}
    foreach ($name in $controlledEnvNames) {
        $previousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
    }

    try {
        Set-Location -LiteralPath $SourceRoot
        foreach ($name in $controlledEnvNames) {
            Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue
        }
        $env:CARGO_ENCODED_RUSTFLAGS = [string]::Join($encodedSeparator, $rustFlags)
        $env:CARGO_TARGET_DIR = $TargetDir
        $env:CARGO_INCREMENTAL = "0"
        $env:CARGO_BUILD_JOBS = "1"
        $env:CARGO_TERM_COLOR = "never"
        $env:CARGO_NET_OFFLINE = "true"
        $env:SOURCE_DATE_EPOCH = $SourceEpoch
        $env:TZ = "UTC"
        $env:LANG = "C"
        $env:LC_ALL = "C"
        Invoke-Checked -Command "cargo" -CommandArgs @(
            "build", "--release", "--locked", "--offline", "--features", "dashboard",
            "--example", "p2p_node", "-j", "1"
        )
    }
    finally {
        Set-Location $previousLocation
        foreach ($name in $controlledEnvNames) {
            [Environment]::SetEnvironmentVariable(
                $name, $previousEnvironment[$name], "Process"
            )
        }
    }
}

try {
    Set-Location -LiteralPath $Root
    if (-not (Test-Path -LiteralPath (Join-Path $Root ".git"))) {
        throw "release runner must be executed from a Git checkout"
    }

    Cleanup-ReleaseWorktrees
    if (Test-Path -LiteralPath $ReproRoot) {
        Remove-Item -LiteralPath $ReproRoot -Recurse -Force
    }

    Write-Host "p2p-net canonical Windows release build"
    Write-Host "Root: $Root"

    Write-Host ""
    Write-Host "==> Capture exact source snapshot"
    $workingStatus = @(Get-WorkingTreeStatus)
    $baseCommit = (& git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw "git rev-parse HEAD failed" }
    $baseTree = (& git rev-parse "HEAD^{tree}").Trim()
    if ($LASTEXITCODE -ne 0) { throw "git rev-parse tree failed" }
    $sourceEpoch = (& git show -s --format=%ct HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw "git show commit timestamp failed" }
    $fingerprint = Get-SourceFingerprint
    $sourceTree = [string]$fingerprint.workspace_tree
    $releaseInputSha256 = [string]$fingerprint.release_input_sha256
    $releaseInputCount = [string]$fingerprint.release_input_file_count
    Write-Host "Base commit: $baseCommit"
    Write-Host "Source snapshot tree: $sourceTree"
    Write-Host "Release-input SHA-256: $releaseInputSha256 ($releaseInputCount files)"
    if ($workingStatus.Count -eq 0) {
        Write-Host "Working tree: clean"
    }
    else {
        Write-Host "Working tree: dirty; exact tracked/untracked non-ignored content will be snapshotted"
    }

    # Freeze the exact captured source before validation. Validation and both
    # reproducibility builds operate on detached worktrees from this same
    # synthetic commit, so generated/untracked state in the live checkout cannot
    # redefine the release while it is running.
    $snapshotCommit = New-SourceSnapshotCommit $sourceTree $baseCommit $sourceEpoch

    $validationEvidence = $null
    if (-not $ForceValidation) {
        $validationEvidence = Find-ValidationEvidence $releaseInputSha256
    }
    if ($validationEvidence) {
        Write-Host ""
        Write-Host "==> Reuse matching full-validation evidence"
        Write-Host "Evidence: $($validationEvidence.Path)"
        Write-Host "Evidence kind: $($validationEvidence.Values['evidence_kind'])"
        Write-Host "Validated release-input SHA-256: $releaseInputSha256"
    }
    else {
        Write-Host ""
        Write-Host "==> Full production validation from frozen snapshot"
        New-Item -ItemType Directory -Path $ReproRoot -Force | Out-Null
        New-Item -ItemType Directory -Path $WorktreeRoot -Force | Out-Null
        Invoke-Checked -Command "git" -CommandArgs @(
            "worktree", "add", "--detach", $ValidationWorktree, $snapshotCommit
        )

        $validationArgs = @("--no-pause")
        if ($NoInstallTools) {
            $validationArgs += "--no-install-tools"
        }
        & (Join-Path $ValidationWorktree "run-full-validation.cmd") @validationArgs
        if ($LASTEXITCODE -ne 0) {
            throw "full production validation failed with exit code $LASTEXITCODE"
        }

        $trackedValidationStatus = @(
            & git -C $ValidationWorktree status --porcelain=v1 --untracked-files=no
        )
        if ($LASTEXITCODE -ne 0) {
            throw "git status failed while checking validation snapshot stability"
        }
        if ($trackedValidationStatus.Count -ne 0) {
            Write-Host "Tracked files changed inside the validation snapshot:"
            $trackedValidationStatus | ForEach-Object { Write-Host "  $_" }
            throw "validation modified tracked source files in the frozen snapshot"
        }

        $snapshotEvidence = Find-ValidationEvidence $releaseInputSha256 $ValidationWorktree
        if (-not $snapshotEvidence) {
            throw "full validation passed but no matching durable evidence manifest was produced for the frozen snapshot"
        }

        $sourceEvidenceDir = Split-Path -Parent $snapshotEvidence.Path
        $evidenceRunName = Split-Path -Leaf $sourceEvidenceDir
        $rootEvidenceRuns = Join-Path $Root "qa\evidence\runs"
        $copiedEvidenceDir = Join-Path $rootEvidenceRuns $evidenceRunName
        New-Item -ItemType Directory -Path $rootEvidenceRuns -Force | Out-Null
        if (Test-Path -LiteralPath $copiedEvidenceDir) {
            Remove-Item -LiteralPath $copiedEvidenceDir -Recurse -Force
        }
        Copy-Item -LiteralPath $sourceEvidenceDir -Destination $copiedEvidenceDir -Recurse -Force
        $copiedManifest = Join-Path $copiedEvidenceDir "manifest.txt"
        $copiedValues = Read-EvidenceManifest $copiedManifest
        $validationEvidence = [pscustomobject]@{
            Path = $copiedManifest
            Values = $copiedValues
            Sha256 = (Get-FileHash -LiteralPath $copiedManifest -Algorithm SHA256).Hash.ToLowerInvariant()
        }

        Remove-ReleaseWorktree $ValidationWorktree
    }

    Write-Host ""
    Write-Host "==> Resolve release identity"
    $hostLine = @(& rustc -vV | Where-Object { $_ -like "host: *" }) | Select-Object -First 1
    if ($LASTEXITCODE -ne 0 -or -not $hostLine) { throw "could not determine rustc host target" }
    $hostTarget = $hostLine.Substring(6).Trim()
    $rustcVersion = (& rustc --version).Trim()
    if ($LASTEXITCODE -ne 0) { throw "rustc --version failed" }
    $cargoVersion = (& cargo --version).Trim()
    if ($LASTEXITCODE -ne 0) { throw "cargo --version failed" }
    if ($rustcVersion -notlike "rustc 1.98.0 *") {
        throw "release requires rustc 1.98.0 exactly (found: $rustcVersion)"
    }

    Initialize-MsvcEnvironment $hostTarget

    Write-Host "Base commit: $baseCommit"
    Write-Host "Base tree: $baseTree"
    Write-Host "Snapshot commit: $snapshotCommit"
    Write-Host "Snapshot tree: $sourceTree"
    Write-Host "Target: $hostTarget"
    Write-Host "SOURCE_DATE_EPOCH: $sourceEpoch"
    Write-Host "Rust: $rustcVersion"
    Write-Host "Cargo: $cargoVersion"

    Write-Host ""
    Write-Host "==> Create independent clean source worktrees"
    New-Item -ItemType Directory -Path $ReproRoot -Force | Out-Null
    New-Item -ItemType Directory -Path $WorktreeRoot -Force | Out-Null
    Invoke-Checked -Command "git" -CommandArgs @(
        "worktree", "add", "--detach", $WorktreeA, $snapshotCommit
    )
    Invoke-Checked -Command "git" -CommandArgs @(
        "worktree", "add", "--detach", $WorktreeB, $snapshotCommit
    )

    $metadataJson = & cargo metadata --manifest-path (Join-Path $WorktreeA "Cargo.toml") --locked --format-version 1 --no-deps
    if ($LASTEXITCODE -ne 0) { throw "cargo metadata --locked failed for frozen release snapshot" }
    $metadata = $metadataJson | ConvertFrom-Json
    $package = @($metadata.packages | Where-Object { $_.name -eq "p2p-net" }) | Select-Object -First 1
    if (-not $package) { throw "could not find p2p-net package metadata in frozen release snapshot" }
    $version = [string]$package.version

    Write-Host ""
    Write-Host "==> Reproducibility build A"
    Invoke-ReproducibleBuild $WorktreeA $TargetA $sourceEpoch

    Write-Host ""
    Write-Host "==> Reproducibility build B"
    Invoke-ReproducibleBuild $WorktreeB $TargetB $sourceEpoch

    $artifactA = Join-Path $TargetA "release\examples\p2p_node.exe"
    $artifactB = Join-Path $TargetB "release\examples\p2p_node.exe"
    if (-not (Test-Path -LiteralPath $artifactA)) { throw "release artifact A was not produced: $artifactA" }
    if (-not (Test-Path -LiteralPath $artifactB)) { throw "release artifact B was not produced: $artifactB" }

    $hashA = (Get-FileHash -LiteralPath $artifactA -Algorithm SHA256).Hash.ToLowerInvariant()
    $hashB = (Get-FileHash -LiteralPath $artifactB -Algorithm SHA256).Hash.ToLowerInvariant()

    Write-Host ""
    Write-Host "==> Verify byte-for-byte reproducibility"
    Write-Host "Build A SHA-256: $hashA"
    Write-Host "Build B SHA-256: $hashB"
    if ($hashA -ne $hashB) {
        throw "independent clean release builds are not byte-for-byte identical; targets are retained under $ReproRoot for investigation"
    }
    Write-Host "Reproducibility verified: both clean builds have identical SHA-256 digests."

    $distDir = Join-Path $Root ("dist\" + $hostTarget)
    $artifactName = "p2p_node-$version-$hostTarget.exe"
    if (Test-Path -LiteralPath $distDir) {
        Remove-Item -LiteralPath $distDir -Recurse -Force
    }
    New-Item -ItemType Directory -Path $distDir -Force | Out-Null
    Copy-Item -LiteralPath $artifactA -Destination (Join-Path $distDir $artifactName)

    @("$hashA  $artifactName") | Set-Content -LiteralPath (Join-Path $distDir "SHA256SUMS.txt") -Encoding ASCII

    $sourceManifestPath = Join-Path $distDir "SOURCE-MANIFEST.txt"
    @(& git ls-tree -r --full-tree $sourceTree) | Set-Content -LiteralPath $sourceManifestPath -Encoding UTF8
    if ($LASTEXITCODE -ne 0) { throw "git ls-tree failed while writing the release source manifest" }
    $sourceManifestSha256 = (Get-FileHash -LiteralPath $sourceManifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
    @("$sourceManifestSha256  SOURCE-MANIFEST.txt") | Set-Content -LiteralPath (Join-Path $distDir "SOURCE-MANIFEST.sha256.txt") -Encoding ASCII

    $releaseInputsPath = Join-Path $distDir "RELEASE-INPUTS.txt"
    [System.IO.File]::WriteAllText(
        $releaseInputsPath,
        [string]$fingerprint.release_input_manifest,
        [System.Text.UTF8Encoding]::new($false)
    )
    @("$releaseInputSha256  RELEASE-INPUTS.txt") | Set-Content -LiteralPath (Join-Path $distDir "RELEASE-INPUTS.sha256.txt") -Encoding ASCII

    $validationEvidenceBundle = Join-Path $distDir "validation-evidence"
    New-Item -ItemType Directory -Path $validationEvidenceBundle -Force | Out-Null
    Copy-Item -Path (Join-Path (Split-Path -Parent $validationEvidence.Path) "*") -Destination $validationEvidenceBundle -Recurse -Force
    $validationEvidenceOutput = Join-Path $validationEvidenceBundle "manifest.txt"
    $validationEvidenceKind = [string]$validationEvidence.Values["evidence_kind"]
    $validationEvidenceRelative = $validationEvidence.Path
    if ($validationEvidenceRelative.StartsWith($Root, [System.StringComparison]::OrdinalIgnoreCase)) {
        $validationEvidenceRelative = $validationEvidenceRelative.Substring($Root.Length).TrimStart([char]92, [char]47)
    }
    $workingTreeState = if ($workingStatus.Count -eq 0) { "clean" } else { "dirty-snapshotted" }

    @(
        "project=p2p-net",
        "version=$version",
        "git_base_commit=$baseCommit",
        "git_base_tree=$baseTree",
        "source_snapshot_commit=$snapshotCommit",
        "source_snapshot_tree=$sourceTree",
        "source_manifest_sha256=$sourceManifestSha256",
        "release_input_sha256=$releaseInputSha256",
        "release_input_file_count=$releaseInputCount",
        "working_tree_state=$workingTreeState",
        "source_date_epoch=$sourceEpoch",
        "rustc=$rustcVersion",
        "cargo=$cargoVersion",
        "target=$hostTarget",
        "linker=link.exe",
        "profile=release",
        "features=default,dashboard",
        "artifact=$artifactName",
        "sha256=$hashA",
        "reproducibility=verified-two-independent-clean-worktrees-from-exact-snapshot",
        "validation=durable-evidence-pass",
        "validation_evidence_kind=$validationEvidenceKind",
        "validation_evidence_path=$validationEvidenceRelative",
        "validation_evidence_sha256=$($validationEvidence.Sha256)"
    ) | Set-Content -LiteralPath (Join-Path $distDir "BUILD-MANIFEST.txt") -Encoding ASCII

    Write-Host ""
    Write-Host "==> Release output"
    Write-Host "Artifact: $(Join-Path $distDir $artifactName)"
    Write-Host "Manifest: $(Join-Path $distDir 'BUILD-MANIFEST.txt')"
    Write-Host "Checksums: $(Join-Path $distDir 'SHA256SUMS.txt')"
    Write-Host "Source manifest: $sourceManifestPath"
    Write-Host "Validation evidence bundle: $validationEvidenceBundle ($validationEvidenceKind)"
    Write-Host ""
    Write-Host "Canonical Windows release build completed successfully."
}
catch {
    Write-Host ""
    Write-Host "============================================================" -ForegroundColor Red
    Write-Host "RELEASE BUILD FAILED" -ForegroundColor Red
    Write-Host "============================================================" -ForegroundColor Red
    Write-Host $_.Exception.Message -ForegroundColor Red
    $script:ExitCode = 1
}
finally {
    Set-Location -LiteralPath $Root
    Cleanup-ReleaseWorktrees
}

if (-not $NoPause -and -not $env:CI) {
    Write-Host ""
    if ($script:ExitCode -eq 0) {
        Read-Host "Release build complete. Press Enter to close" | Out-Null
    }
    else {
        Read-Host "Release build failed. Press Enter to close" | Out-Null
    }
}

exit $script:ExitCode
