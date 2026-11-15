//! `hoard-gguf` command-line interface.
//!
//! Commands: `inspect` and `schema`. The human-readable formatter reads the
//! frozen inspect report through `serde_json::Value`, so presentation stays
//! decoupled from the library's report structs while the JSON contract remains
//! the single source of truth.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use hoard_gguf::{Gguf, GgufFile};
use serde_json::Value;

#[derive(Parser)]
#[command(
    name = "hoard-gguf",
    version,
    about = "Inspect GGUF model files (strict, zero-copy parser)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Inspect a GGUF file: architecture, metadata, tensors, quant mix, memory.
    Inspect {
        /// Path to the GGUF file.
        path: PathBuf,
        /// Print the frozen inspect JSON (schema v1) instead of the human report.
        #[arg(long)]
        json: bool,
        /// Include the full tensor table in the human report.
        #[arg(long)]
        tensors: bool,
    },
    /// Print the inspect JSON Schema (schema v1).
    Schema,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            emit_stderr(&format!("error: {error:#}\n"));
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Inspect {
            path,
            json,
            tensors,
        } => {
            let file = GgufFile::open(&path)
                .with_context(|| format!("failed to open {}", path.display()))?;
            let gguf = Gguf::parse(file.bytes())
                .with_context(|| format!("failed to parse {}", path.display()))?;
            let report = hoard_gguf::inspect(&gguf, Some(path.as_path()));
            if json {
                let mut text = serde_json::to_string_pretty(&report)?;
                text.push('\n');
                emit_stdout(text.as_bytes())?;
            } else {
                let value = serde_json::to_value(&report)?;
                emit_stdout(render_human(&value, tensors).as_bytes())?;
                let mut warnings = String::new();
                for warning in value["warnings"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or(&[])
                {
                    let code = warning["code"].as_str().unwrap_or("warning");
                    let message = warning["message"].as_str().unwrap_or("");
                    warnings.push_str(&format!("warning: {code}: {message}\n"));
                }
                emit_stderr(&warnings);
            }
            Ok(())
        }
        Command::Schema => {
            let mut text = serde_json::to_string_pretty(&hoard_gguf::inspect_schema())?;
            text.push('\n');
            emit_stdout(text.as_bytes())?;
            Ok(())
        }
    }
}

