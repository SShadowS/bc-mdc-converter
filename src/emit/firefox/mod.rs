// Firefox Profiler emitter scaffold (Plan 2). The V8 path (`emit::v8`) is unaffected — this
// module is purely additive on the same format-neutral IR (`crate::ir::Profile`). Filled in
// incrementally across FT0-FT5 (Firefox emitter spec); FT0 only wires the module
// tree + the tick/line/t0 conversion helpers shared by everything that follows.

// `model`/`build`/`sweep`/`markers` are `pub` (not `pub(crate)`) from FT2/FT3/FT4 on: the
// integration test (`tests/firefox.rs`) calls `build::build_tables` / `sweep::build_samples` /
// `markers::build_markers` directly and reads their results' fields, ahead of
// `FirefoxEmitter::emit` being wired end-to-end at FT5 (Global Constraint "test visibility
// convention", Firefox emitter plan).
pub mod model;
pub mod build;
pub mod sweep;
pub mod markers;

use crate::{ir, EmitOptions, Emitter, Error, Result};

/// 10,000 ticks = 1 ms (100 ns ticks, Firefox emitter spec §7.2). f64, never pre-round.
pub fn tick_to_ms(t: i64, t0: i64) -> f64 {
    (t - t0) as f64 / 10000.0
}

/// Firefox is editor-facing: wire line -> 1-based display (Firefox emitter spec §7.3).
/// NOTE: .mdc schema spec §4 calls the wire "1-based"; verify at the browser-load gate. One helper = flippable.
pub fn wire_line_to_display(line: u16) -> u32 {
    line as u32 + 1
}

/// Profile zero = earliest start tick over all invocations (Firefox emitter spec §1.3).
pub fn profile_t0(profile: &ir::Profile) -> i64 {
    profile.invocations.iter().filter_map(|i| i.span).map(|s| s.start_ticks).min().unwrap_or(0)
}

/// Firefox Profiler emitter (Firefox emitter spec §5): assembles the tables built
/// by `build`/`sweep`/`markers` into one `Thread` + `meta` + top-level `FfProfile` and
/// serializes.
pub struct FirefoxEmitter;
impl Emitter for FirefoxEmitter {
    fn emit(&self, profile: &ir::Profile, opts: &EmitOptions) -> Result<Vec<u8>> {
        let t0 = profile_t0(profile);

        // ONE boundary sweep (Firefox emitter spec §1.3/§7.1) drives the call tree AND the self-times: its
        // `temporal_parent` sets the stackTable prefixes (the TRUE temporal edges, not the IR's
        // V8-aggregation `parent_ix`), its `self_ticks` are the exact non-negative self-times, and
        // its sub-intervals become the sample stream once the stack nodes exist.
        let sweep = sweep::run_sweep(profile);
        let mut tables = build::build_tables_with_parents(profile, opts, &sweep.temporal_parent);

        // selfMs = exact self time from the sweep (§7.1): the sum of sub-intervals where the
        // invocation is the leaf. Non-negative by construction, replacing the old
        // `span − Σ aggregation-child durations` that over-counted looped children (negative selfMs).
        let self_ms: Vec<f64> = sweep.self_ticks.iter().map(|&t| t as f64 / 10000.0).collect();
        let samples = sweep.build_samples(&tables.stack_node_of_invocation, t0);
        // IMPORTANT ordering (Firefox emitter spec §5.3): `build_markers` interns marker names into the SAME
        // interner `build_tables` already populated. It MUST run before `tables.interner.strings`
        // is taken (below) as the thread's final `stringArray` — every string index (func names,
        // marker names, URLs, resource names) must resolve against one shared, complete array.
        let markers = markers::build_markers(profile, &mut tables.interner, &self_ms);

        let thread = model::Thread {
            name: "AL Session".to_string(),
            is_main_thread: true,
            process_type: "default".to_string(),
            pid: "0".to_string(),
            tid: 0,
            // Firefox emitter spec §5.2 — REQUIRED, or BcCall/BcException markers never show in the timeline
            // header (Gecko otherwise restricts `timeline-overview` display to named threads).
            show_markers_in_timeline: true,
            // These + `native_symbols` (below) are REQUIRED by the browser's v54->66 upgrader
            // chain (upgrader #56 reads `nativeSymbols.length`); their absence makes
            // profiler.firefox.com reject the profile. The two Option scalars serialize as `null`.
            register_time: 0.0,
            process_startup_time: 0.0,
            unregister_time: None,
            process_shutdown_time: None,
            paused_ranges: vec![],
            func_table: tables.funcs,
            frame_table: tables.frames,
            stack_table: tables.stacks,
            resource_table: tables.resources,
            native_symbols: model::NativeSymbolTable::default(),
            samples,
            markers,
            string_array: tables.interner.strings,
        };

        let meta = build::build_meta(profile, t0);
        let ff = model::FfProfile { meta, libs: tables.libs, threads: vec![thread] };

        serde_json::to_vec(&ff).map_err(|e| Error::Emit(e.to_string()))
    }
}
