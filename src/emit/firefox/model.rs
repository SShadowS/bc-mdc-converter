// Firefox Profiler v54 struct-of-arrays table types (Firefox emitter spec
// §2.1-§2.4, Firefox format spec §A.3/§A.5). Every table is
// struct-of-arrays: parallel `Vec<_>` columns of the same length (`length`), row `i` being
// column-`k`[i]. Filled from the IR by `build.rs`.
//
// All types/fields here are `pub` (not `pub(crate)`) per the Global Constraint "test
// visibility convention" (Firefox emitter plan): the
// FT2 integration test (`tests/firefox.rs`, an external crate) calls `build_tables` and reads
// these fields directly, ahead of `FirefoxEmitter::emit` being wired at FT5.

use serde::Serialize;
use serde_json::json;

/// `meta.categories[i]` (Firefox emitter spec §2.1). `color` must be one of the exact Firefox
/// `CategoryColor` union values; `subcategories[0]` must be `"Other"`.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    pub name: String,
    pub color: String,
    pub subcategories: Vec<String>,
}

/// Fixed BC category list (Firefox emitter spec §2.1 table), `(name, color)`. Index 0 is the required grey
/// default. Index 9 (`Exception`) is marker-only (§3.2) — `category_for` (build.rs) never
/// returns it; no `frameTable`/`stackTable` row ever points at it.
pub const CATEGORIES: &[(&str, &str)] = &[
    ("Other", "grey"),                   // 0
    ("Codeunit", "blue"),                // 1
    ("Table", "orange"),                 // 2
    ("Page", "green"),                   // 3
    ("Report", "purple"),                // 4
    ("Query", "magenta"),                // 5
    ("XmlPort", "yellow"),               // 6
    ("Codeunit (built-in)", "lightblue"),// 7
    ("System", "darkgrey"),              // 8
    ("Exception", "red"),                // 9
];

/// Shared subcategory vocabulary across every category (Firefox emitter spec §2.2). `[0]` must be `"Other"`.
pub const SUBCATEGORIES: &[&str] = &["Other", "Trigger", "Procedure", "Built-in"];

/// One row per distinct `(object_type, object_id, method_name)` (Firefox emitter spec §2.4). A loop calling
/// the same method 10,000 times is ONE func.
#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct FuncTable {
    pub name: Vec<u32>, // -> stringArray
    #[serde(rename = "isJS")]
    pub is_js: Vec<bool>,
    #[serde(rename = "relevantForJS")]
    pub relevant_for_js: Vec<bool>,
    pub resource: Vec<i32>,  // -> resourceTable, or -1
    pub file_name: Vec<u32>, // -> stringArray (al-preview:// URL)
    pub line_number: Vec<u32>,
    pub column_number: Vec<u32>,
    pub length: usize,
}

/// One row per distinct `(func, called_line)` executed context (Firefox emitter spec §2.4). Different call
/// sites / executed lines of the same func are different frames but the same func.
#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct FrameTable {
    pub func: Vec<u32>,        // -> funcTable
    pub address: Vec<i32>,     // always -1 (non-native)
    pub inline_depth: Vec<u32>, // always 0 (v40+)
    pub category: Vec<u32>,    // -> meta.categories
    pub subcategory: Vec<u32>, // -> category's subcategories
    pub native_symbol: Vec<Option<u32>>, // always null
    #[serde(rename = "innerWindowID")]
    pub inner_window_id: Vec<Option<u32>>, // always null
    pub line: Vec<u32>,
    pub column: Vec<u32>,
    pub length: usize,
}

/// The call tree: one row per invocation, NOT per func (Firefox emitter spec §2.4) — the sample stream
/// references invocation-level leaves so recursive/loop calls appear at each depth they
/// occurred. Topologically ordered: every `prefix` is a strictly-lower index than its own row
/// (parents always precede children).
#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct StackTable {
    pub frame: Vec<u32>,          // -> frameTable
    pub prefix: Vec<Option<u32>>, // -> this table (parent node), or null at a root
    pub category: Vec<u32>,
    pub subcategory: Vec<u32>,
    pub length: usize,
}

/// One row per distinct declaring app (Firefox emitter spec §2.3). `type` is always `1` (Library).
#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResourceTable {
    pub lib: Vec<u32>,          // -> top-level `libs`
    pub name: Vec<u32>,         // -> stringArray (app display name)
    pub host: Vec<Option<u32>>, // always null
    #[serde(rename = "type")]
    pub resource_type: Vec<u8>, // always 1 (ResourceType::Library)
    pub length: usize,
}

