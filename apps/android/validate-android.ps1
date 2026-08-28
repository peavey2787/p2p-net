$ErrorActionPreference = "Stop"
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path

& (Join-Path $RepoRoot "run-full-validation.cmd") --no-clean
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

& (Join-Path $PSScriptRoot "build-android.ps1")
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$reproManifest = Join-Path $RepoRoot "dist\android\BUILD-MANIFEST.txt"
if (-not (Test-Path $reproManifest -PathType Leaf)) {
    throw "Android reproducibility manifest missing: $reproManifest"
}
foreach ($required in @("reproducible=true", "apk_byte_identical=true", "aab_byte_identical=true")) {
    $pattern = '^' + [Regex]::Escape($required) + '$'
    if (-not (Select-String -Path $reproManifest -Pattern $pattern -Quiet)) {
        throw "Android release manifest missing $required"
    }
}

Push-Location $PSScriptRoot
try {
    & gradle --no-daemon --stacktrace :app:lintRelease :app:testReleaseUnitTest
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} finally {
    Pop-Location
}
