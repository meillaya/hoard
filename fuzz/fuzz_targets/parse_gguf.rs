#![no_main]

//! Fuzz target: arbitrary bytes into `Gguf::parse`, then touch the whole
//! parsed view and render the full inspect report.
//!
//! Contract under test: for *any* input, parsing either returns a typed error
//! or a fully validated `Gguf`; no input may panic, and every accepted file
//! must survive report generation. This intentionally does not weaken the
//! parser caps or panic behaviour.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(gguf) = hoard_gguf::Gguf::parse(data) else {
        return;
    };

    // Header and file-level facts.
    let header = gguf.header();
    let _ = (
        header.version,
        header.tensor_count,
        header.metadata_kv_count,
    );
    let _ = (
        gguf.alignment(),
        gguf.data_start(),
        gguf.len(),
        gguf.is_empty(),
        gguf.bytes(),
    );
    let _ = gguf.tensor_bytes();
    let _ = gguf.split();

    // Every metadata entry and the whole value tree.
    for entry in gguf.metadata() {
        let _ = (entry.key(), entry.value_type());
        match entry.value() {
            hoard_gguf::MetadataValue::Array(values) => {
                let _ = (
                    values.element_type(),
                    values.len(),
                    values.is_empty(),
                    values.values(),
                );
            }
            _ => {}
        }
    }

    // Every tensor info and its derived geometry.
    for tensor in gguf.tensors() {
        let _ = (
            tensor.name(),
            tensor.shape(),
            tensor.n_dims(),
            tensor.type_id(),
            tensor.quant(),
            tensor.type_name(),
            tensor.elements(),
            tensor.bytes(),
            tensor.offset(),
            tensor.file_offset(),
        );
        let _ = gguf.tensor(tensor.name());
    }

    // Full inspect report generation is part of the parser's public contract.
    let report = hoard_gguf::inspect(&gguf, Some(std::path::Path::new("fuzz.gguf")));
    let _ = (
        report.schema,
        &report.file,
        &report.model,
        report.split,
        &report.metadata,
        &report.tensors,
        &report.quant_mix,
        report.memory,
        &report.warnings,
    );
    let _ = hoard_gguf::inspect_schema();
});
