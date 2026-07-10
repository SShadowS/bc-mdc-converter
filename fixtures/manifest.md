# Fixtures manifest — BC28 instrumentation profiling corpus

Real instrumentation captures from the live **Cronus28** BC28 container (dev
`http://Cronus28:7049/BC`, snapshot `:7083`, tenant `default`, company
`CRONUS Danmark A/S`, CRONUS demo data — a throwaway container, no sensitive
content). Each fixture is a pair:

- `<name>.mdc.zip` — INPUT: the raw profiler ZIP of `<int>.mdc` `MethodContext`
  FlatBuffers (execution-ordered) + a `version` entry. This is what the Rust
  converter consumes.
- `<name>.reference.alcpuprofile` — EXPECTED OUTPUT: produced from the `.mdc.zip`
  by the official AL tooling. Valid V8 CPU profile.

## How they were captured

All captures drove the **shipped `bcdev_profile_*` tools** of bc-dev-mcp
(production path: built `dist/index.js` over MCP; the built
`dist/converter/win-x64/mdc-converter.exe` auto-runs inside `bcdev_profile_finish`,
returning both `zipPath` = the `.mdc` zip and `profilePath` = the reference
`.alcpuprofile`). Flow per fixture:

1. `bcdev_profile_start { kind:"instrumentation", server:"http://Cronus28",
   serverInstance:"BC", tenant:"default", clientType:"WebClient" }` — arms the
   sampler; `SessionId=-1` binds the **next** session on the tenant.
2. Trigger a real **WebClient session** (real AL execution) via
   business-central-mcp (`bc_open_page` / `bc_read_data` / `bc_navigate` /
   `bc_write_data` / `bc_run_report`).
3. `bcdev_profile_poll` until `Started`, then keep driving AL **during** the
   recording window (instrumentation records method-exits only while active).
4. `bcdev_profile_finish` → auto-converts to `.alcpuprofile`.

Driver: `bc-dev-mcp/scratch/capture-fixtures.mjs` (untracked). The `instr.*`
baseline predates it (captured via `scratch/instr-capture2.mjs`, Full verbosity +
sustained page bursts after `Started`).

**Span** = `(endTime − startTime) / 10_000_000` seconds, read from the
`.alcpuprofile` i64 `startTime`/`endTime` ticks with Python (native bigint — do
**not** round-trip through JS `JSON.parse`, it corrupts i64). Note: the
`bcdev_profile_finish` summary `durationMs` is a *different* JS-computed metric
(~10× the span here) — the i64 span below is authoritative.

## Table

| fixture | span (s) | .mdc entries | nodes | exceptions | object types | zip size | workload |
|---|---:|---:|---:|:---:|---|---:|---|
| `instr` (baseline) | **10.886** | 11285 | 706 | Y (35) | Codeunit, Page, Table, PageExtension, Query | 6.51 MB | Sustained WebClient page bursts (Full verbosity) after `Started`. **Primary byte-match acceptance fixture** — the README acceptance-test target (706 nodes). |
| `tiny` | 0.860 | 1639 | 492 | N (0) | Codeunit, Page, Table, PageExtension | 1.35 MB | Minimal: open Customer List (22) + header read → `Started` → open Chart of Accounts (31) + header read → finish. Edge case: fewest nodes/entries. |
| `mixed-objects` | **2.555** | 7977 | 648 | Y (7) | Codeunit, Page, Table, **Report**, PageExtension | 4.48 MB | Render Report 101 (Customer - List) as PDF, read list pages 22/31/16/27, render Report 4 (Vendor - Labels), drill into a Customer Card (page 21) + read. Only fixture besides `long-report` that spans a **Report** object — for category-coloring tests. |
| `with-exception` | 1.033 | 1477 | 470 | **Y (7)** | Codeunit, Page, Table, PageExtension | 1.33 MB | Read Vendor List (27), drill into a Customer Card and `bc_write_data` an invalid `Country/Region Code = "ZZZZ"` (raises a real AL `VALIDATION_ERROR`), read Chart of Accounts (31). Carries **7 populated `ExceptionContext`** tables (see note). |
| `long-report` | **3.461** | 1887 | 511 | N (0) | Codeunit, Page, Table, **Report**, PageExtension | 1.50 MB | **The ≥2 s hard-requirement fixture.** Report 101 (Customer - List) PDF renders interleaved with list bursts (22/31), spread with 1.5 s pauses over a ~24 s window for a long Firefox timeline + high invocation count. |

### The ≥2 s requirement

**`long-report` = 3.461 s** is the designated ≥2 s fixture (heavy report renders,
long timeline). `mixed-objects` (2.555 s) and the `instr` baseline (10.886 s) also
exceed 2 s. `tiny` (0.860 s) and `with-exception` (1.033 s) are intentionally short.

### Exception note (`with-exception`)

The 7 populated `ExceptionContext` tables carry the message *"The given key was not
present in the dictionary."* — caught **platform** exceptions raised inside the
profiled session while opening the Customer Card / validating. Verified by decoding
`MethodContext.exception` (vtable id 2 / off 8 → `ExceptionContext.message` id 0 /
off 4) directly from the `.mdc` FlatBuffers (see
`the .mdc schema spec`). The deliberately-driven `Country/Region Code
= "ZZZZ"` validation error is confirmed at the bc-mcp/client layer, but that
rolled-back UI validation error did **not** itself produce a recorded
`MethodContext.exception`; the retained `ExceptionContext`s are the platform ones
captured during the same card/validation activity. Either way the fixture carries
`ExceptionContext` (7 > 0) and exercises the converter's exception path — with
`tiny` (0) and `long-report` (0) as clean contrasts.

## V8 validity

Every `.reference.alcpuprofile` parses as a valid V8 CPU profile
(`nodes`/`samples`/`timeDeltas` all present, non-empty, equal length):

| fixture | nodes | samples | timeDeltas | valid V8 |
|---|---:|---:|---:|:---:|
| `instr` | 706 | 706 | 706 | ✔ |
| `tiny` | 492 | 492 | 492 | ✔ |
| `mixed-objects` | 648 | 648 | 648 | ✔ |
| `with-exception` | 470 | 470 | 470 | ✔ |
| `long-report` | 511 | 511 | 511 | ✔ |

`instr.*` remains the **primary byte-match acceptance fixture** (the README
acceptance test diffs against its 706 nodes). The four new pairs are additional
variety cases: different spans, object-type mixes (Report coverage in
`mixed-objects`/`long-report`), an exception-carrying case (`with-exception`), a
long timeline (`long-report`), and a minimal edge case (`tiny`).