fn emit_stdout(bytes: &[u8]) -> Result<()> {
    use std::io::Write;

    let mut stdout = std::io::stdout().lock();
    match stdout.write_all(bytes).and_then(|()| stdout.flush()) {
        Ok(()) => Ok(()),
        // `hoard-gguf inspect model.gguf | head` is normal usage; a closed
        // pipe is not an error.
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn emit_stderr(text: &str) {
    use std::io::Write;

    let _ = std::io::stderr().lock().write_all(text.as_bytes());
}

macro_rules! line {
    ($out:ident, $($arg:tt)*) => {{
        use std::fmt::Write as _;
        let _ = writeln!($out, $($arg)*);
    }};
}

fn render_human(report: &Value, show_tensors: bool) -> String {
    let mut out = String::new();
    let file = &report["file"];
    let model = &report["model"];
    let tensors = &report["tensors"];
    let memory = &report["memory"];

    let path = file["path"].as_str().unwrap_or("<unknown>");
    line!(out, "{path}");

    let size = memory["file_bytes"].as_u64().unwrap_or(0);
    line!(
        out,
        "  size        {} ({})",
        human_bytes(size),
        exact_bytes(size)
    );
    let version = file["version"].as_u64().unwrap_or(0);
    let byte_order = file["byte_order"].as_str().unwrap_or("little");
    line!(out, "  format      GGUF v{version} ({byte_order}-endian)");

    let architecture = model["architecture"].as_str().unwrap_or("-");
    match model["name"].as_str() {
        Some(name) => line!(out, "  model       {architecture} \"{name}\""),
        None => line!(out, "  model       {architecture}"),
    }

    let alignment = file["alignment"].as_u64().unwrap_or(0);
    let data_start = file["data_start"].as_u64().unwrap_or(0);
    line!(out, "  alignment   {alignment} bytes");
    line!(
        out,
        "  data        starts at byte {}",
        thousands(data_start)
    );

    let tensor_count = tensors["count"].as_u64().unwrap_or(0);
    let tensor_bytes = memory["tensor_bytes"].as_u64().unwrap_or(0);
    let share = percent(tensor_bytes, size);
    line!(
        out,
        "  tensors     {tensor_count}, {} weights ({share:.1}% of file)",
        human_bytes(tensor_bytes)
    );

    match &report["split"] {
        Value::Null => line!(out, "  split       none"),
        split => {
            let no = split["no"].as_u64().unwrap_or(0);
            let count = split["count"].as_u64().unwrap_or(0);
            let tensors = split["tensors_count"].as_i64().unwrap_or(0);
            line!(
                out,
                "  split       shard {no} of {count} ({tensors} tensors in model)"
            );
        }
    }

    line!(out,);
    line!(out, "quantization");
    if let Some(mix) = report["quant_mix"].as_array() {
        for entry in mix {
            let name = entry["type"].as_str().unwrap_or("unknown");
            let count = entry["tensors"].as_u64().unwrap_or(0);
            let bytes = entry["bytes"].as_u64().unwrap_or(0);
            let percent = entry["percent"].as_f64().unwrap_or(0.0);
            line!(
                out,
                "  {name:<10} {count:>6} tensors  {:>10}  {percent:>6.2}%",
                human_bytes(bytes)
            );
        }
    }

    line!(out,);
    let metadata = &report["metadata"];
    let entries = metadata["entries"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    line!(
        out,
        "metadata ({} keys)",
        metadata["count"].as_u64().unwrap_or(0)
    );
    for entry in entries {
        let key = entry["key"].as_str().unwrap_or("");
        let kind = entry["type"].as_str().unwrap_or("");
        let value = format_metadata_value(entry);
        line!(out, "  {key:<40} {kind:<18} {value}");
    }

    if show_tensors {
        line!(out,);
        line!(out, "tensors by offset");
        if let Some(entries) = tensors["entries"].as_array() {
            for entry in entries {
                let name = entry["name"].as_str().unwrap_or("");
                let kind = entry["type"].as_str().unwrap_or("unknown");
                let shape = entry["shape"]
                    .as_array()
                    .map(|dims| {
                        dims.iter()
                            .map(|dim| dim.as_u64().unwrap_or(0).to_string())
                            .collect::<Vec<_>>()
                            .join("x")
                    })
                    .unwrap_or_default();
                let bytes = match entry["bytes"].as_u64() {
                    Some(bytes) => human_bytes(bytes),
                    None => "unknown".to_string(),
                };
                let file_offset = entry["file_offset"].as_u64().unwrap_or(0);
                line!(
                    out,
                    "  {name:<34} {kind:<8} {shape:<14} {bytes:>10}  offset {}",
                    thousands(file_offset)
                );
            }
        }
    }

    line!(out,);
    line!(out, "memory");
    line!(
        out,
        "  weights    {:>10}  {}",
        human_bytes(tensor_bytes),
        exact_bytes(tensor_bytes)
    );
    line!(
        out,
        "  file       {:>10}  {}",
        human_bytes(size),
        exact_bytes(size)
    );
    line!(
        out,
        "  overhead   {:>10}  (header, tensor table, alignment padding, trailing data)",
        human_bytes(memory["overhead_bytes"].as_u64().unwrap_or(0))
    );
    line!(
        out,
        "  padding    {:>10}",
        human_bytes(memory["padding_bytes"].as_u64().unwrap_or(0))
    );
    out
}

fn format_metadata_value(entry: &Value) -> String {
    match entry["type"].as_str().unwrap_or("") {
        "array" => {
            let array = &entry["array"];
            let element = array["element_type"].as_str().unwrap_or("?");
            let length = array["length"].as_u64().unwrap_or(0);
            let sample = array["sample"].as_array().cloned().unwrap_or_default();
            let rendered = sample
                .iter()
                .map(format_scalar)
                .collect::<Vec<_>>()
                .join(", ");
            let rendered = truncate(&rendered, 72);
            if array["sample_truncated"].as_bool().unwrap_or(false) && !rendered.is_empty() {
                format!("array<{element}>[{length}]  {rendered}, ...")
            } else if rendered.is_empty() {
                format!("array<{element}>[{length}]")
            } else {
                format!("array<{element}>[{length}]  {rendered}")
            }
        }
        _ => truncate(&format_scalar(&entry["value"]), 96),
    }
}

fn format_scalar(value: &Value) -> String {
    match value {
        Value::String(text) => format!("\"{}\"", truncate(text, 64)),
        Value::Bool(boolean) => boolean.to_string(),
        Value::Number(number) => number.to_string(),
        Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut truncated: String = text.chars().take(limit.saturating_sub(1)).collect();
    truncated.push('…');
    truncated
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.2} {}", UNITS[unit])
}

fn exact_bytes(bytes: u64) -> String {
    format!("{} bytes", thousands(bytes))
}

fn thousands(number: u64) -> String {
    let digits = number.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    let offset = digits.len() % 3;
    for (index, ch) in digits.chars().enumerate() {
        if index != 0 && index % 3 == offset {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn percent(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        return 0.0;
    }
    (part as f64 / whole as f64) * 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_units_are_iec() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1024), "1.00 KiB");
        assert_eq!(human_bytes(1185376), "1.13 MiB");
        assert_eq!(human_bytes(1171200), "1.12 MiB");
    }

    #[test]
    fn exact_bytes_have_separators() {
        assert_eq!(exact_bytes(0), "0 bytes");
        assert_eq!(exact_bytes(999), "999 bytes");
        assert_eq!(exact_bytes(1000), "1,000 bytes");
        assert_eq!(exact_bytes(1185376), "1,185,376 bytes");
    }

    #[test]
    fn truncation_is_character_aware() {
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("héllo", 3), "hé…");
    }
}
