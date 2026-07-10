// V8 (`.alcpuprofile`) output shapes + URL/quoting synthesis (algorithm spec §7, §9;
// output format spec field-by-field). Field ORDER below is the serialized
// order (`serde_json` preserves struct declaration order) and MUST match the output format spec tables
// exactly — do not reorder fields, even though it would be alphabetically tidier.
//
// Drop rules (algorithm spec §9): omit `null` (deoptReason is never modeled — always absent),
// omit a `false` bool, omit a `0` int/long. Keep `""` strings and keep `[]` arrays — those
// have no `skip_serializing_if` predicate here on purpose.

use serde::Serialize;

// `build_url` / `quote_identifier_if_needed` moved to `al_url.rs` in FT0 (Plan 2) so the
// Firefox emitter can share them; re-exported here so `v8_json::build_url` /
// `v8_json::quote_identifier_if_needed` keep resolving for existing callers (`emit::v8`)
// unchanged.
pub(crate) use crate::emit::al_url::{build_url, quote_identifier_if_needed};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuProfile {
    pub nodes: Vec<CpuProfileNode>,
    pub start_time: i64,
    pub end_time: i64,
    pub samples: Vec<i32>,
    pub time_deltas: Vec<i64>,
    pub sample_execution_times: Vec<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuProfileNode {
    pub id: i32,
    pub call_frame: CpuProfileCallFrame,
    #[serde(skip_serializing_if = "is_zero_i32")]
    pub hit_count: i32,
    pub children: Vec<i32>, // always emitted, even []
    // deoptReason: never emitted (always null in source; not modeled here at all)
    pub position_ticks: Vec<PositionTickOut>, // always emitted, even []
    pub declaring_application: DeclaringApplicationOut,
    pub application_definition: ApplicationDefinitionOut,
    #[serde(skip_serializing_if = "is_false")]
    pub is_incomplete_measurement: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub is_builtin_code_unit_call: bool,
    #[serde(skip_serializing_if = "is_zero_i32")]
    pub frame_identifier: i32,
    #[serde(skip_serializing_if = "is_zero_i64")]
    pub start_time: i64,
    #[serde(skip_serializing_if = "is_zero_i64")]
    pub end_time: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuProfileCallFrame {
    // `Option<String>` (not `String`) so the empty-sentinel call frame (algorithm spec §7.4
    // `EmptyCpuProfileCallFrame`, used when `this_frame` is absent) can OMIT these — its C#
    // fields are `null`, which serialize as absent keys, NOT as present `""`. A NORMAL frame
    // sets each to `Some(...)`, so a genuinely empty `method_name` still serializes as a present
    // `""` (unchanged from before).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "is_zero_i32")]
    pub line_number: i32,
    #[serde(skip_serializing_if = "is_zero_i32")]
    pub column_number: i32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionTickOut {
    #[serde(skip_serializing_if = "is_zero_i32")]
    pub line: i32,
    #[serde(skip_serializing_if = "is_zero_i32")]
    pub column: i32,
    #[serde(skip_serializing_if = "is_zero_i32")]
    pub ticks: i32,
    #[serde(skip_serializing_if = "is_zero_i64")]
    pub execution_time: i64,
}

/// order: objectType, objectName, objectId (algorithm spec §9). No `skip_serializing_if` anywhere —
/// this nested object is never omitted and its own fields are never dropped (algorithm spec §9: the
/// Default sentinel's `objectId == -1` is non-zero so it's kept anyway; `objectType`/`objectName`
/// are strings, which we never skip).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationDefinitionOut {
    pub object_type: String,
    pub object_name: String,
    pub object_id: i32,
}

/// order: appId, appName, appPublisher, appVersion (algorithm spec §9).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeclaringApplicationOut {
    pub app_id: String,
    pub app_name: String,
    pub app_publisher: String,
    pub app_version: String,
}

fn is_zero_i32(v: &i32) -> bool {
    *v == 0
}
fn is_zero_i64(v: &i64) -> bool {
    *v == 0
}
fn is_false(v: &bool) -> bool {
    !*v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_call_frame_omits_null_strings() {
        // algorithm spec §7.4 EmptyCpuProfileCallFrame: null strings + 0 line/column => serialize to an
        // empty object (all keys omitted), NOT present empty strings.
        let empty = CpuProfileCallFrame {
            function_name: None,
            script_id: None,
            url: None,
            line_number: 0,
            column_number: 0,
        };
        let s = serde_json::to_string(&empty).unwrap();
        assert_eq!(s, "{}");
    }
}
