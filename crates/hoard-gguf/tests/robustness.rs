//! Property tests: arbitrary bytes must never panic.
//!
//! The parser must be total over `&[u8]`: any input either parses or returns a
//! clean error. These tests also exercise the downstream report machinery on
//! inputs that happen to parse, since the inspect report must stay infallible.

mod common;

use std::sync::OnceLock;

use hoard_gguf::{Gguf, inspect};
use proptest::prelude::*;

fn rich() -> &'static [u8] {
    static RICH: OnceLock<Vec<u8>> = OnceLock::new();
    RICH.get_or_init(common::rich_gguf)
}

/// Run the whole pipeline under `catch_unwind` and report whether it panicked.
fn pipeline_does_not_panic(data: &[u8]) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let Ok(gguf) = Gguf::parse(data) {
            let report = inspect(&gguf, None);
            let json = serde_json::to_string(&report).expect("inspect report serializes");
            assert!(!json.is_empty());
            let _ = gguf.split();
            let _ = gguf.tensor_bytes();
            let _ = gguf.len();
            if let Some(tensor) = gguf.tensors().first() {
                let _ = (tensor.name(), tensor.bytes(), tensor.file_offset());
            }
        }
    }))
    .is_ok()
}

proptest! {
    #[test]
    fn arbitrary_bytes_never_panic(data in proptest::collection::vec(any::<u8>(), 0..2048)) {
        prop_assert!(
            pipeline_does_not_panic(&data),
            "panicked on {} random bytes",
            data.len()
        );
    }

    #[test]
    fn arbitrary_gguf_prefixed_bytes_never_panic(
        tail in proptest::collection::vec(any::<u8>(), 0..2048),
    ) {
        let mut data = b"GGUF".to_vec();
        data.extend_from_slice(&tail);
        prop_assert!(pipeline_does_not_panic(&data));
    }

    #[test]
    fn mutated_rich_fixtures_never_panic(
        mutations in proptest::collection::vec((0_usize..rich().len(), any::<u8>()), 0..8),
    ) {
        let mut data = rich().to_vec();
        for (index, byte) in mutations {
            data[index] = byte;
        }
        prop_assert!(pipeline_does_not_panic(&data));
    }

    #[test]
    fn truncated_rich_fixtures_never_panic(cut in 0_usize..rich().len()) {
        let data = &rich()[..cut];
        prop_assert!(pipeline_does_not_panic(data), "panicked on a {cut}-byte prefix");
        prop_assert!(
            Gguf::parse(data).is_err(),
            "a {cut}-byte prefix of the rich fixture parsed successfully"
        );
    }

    #[test]
    fn appended_bytes_to_the_rich_fixture_never_panic(
        suffix in proptest::collection::vec(any::<u8>(), 0..64),
    ) {
        let mut data = rich().to_vec();
        data.extend_from_slice(&suffix);
        prop_assert!(pipeline_does_not_panic(&data));
    }

    #[test]
    fn header_mutations_never_panic(
        version in any::<u32>(),
        tensor_count in any::<u64>(),
        metadata_count in any::<u64>(),
    ) {
        let mut data = common::minimal_gguf();
        data[4..8].copy_from_slice(&version.to_le_bytes());
        data[8..16].copy_from_slice(&tensor_count.to_le_bytes());
        data[16..24].copy_from_slice(&metadata_count.to_le_bytes());
        prop_assert!(pipeline_does_not_panic(&data));
    }
}
