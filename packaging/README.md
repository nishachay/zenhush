# Packaging dither

Status: TEMPLATES ONLY. No binaries have been built from these yet.

- `dither.spec` - PyInstaller spec: one-file, windowed, brand icon.
- `../.github/workflows/build.yml` - CI template: runs both test suites on
  ubuntu/windows/macos runners, then builds and uploads a per-OS binary.

A binary only counts as tested when the CI matrix (or an equivalent real
machine) has run the suites on that OS. Linux source runs are verified
locally; Windows and macOS artifacts are not built yet.
