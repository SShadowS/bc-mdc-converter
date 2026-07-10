use bc_mdc_converter::{convert, EmitOptions, OutputFormat, Error};
use std::process::ExitCode;

fn run() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut positional = Vec::new();
    let mut format = OutputFormat::V8;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--format" => {
                i += 1;
                let v = args.get(i).ok_or_else(|| Error::Cli("--format needs a value".into()))?;
                format = match v.as_str() {
                    "v8" => OutputFormat::V8,
                    "firefox" => OutputFormat::Firefox,
                    "ir-json" => OutputFormat::IrJson,
                    other => return Err(Error::Cli(format!("unknown format '{other}'"))),
                };
            }
            "-h" | "--help" => { print_usage(); return Ok(()); }
            other => positional.push(other.to_string()),
        }
        i += 1;
    }
    if positional.len() != 2 {
        return Err(Error::Cli("expected <in.mdc.zip> <out>".into()));
    }
    let in_path = &positional[0];
    let out_path = &positional[1];
    let bytes = std::fs::read(in_path)?;
    let opts = EmitOptions::default();
    let output = convert(&bytes, format, &opts)?;
    std::fs::write(out_path, &output)?; // UTF-8 no BOM: raw bytes, no prefix
    Ok(())
}

fn print_usage() {
    eprintln!("usage: bc-mdc-converter <in.mdc.zip> <out> [--format v8|firefox|ir-json]");
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => { eprintln!("error: {e}"); print_usage(); ExitCode::FAILURE }
    }
}
