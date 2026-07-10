// Snapshot decode orchestration: read the zip, gate on version, select and
// numerically order `.mdc` entries (execution order), decode each.

use crate::mdc::schema::{decode_method_context, decode_version, MethodContext};
use crate::zip::read_zip_entries;
use crate::{Error, Result};

pub struct DecodedSnapshot {
    pub contexts: Vec<MethodContext>,
    /// Full 4-part `version` entry (decode_version); (0,0,0,0) if the entry is absent.
    pub platform_version: (i32, i32, i32, i32),
    /// Earliest zip-entry mtime (ISO local time, no timezone), lexicographic min —
    /// fixed-width format makes lexicographic == chronological.
    pub earliest_mtime_iso: Option<String>,
}

pub fn decode_snapshot(zip_bytes: &[u8]) -> Result<DecodedSnapshot> {
    let entries = read_zip_entries(zip_bytes)?;

    // Version gate (algorithm spec §1). Absent version => 0.0.0.0 => fails the >=2.0 gate,
    // but the profiler always writes one; missing entry is treated as (0,0).
    let platform_version = entries.iter().find(|e| e.name == "version")
        .map(|e| decode_version(&e.data))
        .unwrap_or((0, 0, 0, 0));
    let (maj, min) = (platform_version.0, platform_version.1);
    if (maj, min) < (2, 0) { return Err(Error::UnsupportedVersion { got: (maj, min) }); }
    let earliest_mtime_iso = entries.iter().filter_map(|e| e.mtime_iso.clone()).min();

    // Collect .mdc entries with numeric stem, sort ascending (numeric, not lexicographic).
    let mut mdc: Vec<(i64, &[u8])> = Vec::new();
    for e in &entries {
        let lower = e.name.to_ascii_lowercase();
        if let Some(stem) = lower.strip_suffix(".mdc") {
            if let Ok(n) = stem.parse::<i64>() {
                if !e.data.is_empty() { mdc.push((n, &e.data)); } // skip empty (.mdc schema spec §2)
            }
        }
    }
    if mdc.is_empty() { return Err(Error::NoMethodContexts); }
    mdc.sort_by_key(|(n, _)| *n);

    let mut contexts = Vec::with_capacity(mdc.len());
    for (_, data) in mdc { contexts.push(decode_method_context(data)?); }
    Ok(DecodedSnapshot { contexts, platform_version, earliest_mtime_iso })
}
