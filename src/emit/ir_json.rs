// ir-json emitter (ir-json format spec): lossless,
// versioned JSON export of the un-aggregated IR for downstream tooling (al-perf). The wire
// structs below are DELIBERATELY separate from `ir.rs` (spec §5.1): the wire format is the
// stable external contract; the IR stays free to refactor.
//
// Invariants owned here (spec §3.5/§3.7):
// - every per-invocation tick is rebased to t0 (JS-safe magnitude); the absolute anchor is
//   exported ONCE as the STRING `t0Ticks` (the raw i64 ~6.4e16 exceeds JS MAX_SAFE_INTEGER)
// - line/column numbers are RAW WIRE VALUES (0-based) — no +1 display shift
// - nullable fields serialize as explicit `null` (NO skip_serializing_if anywhere), so
//   consumer validation stays strict
// - `temporalParentIx` (boundary sweep) is the call tree; `v8AggregationParentIx` is the
//   Phase-2 first-writer-wins aggregation edge, exported for losslessness ONLY

use std::collections::HashMap;

use serde::Serialize;

use crate::emit::firefox::{profile_t0, sweep};
use crate::ir::{self, LineDef};
use crate::mdc::types::DeclaringApplication;
use crate::{EmitOptions, Emitter, Error, Result};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Doc {
    schema_version: u32,
    generator: Generator,
    capture: Capture,
    apps: Vec<App>,
    invocations: Vec<Inv>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Generator {
    name: &'static str,
    version: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Capture {
    platform_version: String,
    t0_ticks: String,
    start_ticks: i64,
    end_ticks: i64,
    approx_wall_clock_start: Option<String>,
    ticks_per_ms: u32,
    invocation_count: usize,
    incomplete_count: usize,
    exception_count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct App {
    id: String,
    name: String,
    publisher: String,
    version: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Inv {
    index: u32,
    object_type: Option<&'static str>,
    object_id: Option<i32>,
    object_name: Option<String>,
    method: Option<String>,
    app_ix: Option<u32>,
    start_ticks: Option<i64>,
    end_ticks: Option<i64>,
    clamped_end_ticks: Option<i64>,
    in_sweep: bool,
    self_ticks: i64,
    temporal_parent_ix: Option<u32>,
    v8_aggregation_parent_ix: Option<u32>,
    is_builtin: bool,
    is_incomplete: bool,
    called_line: Option<LineRef>,
    caller_line: Option<LineRef>,
    lines: Vec<LineHit>,
    exception: Option<Exception>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LineRef {
    object_type: &'static str,
    object_id: i32,
    line: u16,
    column: u16,
    to_line: u16,
    to_column: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LineHit {
    line: u16,
    column: u16,
    to_line: u16,
    to_column: u16,
    hits: i32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Exception {
    message: String,
    line: u16,
}

fn line_ref(ld: &LineDef) -> LineRef {
    LineRef {
        object_type: ld.id.object_type.name(),
        object_id: ld.id.number,
        line: ld.span.from.line,
        column: ld.span.from.column,
        to_line: ld.span.to.line,
        to_column: ld.span.to.column,
    }
}

/// App dedup key, identical to the Firefox emitter's resource dedup (spec §3.3,
/// `emit/firefox/build.rs::resource_key` + `has_app`): `app_id` when non-empty, else
/// `app_name` when non-empty, else the app is treated as ABSENT (`appIx` null).
fn app_key(app: &DeclaringApplication) -> Option<String> {
    if !app.app_id.is_empty() {
        Some(app.app_id.clone())
    } else if !app.app_name.is_empty() {
        Some(app.app_name.clone())
    } else {
        None
    }
}

pub struct IrJsonEmitter;

impl Emitter for IrJsonEmitter {
    fn emit(&self, profile: &ir::Profile, _opts: &EmitOptions) -> Result<Vec<u8>> {
        let t0 = profile_t0(profile);
        let sw = sweep::run_sweep(profile);

        // apps table: first-seen (execution) order over the per-invocation
        // DeclaringApplication — Profile.apps is empty on this path (spec §3.3).
        let mut apps: Vec<App> = Vec::new();
        let mut app_ix_by_key: HashMap<String, u32> = HashMap::new();
        let mut app_ix_of_inv: Vec<Option<u32>> = vec![None; profile.invocations.len()];
        for inv in &profile.invocations {
            let Some(a) = &inv.app else { continue };
            let Some(key) = app_key(a) else { continue };
            let ix = *app_ix_by_key.entry(key).or_insert_with(|| {
                apps.push(App {
                    id: a.app_id.clone(),
                    name: a.app_name.clone(),
                    publisher: a.app_publisher.clone(),
                    version: a.app_version.clone(),
                });
                (apps.len() - 1) as u32
            });
            app_ix_of_inv[inv.index as usize] = Some(ix);
        }

        let invocations: Vec<Inv> = profile
            .invocations
            .iter()
            .map(|inv| {
                let ix = inv.index as usize;
                let eff = sw.effective_end[ix];
                let raw_end = inv.span.map(|s| s.end_ticks);
                Inv {
                    index: inv.index,
                    object_type: inv.object.map(|o| o.object_type.name()),
                    object_id: inv.object.map(|o| o.number),
                    object_name: inv.this_frame.as_ref().map(|f| f.object_name.clone()),
                    method: inv.this_frame.as_ref().map(|f| f.method_name.clone()),
                    app_ix: app_ix_of_inv[ix],
                    start_ticks: inv.span.map(|s| s.start_ticks - t0),
                    end_ticks: raw_end.map(|e| e - t0),
                    clamped_end_ticks: match (eff, raw_end) {
                        (Some(e), Some(r)) if e != r => Some(e - t0),
                        _ => None,
                    },
                    in_sweep: eff.is_some(),
                    self_ticks: sw.self_ticks[ix],
                    temporal_parent_ix: sw.temporal_parent[ix],
                    v8_aggregation_parent_ix: inv.parent_ix,
                    is_builtin: inv.is_builtin,
                    is_incomplete: inv.is_incomplete,
                    called_line: inv.called_line.as_ref().map(line_ref),
                    caller_line: inv.caller_line.as_ref().map(line_ref),
                    lines: inv
                        .lines
                        .iter()
                        .map(|lh| LineHit {
                            line: lh.span.from.line,
                            column: lh.span.from.column,
                            to_line: lh.span.to.line,
                            to_column: lh.span.to.column,
                            hits: lh.hit_count,
                        })
                        .collect(),
                    exception: inv
                        .exception
                        .as_ref()
                        .map(|e| Exception { message: e.message.clone(), line: e.line }),
                }
            })
            .collect();

        let v = profile.capture.platform_version;
        let doc = Doc {
            schema_version: SCHEMA_VERSION,
            generator: Generator {
                name: "bc-mdc-converter",
                version: env!("CARGO_PKG_VERSION"),
            },
            capture: Capture {
                platform_version: format!("{}.{}.{}.{}", v.0, v.1, v.2, v.3),
                t0_ticks: t0.to_string(),
                start_ticks: profile.start_time_ticks - t0,
                end_ticks: profile.end_time_ticks - t0,
                approx_wall_clock_start: profile.capture.approx_wall_clock_start.clone(),
                ticks_per_ms: 10_000,
                invocation_count: profile.invocations.len(),
                incomplete_count: profile.invocations.iter().filter(|i| i.is_incomplete).count(),
                exception_count: profile.invocations.iter().filter(|i| i.exception.is_some()).count(),
            },
            apps,
            invocations,
        };
        serde_json::to_vec(&doc).map_err(|e| Error::Emit(e.to_string()))
    }
}
