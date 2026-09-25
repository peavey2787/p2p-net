param(
    [Parameter(Mandatory = $true)][string]$Launcher
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$Root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$RunId = ((Get-Date).ToUniversalTime().ToString("yyyyMMddTHHmmssZ")) + "-windows-" + [Guid]::NewGuid().ToString("N").Substring(0, 8)
$RunDir = Join-Path $Root ("qa\evidence\runs\" + $RunId)
$Transcript = Join-Path $RunDir "transcript.txt"
$Manifest = Join-Path $RunDir "manifest.txt"
$GitStatusPath = Join-Path $RunDir "git-status.txt"
$InputsPath = Join-Path $RunDir "release-inputs.txt"
$LockHashPath = Join-Path $RunDir "Cargo.lock.sha256.txt"
$StartedUtc = (Get-Date).ToUniversalTime().ToString("o")
$CompletionSentinel = Join-Path $RunDir "launcher-complete.txt"
$RawArgs = if ($env:P2P_VALIDATION_ORIGINAL_ARGS) { $env:P2P_VALIDATION_ORIGINAL_ARGS.Trim() } else { "" }
$Tokens = @()
if ($RawArgs) {
    $Tokens = @($RawArgs -split '\s+' | Where-Object { $_ })
}
$ResumeFrom = "full"
for ($i = 0; $i -lt $Tokens.Count; $i++) {
    if ($Tokens[$i] -ieq "--from" -and ($i + 1) -lt $Tokens.Count) {
        $ResumeFrom = $Tokens[$i + 1]
    }
}
$Mode = if ($ResumeFrom -eq "full") { "full" } else { "resume" }

trap {
    $details = ($_ | Out-String).TrimEnd()
    Write-Host "Validation evidence wrapper failed:"
    Write-Host $details
    try {
        if (Test-Path -LiteralPath $RunDir) {
            $details | Out-File -LiteralPath $Transcript -Append -Encoding UTF8
            [System.IO.File]::WriteAllText(
                (Join-Path $RunDir "WRAPPER-FAIL.txt"),
                ($details + "`n"),
                [System.Text.UTF8Encoding]::new($false)
            )
        }
    }
    catch {}
    exit 125
}

New-Item -ItemType Directory -Path $RunDir -Force | Out-Null
Write-Host "Validation evidence: $RunDir"

function Get-SourceFingerprint {
    $fingerprintScript = Join-Path $PSScriptRoot "source-fingerprint.ps1"
    $raw = @(& $fingerprintScript)
    if (-not $raw -or $raw.Count -eq 0) {
        throw "source fingerprint produced no JSON output"
    }
    $json = ([string]::Join("`n", $raw)).Trim()
    try {
        return ($json | ConvertFrom-Json)
    }
    catch {
        throw "source fingerprint produced invalid JSON: $json"
    }
}

$PreValidationFingerprint = Get-SourceFingerprint
if (-not $PreValidationFingerprint) {
    throw "failed to capture pre-validation source fingerprint"
}
[System.IO.File]::WriteAllText(
    $InputsPath,
    [string]$PreValidationFingerprint.release_input_manifest,
    [System.Text.UTF8Encoding]::new($false)
)
function Capture-Line {
    param([string]$CommandLine)
    try {
        $output = @(& $env:ComSpec /d /s /c $CommandLine 2>$null)
        if ($LASTEXITCODE -eq 0 -and $output.Count -gt 0) {
            return ($output -join " ").Trim()
        }
    }
    catch {}
    return "unknown"
}

function Write-Manifest {
    param([int]$ExitCode)
    $finishedUtc = (Get-Date).ToUniversalTime().ToString("o")
    $result = if ($ExitCode -eq 0) { "pass" } else { "fail" }
    $postFingerprint = $null
    try {
        $postFingerprint = Get-SourceFingerprint
    }
    catch {}

    $gitCommit = Capture-Line "git rev-parse HEAD"
    $gitTree = Capture-Line "git rev-parse HEAD^{tree}"
    $gitStatus = @()
    $gitStatusAvailable = $false
    try {
        $gitStatus = @(& git -C $Root status --porcelain=v1 --untracked-files=all 2>$null)
        $gitStatusAvailable = ($LASTEXITCODE -eq 0)
    }
    catch {}
    [System.IO.File]::WriteAllLines($GitStatusPath, $gitStatus, [System.Text.UTF8Encoding]::new($false))
    $lockHash = if (Test-Path -LiteralPath (Join-Path $Root "Cargo.lock")) {
        (Get-FileHash -LiteralPath (Join-Path $Root "Cargo.lock") -Algorithm SHA256).Hash.ToLowerInvariant()
    } else { "missing" }
    [System.IO.File]::WriteAllText($LockHashPath, ($lockHash + "  Cargo.lock`n"), [System.Text.ASCIIEncoding]::new())

    $sourceWorkspaceTree = [string]$PreValidationFingerprint.workspace_tree
    $sourceFingerprintMode = if ($PreValidationFingerprint.PSObject.Properties["fingerprint_mode"]) { [string]$PreValidationFingerprint.fingerprint_mode } else { "git-worktree" }
    $releaseInputSha256 = [string]$PreValidationFingerprint.release_input_sha256
    $releaseInputFileCount = [string]$PreValidationFingerprint.release_input_file_count
    $postReleaseInputSha256 = "unknown"
    $postReleaseInputFileCount = "unknown"
    if ($postFingerprint) {
        $postReleaseInputSha256 = [string]$postFingerprint.release_input_sha256
        $postReleaseInputFileCount = [string]$postFingerprint.release_input_file_count
    }
    $releaseInputsStable = if ($postReleaseInputSha256 -eq $releaseInputSha256) { "true" } else { "false" }
    $gitStatusState = if ($gitStatus.Count -eq 0) { "clean" } else { "dirty" }
    if (-not $gitStatusAvailable) { $gitStatusState = "unavailable" }

    $lines = @(
        "schema=1",
        "evidence_kind=machine-captured",
        "platform=windows",
        "mode=$Mode",
        "resume_from=$ResumeFrom",
        "result=$result",
        "exit_code=$ExitCode",
        "started_utc=$StartedUtc",
        "finished_utc=$finishedUtc",
        "source_workspace_tree=$sourceWorkspaceTree",
        "source_fingerprint_mode=$sourceFingerprintMode",
        "release_input_sha256=$releaseInputSha256",
        "release_input_file_count=$releaseInputFileCount",
        "post_validation_release_input_sha256=$postReleaseInputSha256",
        "post_validation_release_input_file_count=$postReleaseInputFileCount",
        "release_inputs_stable=$releaseInputsStable",
        "git_commit=$gitCommit",
        "git_tree=$gitTree",
        "git_status=$gitStatusState",
        "cargo_lock_sha256=$lockHash",
        "rustc=$(Capture-Line 'rustc --version')",
        "rustc_verbose=$(Capture-Line 'rustc -vV')",
        "cargo=$(Capture-Line 'cargo --version')",
        "cargo_audit=$(Capture-Line 'cargo audit --version')",
        "cargo_deny=$(Capture-Line 'cargo deny --version')",
        "arguments=$RawArgs",
        "transcript=transcript.txt"
    )
    [System.IO.File]::WriteAllLines($Manifest, $lines, [System.Text.UTF8Encoding]::new($false))
    $markerName = if ($ExitCode -eq 0) { "PASS" } else { "FAIL" }
    $marker = Join-Path $RunDir $markerName
    [System.IO.File]::WriteAllText($marker, ($result + "`n"), [System.Text.ASCIIEncoding]::new())
}

$oldActive = $env:P2P_VALIDATION_EVIDENCE_ACTIVE
$oldCompletionSentinel = $env:P2P_VALIDATION_COMPLETION_SENTINEL
$env:P2P_VALIDATION_EVIDENCE_ACTIVE = "1"
$env:P2P_VALIDATION_COMPLETION_SENTINEL = $CompletionSentinel
if (Test-Path -LiteralPath $CompletionSentinel) {
    Remove-Item -LiteralPath $CompletionSentinel -Force
}
try {
    $commandLine = 'call "{0}" {1} 2>&1' -f $Launcher, $RawArgs
    & $env:ComSpec /d /s /c $commandLine | Tee-Object -FilePath $Transcript
    $status = $LASTEXITCODE
    if ($status -eq 0 -and -not (Test-Path -LiteralPath $CompletionSentinel)) {
        $message = "Validation launcher exited with status 0 before reaching its completion sentinel."
        Write-Host $message
        $message | Out-File -LiteralPath $Transcript -Append -Encoding UTF8
        $status = 124
    }
}
catch {
    $_ | Out-String | Out-File -LiteralPath $Transcript -Append -Encoding UTF8
    Write-Host $_
    $status = 1
}
finally {
    $env:P2P_VALIDATION_EVIDENCE_ACTIVE = $oldActive
    $env:P2P_VALIDATION_COMPLETION_SENTINEL = $oldCompletionSentinel
}

Write-Manifest $status
Write-Host "Validation evidence saved: $Manifest"
exit $status
