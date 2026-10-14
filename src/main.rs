use bc_mdc_converter::{convert, snapshot_json::{write_snapshot_json, SnapshotLimits}, EmitOptions, OutputFormat, Error};
use std::process::ExitCode;

enum Fmt { Profile(OutputFormat), SnapshotJson }
const FORMATS: &str = "v8\nfirefox\nir-json\nsnapshot-json\n";

fn run() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut positional = Vec::new();
    let mut format = Fmt::Profile(OutputFormat::V8);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--format" => {
                i += 1;
                let v = args.get(i).ok_or_else(|| Error::Cli("--format needs a value".into()))?;
                format = match v.as_str() {
                    "v8" => Fmt::Profile(OutputFormat::V8),
                    "firefox" => Fmt::Profile(OutputFormat::Firefox),
                    "ir-json" => Fmt::Profile(OutputFormat::IrJson),
                    "snapshot-json" => Fmt::SnapshotJson,
                    other => return Err(Error::Cli(format!("unknown format '{other}'"))),
                };
            }
            "--version" => { println!("bc-mdc-converter {}", env!("CARGO_PKG_VERSION")); return Ok(()); }
            "--formats" => { print!("{FORMATS}"); return Ok(()); }
            "-h" | "--help" => { print_usage(); return Ok(()); }
            other => positional.push(other.to_string()),
        }
        i += 1;
    }
    if positional.len() != 2 { return Err(Error::Cli("expected <in.mdc.zip> <out>".into())); }
    let (in_path, out_path) = (&positional[0], &positional[1]);
    let bytes = std::fs::read(in_path)?;
    match format {
        Fmt::Profile(f) => {
            let output = convert(&bytes, f, &EmitOptions::default())?;
            std::fs::write(out_path, &output)?; // UTF-8 no BOM: raw bytes, no prefix
        }
        Fmt::SnapshotJson => {
            std::panic::set_hook(Box::new(|_| {})); // decode panics are caught and reported as errors
            let file = std::fs::File::create(out_path)?;
            if let Err(e) = write_snapshot_json(&bytes, file, &SnapshotLimits::default()) {
                let _ = std::fs::remove_file(out_path);
                return Err(e);
            }
        }
    }
    Ok(())
}

fn print_usage() {
    eprintln!("usage: bc-mdc-converter <in.mdc.zip> <out> [--format v8|firefox|ir-json|snapshot-json]\n       bc-mdc-converter --version | --formats");
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => { eprintln!("error: {e}"); print_usage(); ExitCode::FAILURE }
    }
}
