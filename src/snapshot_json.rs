// Lossless JSON dump of a debugging snapshot zip (`--format snapshot-json`, schemaVersion 1).
// Streams: one zip entry inflated and one method written at a time; every limit is checked while
// reading/writing, never after allocating the whole thing.

use crate::mdc::schema::decode_version;
use crate::mdc::snapshot::{decode_snap_method, SnapFrame, SnapLineContext, SnapMethod, SnapSpan, SnapVariable};
use crate::mdc::types::ObjectType;
use crate::{Error, Result};
use serde_json::{json, Value};
use std::io::{Read, Write};

#[derive(Clone, Debug)]
pub struct SnapshotLimits { pub max_entry_bytes: u64, pub max_total_bytes: u64, pub max_output_bytes: u64 }
impl Default for SnapshotLimits {
    fn default() -> Self { SnapshotLimits { max_entry_bytes: 64 << 20, max_total_bytes: 512 << 20, max_output_bytes: 64 << 20 } }
}

struct Counting<W: Write> { inner: W, written: u64, max: u64 }
impl<W: Write> Write for Counting<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.written += buf.len() as u64;
        if self.written > self.max {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "snapshot-json output exceeds the limit"));
        }
        self.inner.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> { self.inner.flush() }
}

fn span(s: &Option<SnapSpan>) -> Value {
    match s {
        Some(s) => json!({ "from": { "line": s.from.0, "column": s.from.1 }, "to": { "line": s.to.0, "column": s.to.1 } }),
        None => Value::Null,
    }
}
fn type_name(t: Option<i8>) -> Value { t.map(|t| Value::from(ObjectType::from_i8(t).name())).unwrap_or(Value::Null) }
fn object(t: Option<i8>, id: Option<i32>, name: Option<&str>) -> Value {
    json!({ "type": type_name(t), "typeId": t, "id": id, "name": name })
}
fn variable(v: &SnapVariable) -> Value {
    json!({ "name": v.name, "type": v.type_name, "value": v.value, "hasChildren": v.has_children,
            "children": v.children.iter().map(variable).collect::<Vec<_>>() })
}
fn frame(f: &SnapFrame) -> Value {
    json!({ "methodName": f.method_name, "object": object(f.object_type_id, f.object_id, Some(&f.object_name)), "span": span(&f.span) })
}
fn line_context(lc: &SnapLineContext) -> Value {
    json!({ "index": lc.index, "lineNumber": lc.line_number, "span": span(&lc.span),
            "timestamp": lc.timestamp.to_string(), "variables": lc.variables.iter().map(variable).collect::<Vec<_>>() })
}
fn method(entry: &str, m: &SnapMethod) -> Value {
    let name = m.frames.iter().find(|f| f.object_type_id == m.object_type_id && f.object_id == m.object_id).map(|f| f.object_name.as_str());
    json!({
        "entry": entry, "methodName": m.method_name, "object": object(m.object_type_id, m.object_id, name),
        "app": m.app.as_ref().map(|a| json!({ "name": a.app_name, "publisher": a.app_publisher, "version": a.app_version, "id": a.app_id })),
        "isBuiltin": m.is_builtin, "isIncomplete": m.is_incomplete,
        "frames": m.frames.iter().map(frame).collect::<Vec<_>>(),
        "lineContexts": m.line_contexts.iter().map(line_context).collect::<Vec<_>>(),
        "exception": m.exception.as_ref().map(|e| json!({ "message": e.message, "lineNumber": e.line_number, "span": span(&e.span) })),
    })
}
fn source(entry: &str) -> Value {
    // Observed: `Page%50152.al`. Inferred grammar `<ObjectType>%<ObjectId>.al`; anything else keeps only `entry`.
    let parsed = entry.strip_suffix(".al").and_then(|stem| stem.split_once('%'))
        .and_then(|(t, id)| id.parse::<i32>().ok().map(|id| (t.to_string(), id)));
    match parsed {
        Some((t, id)) => json!({ "entry": entry, "objectType": t, "objectId": id }),
        None => json!({ "entry": entry }),
    }
}

fn read_limited(file: &mut impl Read, limits: &SnapshotLimits, total: &mut u64, name: &str) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    file.by_ref().take(limits.max_entry_bytes + 1).read_to_end(&mut data)?;
    if data.len() as u64 > limits.max_entry_bytes { return Err(Error::Decode(format!("{name} exceeds the per-entry limit"))); }
    *total += data.len() as u64;
    if *total > limits.max_total_bytes { return Err(Error::Decode("archive exceeds the total decompressed limit".into())); }
    Ok(data)
}

pub fn write_snapshot_json<W: Write>(zip_bytes: &[u8], out: W, limits: &SnapshotLimits) -> Result<()> {
    let mut archive = ::zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|e| Error::Zip(e.to_string()))?;
    let names: Vec<String> = archive.file_names().map(str::to_string).collect();
    let mut total = 0u64;
    let version = match names.iter().position(|n| n == "version") {
        Some(_) => {
            let mut f = archive.by_name("version").map_err(|e| Error::Zip(e.to_string()))?;
            let v = decode_version(&read_limited(&mut f, limits, &mut total, "version")?);
            vec![v.0, v.1, v.2, v.3]
        }
        None => vec![0, 0, 0, 0],
    };
    let mut mdc: Vec<(i64, String)> = names.iter()
        .filter_map(|n| n.to_ascii_lowercase().strip_suffix(".mdc").and_then(|s| s.parse::<i64>().ok()).map(|k| (k, n.clone())))
        .collect();
    mdc.sort_by_key(|(k, _)| *k);
    let sources: Vec<Value> = names.iter().filter(|n| n.to_ascii_lowercase().ends_with(".al")).map(|n| source(n)).collect();

    let mut w = Counting { inner: std::io::BufWriter::new(out), written: 0, max: limits.max_output_bytes };
    let io = |e: std::io::Error| Error::Emit(e.to_string());
    write!(w, "{{\"format\":\"bc-mdc-snapshot\",\"schemaVersion\":1,\"version\":{},\"methods\":[", json!(version)).map_err(io)?;
    let mut first = true;
    for (_, name) in &mdc {
        let data = {
            let mut f = archive.by_name(name).map_err(|e| Error::Zip(e.to_string()))?;
            read_limited(&mut f, limits, &mut total, name)?
        };
        if data.is_empty() { continue; }
        let m = decode_snap_method(&data).map_err(|e| Error::Decode(format!("{name}: {e}")))?;
        if !first { w.write_all(b",").map_err(io)?; }
        first = false;
        serde_json::to_writer(&mut w, &method(name, &m)).map_err(|e| Error::Emit(e.to_string()))?;
    }
    write!(w, "],\"sources\":{}}}", json!(sources)).map_err(io)?;
    w.flush().map_err(io)
}

#[doc(hidden)]
pub fn write_str_for_test<W: Write>(out: W, s: &str) -> Result<()> {
    serde_json::to_writer(out, s).map_err(|e| Error::Emit(e.to_string()))
}
