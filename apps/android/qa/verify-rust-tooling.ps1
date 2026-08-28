$ErrorActionPreference = "Stop"

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
    $exitCode = $process.ExitCode
    $process.Dispose()

    return [PSCustomObject]@{
        ExitCode = $exitCode
        Output = ($stdout + $stderr).Trim()
    }
}

$rustc = Invoke-NativeCapture "rustup.exe" "run 1.98.0 rustc --version"
if ($rustc.ExitCode -ne 0) {
    throw "rustup run 1.98.0 rustc --version failed with exit code $($rustc.ExitCode): $($rustc.Output)"
}
if (-not $rustc.Output.StartsWith("rustc 1.98.0 ")) {
    throw "Android builds require rustc 1.98.0, got: $($rustc.Output)"
}

$cargoNdk = Invoke-NativeCapture "rustup.exe" "run 1.98.0 cargo ndk --version"
if ($cargoNdk.ExitCode -ne 0) {
    throw "cargo-ndk probe failed with exit code $($cargoNdk.ExitCode): $($cargoNdk.Output)"
}
if (-not $cargoNdk.Output.Contains("4.1.2")) {
    throw "Android builds require cargo-ndk 4.1.2, got: $($cargoNdk.Output)"
}

Write-Host "Pinned Android Rust tooling verified: $($rustc.Output); cargo-ndk 4.1.2"
