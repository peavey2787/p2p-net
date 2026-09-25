param(
    [string]$Tree = ""
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$Root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$releaseInputPaths = @(
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    ".cargo/config.toml",
    "crates",
    "apps",
    "external",
    "examples",
    "assets"
)

function Invoke-GitLines {
    param([Parameter(Mandatory = $true)][string[]]$Arguments)
    $lines = @(& git @Arguments)
    if ($LASTEXITCODE -ne 0) {
        throw "git $($Arguments -join ' ') failed with exit code $LASTEXITCODE"
    }
    return $lines
}

function Get-Sha256Text {
    param([Parameter(Mandatory = $true)][string]$Text)
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [System.Text.Encoding]::UTF8.GetBytes($Text)
        return ([System.BitConverter]::ToString($sha.ComputeHash($bytes))).Replace("-", "").ToLowerInvariant()
    }
    finally {
        $sha.Dispose()
    }
}

$previousLocation = Get-Location
$tempIndex = $null
$tempGitDir = $null
$syntheticGitArgs = @()
$oldIndex = [Environment]::GetEnvironmentVariable("GIT_INDEX_FILE", "Process")
$fingerprintMode = "git-worktree"
try {
    Set-Location -LiteralPath $Root
    if (-not $Tree) {
        $hasHead = $false
        try {
            & git -C $Root rev-parse --verify "HEAD^{tree}" *> $null
            $hasHead = ($LASTEXITCODE -eq 0)
        }
        catch {
            $hasHead = $false
        }

        if ($hasHead) {
            $tempIndex = Join-Path ([System.IO.Path]::GetTempPath()) ("p2p-net-index-" + [Guid]::NewGuid().ToString("N"))
            [Environment]::SetEnvironmentVariable("GIT_INDEX_FILE", $tempIndex, "Process")
            Invoke-GitLines @("read-tree", "HEAD") | Out-Null
            Invoke-GitLines @("add", "-A") | Out-Null
            $Tree = (Invoke-GitLines @("write-tree") | Select-Object -First 1).Trim()
        }
        else {
            # Source archives intentionally do not contain .git. Build the same style of
            # Git tree inventory in a temporary object database without modifying source.
            $fingerprintMode = "synthetic-worktree"
            [Environment]::SetEnvironmentVariable("GIT_INDEX_FILE", $null, "Process")
            $tempGitDir = Join-Path ([System.IO.Path]::GetTempPath()) ("p2p-net-fingerprint-" + [Guid]::NewGuid().ToString("N") + ".git")
            & git init --bare --quiet $tempGitDir
            if ($LASTEXITCODE -ne 0) {
                throw "failed to initialize temporary Git object database for source-archive fingerprinting"
            }
            $syntheticGitArgs = @(
                "-c", "core.autocrlf=false",
                "-c", "core.filemode=false",
                "--git-dir=$tempGitDir",
                "--work-tree=$Root"
            )
            # A manifest-only handoff may intentionally omit Cargo.lock so Cargo can
            # resolve it from scratch. Git treats a never-existing pathspec as fatal,
            # so stage only release inputs that actually exist in the source archive.
            $existingReleaseInputPaths = @(
                $releaseInputPaths | Where-Object {
                    Test-Path -LiteralPath (Join-Path $Root $_)
                }
            )
            if ($existingReleaseInputPaths.Count -eq 0) {
                throw "no release inputs were found for source-archive fingerprinting"
            }
            & git @syntheticGitArgs add -A -- @existingReleaseInputPaths
            if ($LASTEXITCODE -ne 0) {
                throw "failed to stage release inputs for source-archive fingerprinting"
            }
            $treeLines = @(& git @syntheticGitArgs write-tree)
            if ($LASTEXITCODE -ne 0 -or $treeLines.Count -eq 0) {
                throw "failed to create synthetic release-input tree"
            }
            $Tree = ([string]$treeLines[0]).Trim()
        }
    }

    $lsTreeArgs = @("ls-tree", "-r", "--full-tree", $Tree, "--") + $releaseInputPaths
    if ($tempGitDir) {
        $lines = @(& git @syntheticGitArgs @lsTreeArgs)
        if ($LASTEXITCODE -ne 0) {
            throw "failed to enumerate synthetic release-input tree"
        }
    }
    else {
        $lines = @(Invoke-GitLines $lsTreeArgs)
    }
    $canonical = if ($lines.Count -gt 0) { ([string]::Join("`n", $lines) + "`n") } else { "" }
    $digest = Get-Sha256Text $canonical

    [pscustomobject]@{
        workspace_tree = $Tree
        fingerprint_mode = $fingerprintMode
        release_input_sha256 = $digest
        release_input_file_count = $lines.Count
        release_input_manifest = $canonical
    } | ConvertTo-Json -Compress
}
finally {
    [Environment]::SetEnvironmentVariable("GIT_INDEX_FILE", $oldIndex, "Process")
    if ($tempIndex -and (Test-Path -LiteralPath $tempIndex)) {
        Remove-Item -LiteralPath $tempIndex -Force
    }
    if ($tempGitDir -and (Test-Path -LiteralPath $tempGitDir)) {
        Remove-Item -LiteralPath $tempGitDir -Recurse -Force
    }
    Set-Location $previousLocation
}
