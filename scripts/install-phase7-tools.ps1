$ErrorActionPreference = "Stop"

Write-Host "Installing cargo-audit with locked dependencies..."
cargo install cargo-audit --locked

Write-Host "Installing cargo-deny with locked dependencies..."
cargo install cargo-deny --locked

Write-Host ""
Write-Host "Installed stable-compatible validation tools." -ForegroundColor Green
Write-Host "cargo-fuzz is intentionally not installed by this stable-tooling script."
