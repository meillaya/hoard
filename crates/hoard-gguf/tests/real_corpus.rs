//! Real-corpus tests against `fixtures/real/`.
//!
//! `fixtures/real/` is gitignored, so these tests skip (with an `eprintln!`
//! notice) when the files are not present; they still run whenever the corpus
//! has been fetched, which is what CI does.
//!
//! `stories260K.gguf` is a small little-endian GGUF v3 Llama model; the
//! `-be.gguf` twin has been byte-swapped to big-endian and must be rejected
//! with the big-endian message.

mod common;

use hoard_gguf::{Gguf, GgufFile, inspect};

const LE_FILE: &str = "stories260K.gguf";
const BE_FILE: &str = "stories260K-be.gguf";

fn present(name: &str) -> Option<std::path::PathBuf> {
    let path = common::real_fixture(name);
    if path.is_file() {
        Some(path)
    } else {
        eprintln!(
            "skipping real-corpus test: {} is not present",
            path.display()
        );
        None
    }
}

#[test]
fn real_little_endian_corpus_parses_with_the_expected_shape() {
    let Some(path) = present(LE_FILE) else {
        return;
    };
    let file = GgufFile::open(&path).expect("open real corpus file");
    assert_eq!(file.len(), 1_185_376);
    assert_eq!(file.bytes().len(), 1_185_376_usize);
    let gguf = Gguf::parse(file.bytes()).expect("parse real LE corpus file");

    assert_eq!(gguf.header().version, 3);
    assert_eq!(gguf.alignment(), 32);
    assert_eq!(gguf.data_start(), 14_176);
    assert_eq!(gguf.tensors().len(), 48);
    assert_eq!(gguf.metadata().len(), 19);
    assert_eq!(gguf.tensor_bytes().tensor_bytes, 1_171_200);
    assert!(gguf.tensor_bytes().bytes_known);
    assert_eq!(gguf.tensor_bytes().unknown_types, 0);
    assert!(gguf.tensors().iter().all(|tensor| tensor.type_id() == 0));
    assert!(
        gguf.tensors()
            .iter()
            .all(|tensor| tensor.type_name() == "F32")
    );
    assert!(
        gguf.warnings().is_empty(),
        "unexpected warnings: {:?}",
        gguf.warnings()
    );
    assert_eq!(gguf.split().expect("split metadata analysis"), None);

    let report = inspect(&gguf, Some(&path));
    assert_eq!(report.model.architecture.as_deref(), Some("llama"));
    assert_eq!(report.model.name.as_deref(), Some("llama"));
    assert_eq!(report.schema, 1);
    assert_eq!(report.tensors.count, 48);
    assert_eq!(report.tensors.total_bytes, 1_171_200);
    assert_eq!(report.metadata.count, 19);
    assert_eq!(report.file.size, 1_185_376);
    assert_eq!(report.file.alignment, 32);
    assert_eq!(report.file.data_start, 14_176);
    assert_eq!(report.quant_mix.len(), 1);
    assert_eq!(report.quant_mix[0].type_id, 0);
    assert_eq!(report.quant_mix[0].value_type, "F32");
    assert_eq!(report.quant_mix[0].tensors, 48);
    assert_eq!(report.quant_mix[0].bytes, 1_171_200);
    assert_eq!(report.quant_mix[0].percent, 100.0);
    assert_eq!(report.memory.overhead_bytes, 1_185_376 - 1_171_200);
    assert_eq!(report.split, None);
    assert!(report.warnings.is_empty());
}

#[test]
fn real_big_endian_corpus_is_rejected_with_the_big_endian_message() {
    let Some(path) = present(BE_FILE) else {
        return;
    };
    let bytes = std::fs::read(&path).expect("read big-endian corpus file");
    let error = match Gguf::parse(&bytes) {
        Err(error) => error,
        Ok(_) => panic!("big-endian corpus file parsed successfully"),
    };
    assert_eq!(error.code(), "big-endian");
    assert!(
        error.to_string().contains("big-endian"),
        "unexpected message: {error}"
    );
    assert!(
        error
            .to_string()
            .contains("big-endian GGUF is not supported (write little-endian)"),
        "unexpected message: {error}"
    );
}

#[test]
fn real_little_endian_corpus_tensor_ranges_are_inside_the_file() {
    let Some(path) = present(LE_FILE) else {
        return;
    };
    let file = GgufFile::open(&path).expect("open real corpus file");
    let gguf = Gguf::parse(file.bytes()).expect("parse real LE corpus file");
    let file_len = gguf.len();
    for tensor in gguf.tensors() {
        let start = tensor.file_offset();
        let bytes = tensor.bytes().expect("F32 size is known");
        let end = start.checked_add(bytes).expect("no overflow");
        assert_eq!(start % 32, 0, "tensor {} is not aligned", tensor.name());
        assert!(end <= file_len, "tensor {} exceeds the file", tensor.name());
    }
}
