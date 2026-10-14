# Fixtures — ground-truth instrumentation corpus

5 real BC28 instrumentation captures from the live Cronus28 container, each a pair:
`<name>.mdc.zip` (INPUT — the profiler zip of `<int>.mdc` FlatBuffers the converter
consumes) + `<name>.reference.alcpuprofile` (EXPECTED — output of the official AL
tooling, valid V8). See **`manifest.md`** for the full table (workload, span, counts).

| fixture | span s | `.mdc` | nodes | exc | notable |
|---|--:|--:|--:|:-:|---|
| `instr` (baseline) | 10.886 | 11285 | 706 | Y(35) | **primary byte-match acceptance fixture** (the acceptance-test target) |
| `tiny` | 0.860 | 1639 | 492 | N | smallest — edge case, few nodes |
| `with-exception` | 1.033 | 1477 | 470 | Y(7) | exercises `ExceptionContext` |
| `mixed-objects` | 2.555 | 7977 | 648 | Y(7) | spans a Report object; ≥2s |
| `long-report` | 3.461 | 1887 | 511 | N | the designated **≥2s** run; long timeline |

`tiny`/`long-report` (0 exceptions) are clean contrasts to the exception-bearing ones.
`long-report`/`mixed-objects`/`instr` all exceed 2s (long timelines → Firefox timeline
+ Rust-perf stress). Object types across the corpus: Codeunit, Page, Table, PageExt,
Query, Report.

## Acceptance test

`instr` is the **primary byte-match target** — a faithful V8 emitter must structurally
reproduce `instr.reference.alcpuprofile` (706 nodes; `Σ node.hitCount == 11,285` ==
`.mdc` count — the strongest gate). The other four are **variety/regression fixtures**:
run the converter over each and assert its own reference (node count, hitCount
conservation, samples/timeDeltas lengths, callFrame url grammar, exception handling
for the `Y` rows). MAY differ: JSON key order, whitespace, tie ordering, `frameIdentifier`
until pinned.

```
converter <name>.mdc.zip out.alcpuprofile   # then structural-diff vs <name>.reference.alcpuprofile
```

## Notes

- **Spans** are `(endTime − startTime)/10_000_000` s from the i64 `startTime`/`endTime`
  ticks — read with a bigint-safe parser (NOT JS `JSON.parse`, it corrupts i64).
- `with-exception` carries 7 caught **platform** `ExceptionContext`s ("key not present
  in the dictionary") from a validation session, byte-verified in the `.mdc` `exception`
  slot. A rolled-back UI `VALIDATION_ERROR` did NOT record as a `MethodContext.exception`
  — documented in `manifest.md`.
- All from a throwaway CRONUS demo container — no sensitive content. Capture more via
  bc-dev-mcp `bcdev_profile { kind:"instrumentation" }` against Cronus28 if needed.

## snapshot-page-exception.zip
Debugging snapshot (ExecutionContext=Debugging, SnapshotVerbosity=SnapPoint) from Cronus29 (BC29, runtime 18.0), 2026-10-14:
Trigger Zoo page 50152 opened from a WebClient session, snappoint on line 35 (wire line 34). Entries: `228.mdc`,
`Page%50152.al`, `version`. One method (`OnOpenPage`), line contexts [0] span 34:8-34:44 (`PageLocalText = ''`)
and [1] span 35:8-35:39 (`PageLocalText = 'page trigger zoo'`), exception `page OnOpenPage break` with line_number 1.
