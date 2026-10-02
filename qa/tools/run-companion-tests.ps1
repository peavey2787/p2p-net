# Runs a companion crate's (external\<dir>, e.g. libp2p-relay) unit, smoke, and
# doc tests against the audited root Cargo.lock. Usage:
#   run-companion-tests.ps1 -Companion <dir under external\>
# Companions are deliberately outside the
# production workspace and have no committed lockfile (/external/**/Cargo.lock is
# ignored), so seed its lockfile from the root lock, let Cargo prune it to the
# companion's graph, and fail if that graph needs any package version the root
# lock does not pin.
param([Parameter(Mandatory = $true)][string] $Companion)
$ErrorActionPreference = 'Stop'

$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$Manifest = Join-Path $Root "external\$Companion\Cargo.toml"
$RootLock = Join-Path $Root 'Cargo.lock'
$CompanionLock = Join-Path $Root "external\$Companion\Cargo.lock"
if (-not (Test-Path -LiteralPath $Manifest)) {
    Write-Host "ERROR: no companion manifest at $Manifest"
    exit 1
}

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

# One test thread: upstream suites (e.g. gossipsub peer scoring) assert
# wall-clock sleep windows that parallel tests on a loaded runner overshoot.
& cargo test --manifest-path $Manifest --locked --all-features -j 1 -- --test-threads=1
exit $LASTEXITCODE
