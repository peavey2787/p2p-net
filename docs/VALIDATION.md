# Stable validation and hostile-network testing

Run the full stable validation script from the crate root.

PowerShell:

```powershell
.\scripts\run-full-validation.ps1
```

Bash:

```bash
./scripts/run-full-validation.sh
```

The script runs stable validation with DNS enabled by default through p2p-net's own startup resolver. Configured and cached `/dns`, `/dns4`, `/dns6`, and `/dnsaddr` peer addresses are resolved before dialing. Because WebSocket support in rust-libp2p 0.56 expects `libp2p-dns`, p2p-net patches that adapter to a local no-Hickory implementation and patches unused mDNS to a local no-op placeholder. Hickory DNS packages are rejected from `Cargo.lock`. `/dnsaddr` uses the configurable bounded DoH policy documented in `docs/DNSADDR_DOH.md`.

The script remains the canonical one-command runner after the profile/environment refactor. New unit tests such as `environment_detection`, `capability_resolver`, and `mediator_role` are picked up by `cargo test --workspace`, so you do not need a separate command for Phase 2, Phase 3, or Phase 4.

The script regenerates the dependency lockfile, auto-formats the tree, runs the dependency graph guard, then runs the stable checks with isolated target directories:

```text
cargo generate-lockfile
cargo fmt
cargo test --workspace --locked -j 1
cargo test --features dashboard --locked -j 1
cargo clippy --workspace --all-targets --all-features --locked -j 1 -- -D warnings
cargo audit
cargo deny check
cargo test --test multi_node_hostile --locked -j 1 -- --ignored --nocapture
```

Defaults:

- Missing `cargo-audit` and `cargo-deny` are installed automatically unless `-NoInstallTools` / `--no-install-tools` is used.
- `Cargo.lock` is regenerated first so stale lockfile entries, including old DNS resolver packages, are removed before audit.
- `rust-toolchain.toml` pins the stable toolchain, and the script rejects nightly/beta/dev rustc builds before running validation.
- Fuzz targets are included under `fuzz/`, but they are not run by the stable one-file validation script.

PowerShell options:

```powershell
.\scripts\run-full-validation.ps1 -SkipIgnored
.\scripts\run-full-validation.ps1 -NoInstallTools
.\scripts\run-full-validation.ps1 -NoClean
```

Bash options:

```bash
./scripts/run-full-validation.sh --skip-ignored
./scripts/run-full-validation.sh --no-install-tools
./scripts/run-full-validation.sh --no-clean
```


Linux packet-loss/latency simulation:

```bash
sudo ./scripts/netem-linux.sh lo start
cargo test --test multi_node_hostile -- --ignored --nocapture
sudo ./scripts/netem-linux.sh lo stop
```

`netem-linux.sh` requires Linux `tc` and root privileges. Windows should skip netem tests.
