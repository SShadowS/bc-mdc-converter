# bc-mdc-converter

Standalone Rust converter that turns a Business Central instrumentation snapshot (`.mdc` FlatBuffers zip) into a CPU profile — byte-identical to the output of the official AL tooling, with no .NET runtime required.

[![Rust](https://img.shields.io/badge/rust-2021-orange)](https://rust-lang.org)
[![Zero Runtime Dependencies](https://img.shields.io/badge/runtime%20deps-zero-green)]()
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

| Metric | Value |
|--------|-------|
| Language | Rust (edition 2021) |
| Runtime dependencies | Zero (no .NET, no `dotnet`, no `flatc`) |
| Build dependencies | 3 crates (`zip`, `serde`, `serde_json`) |
| Binary size | ~556 KB (release, Windows) |
| Output formats | 3 (`v8`, `firefox`, `ir-json`) |
| Tests | 64 (incl. byte-identity acceptance against 5 real fixtures) |

It replaces the .NET shim that `bc-dev-mcp` shells out to for `bcdev_profile { kind:"instrumentation" }` — turning instrumentation profiling into a single dependency-free binary.

## Features

| Feature | Description |
|---------|-------------|
| **Faithful V8 output** | `.alcpuprofile` (V8 / Chrome-DevTools format + BC extensions), byte-identical to the official AL tooling's output for all 5 fixtures |
| **Firefox Profiler output** | Processed profile (`preprocessedProfileVersion: 54`) that loads directly at [profiler.firefox.com](https://profiler.firefox.com) — richer view with markers and categories |
| **Lossless IR export** | `--format ir-json` writes the un-aggregated IR (`schemaVersion: 1`) as the interchange contract for downstream tooling (al-perf) |
| **Time-exact Firefox samples** | Each invocation's exact `[start, end]` span becomes a duration-weighted sample stream (`weightType: "tracing-ms"`) — flame graph shows real ms, no resampling |
| **Categories + resources** | Object-type categories (`Codeunit`, `Page`, `Table`, …) plus a per-app resource table, so Firefox collapse/focus transforms work out of the box |
| **Per-call markers** | `BcCall` marker per invocation (nested waterfall in Marker Chart), `BcException` markers at exact throw points |
| **Hand-rolled decoder** | FlatBuffers reading implemented directly against the documented schema — no `flatc`, no codegen, append-only/stable across BC versions |

## Prerequisites

- Rust toolchain (edition 2021 or later) — build only; the produced binary is standalone

## Installation

Prebuilt binaries for Windows and Linux (static musl) are on the [Releases](https://github.com/SShadowS/bc-mdc-converter/releases) page. Or build from source:

```
cargo build --release
```

Binary lands at `target/release/bc-mdc-converter`. Run tests (including the byte-identity acceptance suite) with `cargo test`.

## Usage

```
bc-mdc-converter <in.mdc.zip> <out> [--format v8|firefox|ir-json]
```

Example:

```
bc-mdc-converter fixtures/instr.mdc.zip out.alcpuprofile
```

| Format | Output | Validity bar |
|--------|--------|--------------|
| `v8` (default) | `.alcpuprofile` — V8 / Chrome-DevTools CPU profile JSON with BC extensions, written as raw bytes (no BOM, no trailing newline) | Byte-identical to the official AL tooling's output for all 5 fixtures |
| `firefox` | Firefox Profiler processed profile — drag-and-drop into [profiler.firefox.com](https://profiler.firefox.com) | Loads correctly in the profiler (not byte-referenced against a reference fixture) |
| `ir-json` | Lossless, versioned JSON export of the un-aggregated IR — one row per invocation with rebased tick spans, temporal call tree (`temporalParentIx`), exact self time (`selfTicks`), per-statement line hits, exceptions. Suggested extension: `.ir.json` | Versioned schema (`schemaVersion: 1`); emitter source `src/emit/ir_json.rs` |

- Default format is `v8` regardless of `<out>`'s extension.
- Exit code `0` on success; any error (bad args, missing/malformed input, unrecognized `--format`) prints a message to stderr plus the usage line and exits non-zero. `-h` / `--help` prints usage and exits `0`.
- SQL note (`firefox`): the `.mdc` snapshot carries no SQL data, so no `BcSql` markers and no `counters` are emitted — the channel is reserved for a future source. Do not expect SQL timelines.

## Architecture

Decode once, then emit into any format from a shared format-neutral IR:

```
<in.mdc.zip>
    |
    v
zip reader ── per-call .mdc entries
    |
    v
FlatBuffers decoder (hand-rolled, src/mdc/)
    |
    v
format-neutral IR (src/ir.rs, src/build_ir.rs)
    |
    +-- V8 emitter ────── AggregateFrames --> .alcpuprofile  (byte-identical)
    +-- Firefox emitter ─ boundary sweep ──> processed profile (markers, categories)
    +-- IR-JSON emitter ─ lossless export ─> .ir.json         (schemaVersion 1)
```

`AggregateFrames` (the hard phase) lives only in the V8 emitter; the other formats consume the raw un-aggregated IR.

## Why

BC's instrumentation profiler records a zip of per-call `.mdc` FlatBuffers, but the only shipped way to turn one into a `.alcpuprofile` lives inside VS Code's AL language server — not exposed by any CLI. This project is an independent, dependency-free implementation of that conversion, validated byte-for-byte against real captures.

## Acceptance testing

`cargo test` runs the acceptance suite: convert each fixture zip in `fixtures/` and compare byte-for-byte against the reference `.alcpuprofile` produced by the official AL tooling from the same input. The strongest correctness gate: `Σ node.hitCount == number of .mdc entries`.

| Fixture | Content |
|---------|---------|
| `instr` | Real BC28 capture, 11,285 `.mdc` entries, 706 nodes |
| `tiny` | Minimal capture |
| `with-exception` | Capture containing exceptions |
| `mixed-objects` | Multiple object types |
| `long-report` | Long-running report |

Reference outputs were generated with the official AL tooling from the same inputs. See `fixtures/manifest.md` for details.

## Key Files

| File | Purpose |
|------|---------|
| `src/main.rs` | CLI entry point and argument parsing |
| `src/lib.rs` | Library root wiring the pipeline |
| `src/zip.rs` | Snapshot zip reading |
| `src/mdc/` | `.mdc` FlatBuffers schema + hand-rolled decoder |
| `src/ir.rs`, `src/build_ir.rs` | Format-neutral IR and its construction |
| `src/emit/v8.rs`, `src/emit/v8_aggregate.rs` | V8 emitter incl. `AggregateFrames` |
| `src/emit/firefox/` | Firefox Profiler emitter |
| `src/emit/ir_json.rs` | Lossless IR JSON emitter |
| `tests/acceptance.rs` | Byte-identity acceptance suite against `fixtures/` |

## Integrating back into bc-dev-mcp

`bc-dev-mcp` swaps its .NET shim (`tools/mdc-converter/`) for this binary: point `converter.ts`'s shim discovery at this build, drop the al-tool-DLL + `dotnet` runtime dependency and the per-RID .NET publish. Sampling is unaffected (it never needed conversion).

---

**Author**: Torben Leth (sshadows@sshadows.dk)
**License**: MIT (see [LICENSE](LICENSE))
