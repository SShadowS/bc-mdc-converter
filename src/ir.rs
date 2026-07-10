// Format-neutral intermediate representation for BC MDC → V8 alcpuprofile conversion.
// This is the IR contract that emitters depend on.

use crate::mdc::schema::StackFrame;
use crate::mdc::types::{ApplicationObjectId, SourceSpan, DeclaringApplication};

/// Full statement-line key (object id + span). Emitters key equality on (from.line, id).
#[derive(Clone, Copy, Debug)]
pub struct LineDef {
    pub id: ApplicationObjectId,
    pub span: SourceSpan,
}

/// Statement-line key used for ALL HashMap lookups keyed on a statement. Equality/hash use
/// ONLY `(from.line, id)` (algorithm spec §2.1) — column and the `to` position are deliberately
/// ignored, which is what makes a loop body (which re-hits the same lines) collapse to one
/// key. Shared by `build_ir` (Phase 2) and `emit::v8` (Phase 3) so both key on one definition.
///
/// `pub(crate)` — an entirely internal type. Nothing outside the crate ever names, builds, or
/// inspects a `LineKey`; the V8 working state that keys a map on it exposes only opaque
/// accessor methods (`V8State::tick_count`/`tick_values`), never the map itself.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct LineKey(u16, ApplicationObjectId); // (from.line, id) — the ONLY equality fields (§2.1)
impl LineKey {
    pub(crate) fn of(ld: &LineDef) -> LineKey {
        LineKey(ld.span.from.line, ld.id)
    }
}

/// Loose line equality: both None, or equal (from.line, id) (§2.1). Column and To ignored.
pub(crate) fn line_opt_eq(a: Option<LineDef>, b: Option<LineDef>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => x.span.from.line == y.span.from.line && x.id == y.id,
        _ => false,
    }
}

/// §2.5 CreateLineDefinitionFromFrame: None if either the frame's id or its statement span
/// is absent.
pub(crate) fn line_def_from_frame(f: &StackFrame) -> Option<LineDef> {
    Some(LineDef { id: f.id?, span: f.statement_span? })
}

/// One executed statement line within an invocation (raw, unpruned, encounter order).
#[derive(Clone, Copy, Debug)]
pub struct LineHit {
    pub span: SourceSpan,
    pub hit_count: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct Span {
    pub start_ticks: i64,
    pub end_ticks: i64,
}

#[derive(Clone, Debug)]
pub struct FrameInfo {
    // = mc.frames[0] (the executed frame)
    pub method_name: String,
    pub id: Option<ApplicationObjectId>,
    pub object_name: String,
    pub statement_span: Option<SourceSpan>,
}

#[derive(Clone, Debug, Default)]
pub struct AppInfo {
    pub app: DeclaringApplication, // deduped list slot
}

#[derive(Clone, Debug)]
pub struct Exception {
    pub message: String,
    pub line: u16,
}

#[derive(Clone, Debug)]
pub struct SqlEvent; // reserved extension point — never populated in Plan 1

#[derive(Clone, Debug)]
pub struct Invocation {
    pub index: u32,                          // ExecutionIndex == .mdc ordinal
    pub object: Option<ApplicationObjectId>, // mc.Id (None => no V8 frame, still consumes an index)
    pub this_frame: Option<FrameInfo>,       // mc.frames[0]
    pub app: Option<DeclaringApplication>,   // mc.App
    pub span: Option<Span>,                  // mc.ExecutionSpan
    pub is_builtin: bool,
    pub is_incomplete: bool,
    pub parent_ix: Option<u32>,              // caller invocation index
    pub child_ixs: Vec<u32>,                 // callee invocation indices, ascending
    pub caller_line: Option<LineDef>,        // full key (from frames[1], strict)
    pub called_line: Option<LineDef>,        // full key (from frames[0])
    pub lines: Vec<LineHit>,                 // raw unpruned line hits, encounter order
    pub exception: Option<Exception>,
    pub sql: Vec<SqlEvent>,                  // always empty in Plan 1
}

/// Capture-level metadata decoded from the snapshot ZIP itself (NOT from the .mdc
/// FlatBuffers): the `version` entry and the earliest zip-entry DOS mod time. Carried on
/// the IR so emitters that need it (ir-json, spec §3.2) can reach it without re-reading
/// the zip. `Default` (zeros/None) when a Profile is built directly from contexts
/// (`build_profile`, tests); `convert()` overwrites it from the decoded snapshot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CaptureMeta {
    pub platform_version: (i32, i32, i32, i32),
    pub approx_wall_clock_start: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Profile {
    pub start_time_ticks: i64,               // invocations[0].span.start (NOT min)
    pub end_time_ticks: i64,                 // invocations[last].span.end (NOT max) — algorithm spec §7.1
    pub invocations: Vec<Invocation>,
    pub apps: Vec<AppInfo>,
    pub capture: CaptureMeta,                // zip-level metadata; Default unless set by convert()
}
