# Changelog

## [0.2.0] - 2026-10-14
### Added
- `--format snapshot-json`: lossless JSON of a debugging snapshot (line contexts, variable trees,
  exceptions, bundled source names), streaming with per-entry (64 MB), total decompressed (512 MB)
  and output (64 MB) limits. Malformed archives are an error, never a crash.
- `--version` and `--formats`.

## [0.1.0] - 2026-07-11
- First release: `.mdc` instrumentation snapshot to `.alcpuprofile` (v8), Firefox Profiler and ir-json.
