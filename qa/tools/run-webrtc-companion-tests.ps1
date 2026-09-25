# Runs the p2p-net-webrtc companion crate's unit, smoke, and doc tests against
# the audited root Cargo.lock. The companion is deliberately outside the
# production workspace and has no committed lockfile (/external/**/Cargo.lock is
# ignored), so seed its lockfile from the root lock, let Cargo prune it to the
# companion's graph, and fail if that graph needs any package version the root
# lock does not pin.
$ErrorActionPreference = 'Stop'

$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$Manifest = Join-Path $Root 'external\libp2p-webrtc\Cargo.toml'
$RootLock = Join-Path $Root 'Cargo.lock'
$CompanionLock = Join-Path $Root 'external\libp2p-webrtc\Cargo.lock'

function Get-LockPackages([string] $Path) {
    $name = $null
    foreach ($line in Get-Content -LiteralPath $Path) {
        if ($line -match '^name = "(.+)"$') {
            $name = $Matches[1]
        } elseif ($line -match '^version = "(.+)"$') {
            "$name $($Matches[1])"
        }
    }
}

Copy-Item -LiteralPath $RootLock -Destination $CompanionLock -Force
& cargo metadata --format-version 1 --manifest-path $Manifest | Out-Null
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$pinned = @{}
foreach ($package in Get-LockPackages $RootLock) {
    $pinned[$package] = $true
}
$unpinned = @(Get-LockPackages $CompanionLock | Where-Object { -not $pinned.ContainsKey($_) } | Sort-Object -Unique)
if ($unpinned.Count -gt 0) {
    Write-Host 'ERROR: the companion test graph needs packages the audited root Cargo.lock does not pin:'
    $unpinned | ForEach-Object { Write-Host "  $_" }
    exit 1
}
Write-Host 'Companion lockfile is a subset of the audited root Cargo.lock.'

& cargo test --manifest-path $Manifest --locked --all-features -j 1
exit $LASTEXITCODE
