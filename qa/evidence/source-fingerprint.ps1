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
$oldIndex = [Environment]::GetEnvironmentVariable("GIT_INDEX_FILE", "Process")
try {
    Set-Location -LiteralPath $Root
    if (-not $Tree) {
        $tempIndex = Join-Path ([System.IO.Path]::GetTempPath()) ("p2p-net-index-" + [Guid]::NewGuid().ToString("N"))
        [Environment]::SetEnvironmentVariable("GIT_INDEX_FILE", $tempIndex, "Process")
        Invoke-GitLines @("read-tree", "HEAD") | Out-Null
        Invoke-GitLines @("add", "-A") | Out-Null
        $Tree = (Invoke-GitLines @("write-tree") | Select-Object -First 1).Trim()
    }

    $args = @("ls-tree", "-r", "--full-tree", $Tree, "--") + $releaseInputPaths
    $lines = @(Invoke-GitLines $args)
    $canonical = if ($lines.Count -gt 0) { ([string]::Join("`n", $lines) + "`n") } else { "" }
    $digest = Get-Sha256Text $canonical

    [pscustomobject]@{
        workspace_tree = $Tree
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
    Set-Location $previousLocation
}
