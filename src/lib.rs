pub mod error;
pub use error::{Error, Result};

// Modules filled in by later tasks.
pub mod zip;
pub mod flatbuffers;
pub mod mdc;
pub mod ir;
pub mod build_ir;
pub mod hash;
pub mod emit;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat { V8, Firefox, IrJson }

#[derive(Clone, Debug, Default)]
pub struct EmitOptions { pub project_name: Option<String> }

pub trait Emitter {
    fn emit(&self, profile: &ir::Profile, opts: &EmitOptions) -> Result<Vec<u8>>;
}

/// Full pipeline: decode zip -> IR -> emit.
///
/// **Trust model:** `zip_bytes` is assumed to be well-formed Business Central
/// profiler output (Microsoft's own `.mdc` snapshot zip). The hand-rolled
/// FlatBuffers reader indexes the decompressed buffers directly for speed, so a
/// *malformed* or truncated `.mdc`/`version` entry may panic rather than return
/// `Err`. Callers feeding untrusted input should validate/catch accordingly.
pub fn convert(zip_bytes: &[u8], format: OutputFormat, opts: &EmitOptions) -> Result<Vec<u8>> {
    let snapshot = mdc::decode::decode_snapshot(zip_bytes)?;
    let mut profile = build_ir::build_profile(&snapshot.contexts);
    profile.capture = ir::CaptureMeta {
        platform_version: snapshot.platform_version,
        approx_wall_clock_start: snapshot.earliest_mtime_iso,
    };
    let emitter = emit::pick_emitter(format);
    emitter.emit(&profile, opts)
}
