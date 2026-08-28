# Android physical-device qualification

These scripts are deliberately **not** part of the normal root validation run.
They require a real/emulated Android device and can run for hours. They exist to
turn the Android-specific lifecycle/resource requirements into repeatable release
evidence without burning normal CI minutes.

Before running them, install the release APK, launch it once, grant the permissions
your test needs, and verify that the node reaches ONLINE.

## Resource soak

Windows PowerShell:

```powershell
.\apps\android\qa\resource-soak.ps1 -DurationMinutes 360 -SampleSeconds 30
```

Linux/macOS shell:

```bash
./apps/android/qa/resource-soak.sh 360 30
```

Use `1440` minutes for the final 24-hour mostly-idle release soak. The script
samples process PSS/RSS, thread count, file-descriptor count when `/proc` allows
it, and `dumpsys cpuinfo`. It fails if the application process disappears and
writes evidence under `apps/android/qa/evidence/` (gitignored).

A release reviewer should inspect the resulting series for a plateau after warmup.
A monotonic memory/thread/FD trend is a blocker even if the process remains alive.
CPU should settle when the network is idle rather than showing a busy polling loop.

## Lifecycle/process-death cycles

Windows:

```powershell
.\apps\android\qa\lifecycle-cycles.ps1 -Cycles 100
```

Linux/macOS:

```bash
./apps/android/qa/lifecycle-cycles.sh 100
```

This repeatedly force-stops the package, relaunches the Activity/service, and
verifies that a process comes back. Each cycle records PSS/RSS, thread count,
and file-descriptor count when `/proc` access allows it, so lifecycle leaks are
visible instead of checking only process survival. It exercises app-private
identity/cache reopening and deterministic service/task cleanup across real
process death. After the run, confirm the peer identity remained stable, verify
resource samples plateau rather than trend upward, and inspect logcat for native
aborts, ANRs, foreground-service failures, or repeated permission errors.

These scripts complement, rather than replace, Android Studio profilers and the
functional transport/network-change matrices in `../IMPLEMENTATION_PLAN.md`.