/// One `Lib` per declaring app (Firefox emitter spec §2.3), lives in the top-level `Profile.libs` array.
/// Only `name`/`debugName`/`breakpadId` carry real content — nothing symbolicates.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Lib {
    pub arch: String,
    pub name: String,
    pub path: String,
    pub debug_name: String,
    pub debug_path: String,
    pub breakpad_id: String,
    pub code_id: Option<String>,
}

/// The sample stream (Firefox emitter spec §1.2/§1.3) — one row per boundary-sweep sub-interval, produced by
/// `sweep::build_samples`. This is the CORE mechanism (§0): a time-exact, duration-weighted
/// stream so self-time falls out exactly (§7.1), unlike a count-based `'samples'` weight type.
#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SamplesTable {
    pub stack: Vec<Option<u32>>, // -> stackTable, or null for an idle gap
    pub time: Vec<f64>,          // sub-interval start, ms (f64), (b_k - t0) / 10000.0
    pub weight: Vec<f64>,        // sub-interval duration, ms (f64), (b_{k+1} - b_k) / 10000.0
    pub weight_type: String,     // always "tracing-ms" (Firefox emitter spec §1.1/§1.2)
    pub length: usize,
}

/// The execution waterfall: `RawMarkerTable` (Firefox emitter spec §3, `profile.ts:215-226`),
/// struct-of-arrays, one row per marker. `BcCall` is an INTERVAL (phase 1) marker per invocation
/// (§3.1); `BcException` is an INSTANT (phase 0) marker per throw (§3.2); zero `BcSql` markers
/// are ever produced for the `.mdc` path, though the schema is registered (§3.3). `category` is
/// required/non-null on every row (never inherited, unlike `frameTable.category`).
#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct MarkerTable {
    pub name: Vec<u32>, // -> stringArray
    pub start_time: Vec<Option<f64>>,
    pub end_time: Vec<Option<f64>>,
    pub phase: Vec<u8>,     // 0 INSTANT, 1 INTERVAL (Firefox emitter spec §3, constants.ts:110-113)
    pub category: Vec<u32>, // -> meta.categories; required, non-null
    pub data: Vec<serde_json::Value>,
    pub length: usize,
}

/// `meta.markerSchema` (Firefox emitter spec §3.1/§3.2/§3.3), verbatim. At v54 the field-list key is `data`
/// (v55 renames it to `fields`; the browser's on-load upgrader chain migrates it, Firefox emitter spec §3.1).
/// Schema lookup is purely by `data.type` since v52 (the marker `name` is not used for lookup).
pub fn marker_schema() -> Vec<serde_json::Value> {
    vec![
        json!({
            "name": "BcCall",
            "tooltipLabel": "{marker.data.object}.{marker.data.method}",
            "tableLabel": "{marker.data.object}.{marker.data.method} — {marker.data.durationMs}",
            "chartLabel": "{marker.data.method}",
            "display": ["marker-chart", "marker-table", "timeline-overview"],
            "isStackBased": true,
            "data": [
                { "key": "object", "label": "Object", "format": "string" },
                { "key": "objectType", "label": "Type", "format": "string" },
                { "key": "objectId", "label": "Object ID", "format": "integer" },
                { "key": "app", "label": "App", "format": "string" },
                { "key": "method", "label": "Method", "format": "string" },
                { "key": "line", "label": "Called line", "format": "integer" },
                { "key": "durationMs", "label": "Duration", "format": "duration" },
                { "key": "selfMs", "label": "Self", "format": "duration" },
                { "key": "hitCount", "label": "Hits", "format": "integer" },
                { "key": "incomplete", "label": "Incomplete", "format": "string" }
            ]
        }),
        json!({
            "name": "BcException",
            "tooltipLabel": "{marker.data.object}.{marker.data.method}: {marker.data.message}",
            "tableLabel": "{marker.data.message}",
            "display": ["marker-chart", "marker-table", "timeline-overview"],
            "data": [
                { "key": "message", "label": "Message", "format": "string" },
                { "key": "line", "label": "Line", "format": "integer" },
                { "key": "object", "label": "Object", "format": "string" },
                { "key": "method", "label": "Method", "format": "string" }
            ]
        }),
        json!({
            "name": "BcSql",
            "tooltipLabel": "{marker.data.statement}",
            "tableLabel": "SQL ({marker.data.durationMs}) — {marker.data.rows} rows",
            "display": ["marker-chart", "marker-table"],
            "data": [
                { "key": "statement", "label": "Statement", "format": "sanitized-string" },
                { "key": "durationMs", "label": "Duration", "format": "duration" },
                { "key": "rows", "label": "Rows read", "format": "integer" }
            ]
        }),
    ]
}

