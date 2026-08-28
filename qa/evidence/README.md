# Validation evidence

`run-full-validation.cmd` and `run-full-validation.sh` persist every validation run under
`qa/evidence/runs/`. Generated run directories are intentionally Git-ignored so collecting
evidence never dirties the source tree.

Each machine-captured run binds `release_input_sha256` to the source fingerprint captured before validation starts and records the post-validation fingerprint separately. This keeps the evidence attached to the code that entered validation even when validation leaves disposable untracked output. The canonical release runner performs the stricter mutation check in its clean validation worktree and fails if validation changes any tracked snapshot file.

Each machine-captured run contains:

- `transcript.txt` — complete stdout/stderr from the validation runner;
- `manifest.txt` — result, mode, timestamps, toolchain identity, Git/source identity, and hashes;
- `git-status.txt` — the source worktree state seen by the run;
- `release-inputs.txt` — the canonical build-input inventory used to bind validation to release code;
- `Cargo.lock.sha256.txt` — the exact lockfile digest;
- `PASS` or `FAIL` — a simple terminal result marker.

The canonical release-input fingerprint is SHA-256 over the normalized `git ls-tree` listing for
`Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.cargo/config.toml`, `crates/`, `apps/`,
`external/`, `examples/`, and `assets/`. The checked-in Cargo config is included because its
workspace patch paths affect dependency resolution and therefore the build graph. Documentation,
QA implementation, generated validation evidence, and release-runner implementation remain
excluded so tooling-only evidence/release fixes do not force revalidation of unchanged production
build inputs.

`qa/evidence/recovered/` contains preserved machine transcripts whose automatic evidence wrapper
metadata could not be completed at run time. Recovered records must retain the original transcript
bytes, pin the transcript SHA-256, explain the recovery reason, and reconstruct only metadata that
can be independently derived from the transcript and exact source snapshot.

`qa/evidence/attestations/` is reserved for explicitly identified historical evidence that could
not be machine-captured at the time. An attestation must never claim to contain a transcript that
does not exist. Release manifests preserve the evidence kind (`machine-captured`,
`recovered-machine-transcript`, or `user-attested`) instead of presenting them as equivalent.
