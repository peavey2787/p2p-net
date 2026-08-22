# CI and validation helpers

This folder contains the audit/deny policy files used by the canonical root-level launchers:

- `run-full-validation.cmd` on Windows
- `run-full-validation.sh` on Linux

The launchers intentionally live at the repository root so local validation can be started directly without navigating into `qa/ci/`.