/// `meta` assembly (Firefox emitter spec §5.1, Firefox format spec §A.2), built by `build::build_meta`. `start_time` is a
/// **documented deviation** from Firefox emitter spec §7.2's `.NET-epoch` formula: the `.mdc`'s earliest tick
/// (`profile_t0`) is an opaque instrumentation counter, not a real wall-clock-anchored
/// `DateTime.Ticks` value — subtracting the .NET-epoch-to-Unix-epoch offset
/// (621355968000000000 ticks, Firefox emitter spec §7.2) from it goes NEGATIVE for every fixture we have. We
/// emit `0.0` and let `end_time` (already relative to `t0`) carry the real profile duration;
/// nothing downstream needs an absolute wall-clock anchor. `sampleUnits` is intentionally
/// OMITTED (Firefox emitter spec §5.1 — `weightType: 'tracing-ms'` already drives ms display; setting a wrong
/// unit would be worse than omitting the field).
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    pub version: u32, // 34 == GECKO_PROFILE_VERSION
    pub preprocessed_profile_version: u32, // 54 == the classic per-thread shape we emit (06 §A.1)
    pub interval: u32, // nominal; instrumentation isn't sampled. 1 ms.
    pub start_time: f64, // ALWAYS 0.0 — see doc-comment above
    pub end_time: f64,   // tick_to_ms(profile.end_time_ticks, t0)
    pub process_type: u32,
    pub stackwalk: u32,
    pub symbolicated: bool,
    pub product: String,
    pub categories: Vec<Category>,
    pub marker_schema: Vec<serde_json::Value>,
    pub keep_profile_thread_order: bool,
}

/// Empty native-symbol table (Firefox format spec §A.3 omitted this — an incomplete field list). REQUIRED
/// even though it is always empty for an AL profile: the browser's v54->66 upgrader chain reads
/// `thread.nativeSymbols.length` (upgrader #56), so its absence makes profiler.firefox.com reject
/// the profile with "Cannot read properties of undefined (reading 'length')". Struct-of-arrays,
/// always length 0 for the `.mdc` path (nothing symbolicates).
#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct NativeSymbolTable {
    pub lib_index: Vec<u32>,
    pub address: Vec<i64>,
    pub name: Vec<u32>,
    pub function_size: Vec<Option<i64>>,
    pub length: usize,
}

/// One thread per capture (Firefox emitter spec §5.2 — multi-activity -> multi-thread is additive, not built
/// yet). `show_markers_in_timeline: true` is REQUIRED, not merely nice-to-have: for an
/// imported/synthesized thread, Gecko's `timeline-overview` marker display only activates when
/// the thread opts in with this flag; without it, `BcCall`/`BcException` markers silently fail
/// to appear in the timeline header even though they still render fine in the Marker
/// Chart/Table. All tables live on the thread at v54 (they move to `profile.shared` at v56;
/// Firefox format spec §A.1).
///
/// `native_symbols` + the time scalars (`register_time`/`process_startup_time`/
/// `unregister_time`/`process_shutdown_time`) + `paused_ranges` are all REQUIRED by the browser's
/// v54->66 upgrader chain (Firefox format spec §A.3's Thread list was incomplete — confirmed at the
/// browser-load gate). The two `Option<f64>` scalars must serialize as JSON `null` (present, not
/// omitted) — so they carry NO `skip_serializing_if`; `paused_ranges` must serialize as `[]`;
/// `native_symbols` as the empty table (upgrader #56 reads its `.length`).
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub name: String,
    pub is_main_thread: bool,
    pub process_type: String,
    pub pid: String, // string since v47 (Firefox format spec §A.1)
    pub tid: u32,
    pub show_markers_in_timeline: bool,
    pub register_time: f64,
    pub process_startup_time: f64,
    pub unregister_time: Option<f64>, // emit `null` (no skip_serializing_if)
    pub process_shutdown_time: Option<f64>, // emit `null` (no skip_serializing_if)
    pub paused_ranges: Vec<serde_json::Value>, // emit `[]`
    pub func_table: FuncTable,
    pub frame_table: FrameTable,
    pub stack_table: StackTable,
    pub resource_table: ResourceTable,
    pub native_symbols: NativeSymbolTable,
    pub samples: SamplesTable,
    pub markers: MarkerTable,
    pub string_array: Vec<String>,
}

/// Top-level processed profile (Firefox format spec §A.2). `counters` is deliberately ABSENT — not
/// `Option`/`skip_serializing_if`, the field does not exist on this struct at all — because the
/// honest SQL stance (Firefox format spec Part B) is that `.mdc` carries no SQL/counter-worthy time series,
/// so there is nothing to reserve a null/empty value for. `threads` is a `Vec` (one thread today;
/// a multi-activity capture would add one thread per activity, Firefox emitter spec §5.2, without changing
/// this shape).
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FfProfile {
    pub meta: Meta,
    pub libs: Vec<Lib>,
    pub threads: Vec<Thread>,
}
