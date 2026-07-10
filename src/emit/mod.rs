// Format-specific emitters. V8 (`.alcpuprofile`) is the primary, byte-exact target; Firefox
// Profiler (`firefox::FirefoxEmitter`) and ir-json (`ir_json::IrJsonEmitter`) are additional
// format-neutral-IR emitters built on top of the same `ir::Profile`.

pub mod v8;
pub(crate) mod v8_aggregate;
pub(crate) mod v8_json;
pub(crate) mod al_url;
pub mod firefox;
pub mod ir_json;

use crate::{Emitter, OutputFormat};

pub fn pick_emitter(format: OutputFormat) -> Box<dyn Emitter> {
    match format {
        OutputFormat::V8 => Box::new(v8::V8Emitter),
        OutputFormat::Firefox => Box::new(firefox::FirefoxEmitter),
        OutputFormat::IrJson => Box::new(ir_json::IrJsonEmitter),
    }
}
