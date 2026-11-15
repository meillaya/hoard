//! Manifest builder tests.
//!
//! The builder is exercised with fabricated source metadata, with a real
//! content-addressed store in the end-to-end test, and against the manifest v1
//! validation rules. There is no network.

use std::fs;

use hoard_core::artifact::{
    Selection, SourceFile, build_manifest, infer_role, split_members, write_canonical,
};
use hoard_core::cas::{Entry, Store};
use hoard_core::hash::sha256_file;
use hoard_core::{
    Error, FileEntry, Hash32, Manifest, RepoType, Role, SCHEMA_VERSION, Source, SourceKind,
};

/// A Hugging Face commit sha used by the fabricated source.
const REVISION: &str = "499bc8821c6b12b4e53c5bffcb21ec206f212d81";

/// Returns a digest filled with `seed`, for fabricated metadata.
fn hash(seed: u8) -> Hash32 {
    Hash32::from_bytes([seed; 32])
}

/// Returns the source used by most tests.
fn source() -> Source {
    Source {
        kind: SourceKind::HuggingFace,
        repo: "ggml-org/models".to_owned(),
        repo_type: RepoType::Model,
        revision: REVISION.to_owned(),
    }
}

/// Returns a fabricated source listing entry.
fn remote_file(path: &str, size: u64, sha: u8) -> SourceFile {
    SourceFile {
        path: path.to_owned(),
        size,
        sha256: hash(sha),
    }
}

/// Returns a fabricated fetched entry consistent with a corresponding
/// [`remote_file`] call.
fn fetched(path: &str, size: u64, sha: u8, blake: u8) -> (String, Entry) {
    (
        path.to_owned(),
        Entry {
            blake3: hash(blake),
            sha256: hash(sha),
            size,
        },
    )
}

/// Returns a selection of `paths` with no role override and no group.
fn selection(paths: &[&str]) -> Selection {
    Selection {
        files: paths.iter().map(|path| (*path).to_owned()).collect(),
        group: None,
        role: None,
    }
}

/// Returns a vector of owned paths.
fn paths(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

/// Returns the sibling paths of `path` in a built manifest.
fn siblings<'a>(manifest: &'a Manifest, path: &str) -> &'a [String] {
    &manifest
        .files
        .iter()
        .find(|file| file.path == path)
        .unwrap_or_else(|| panic!("{path} is in the manifest"))
        .siblings
}

#[test]
fn infers_roles_from_base_names() {
    let cases = [
        ("mmproj-model-f16.gguf", Role::Mmproj),
        ("nested/mmproj-1.gguf", Role::Mmproj),
        ("mtp-draft.gguf", Role::Mtp),
        ("drafts/mtp-00001-of-00002.gguf", Role::Mtp),
        ("tokenizer.json", Role::Tokenizer),
        ("tokenizer_config.json", Role::Tokenizer),
        ("vocab.json", Role::Tokenizer),
        ("meta/vocab.txt", Role::Tokenizer),
        ("config.json", Role::Config),
        ("nested/config.json", Role::Config),
        ("model-q4_k_m.gguf", Role::Weights),
        ("model-00001-of-00002.gguf", Role::Weights),
        ("README.md", Role::Other),
        ("LICENSE", Role::Other),
        ("mmproj-notes.txt", Role::Mmproj),
        ("tokenizer.gguf", Role::Tokenizer),
        ("config.json.gguf", Role::Weights),
    ];
    for (path, role) in cases {
        assert_eq!(infer_role(path), role, "{path}");
    }

    // The roles are case-sensitive like the source paths themselves.
    assert_eq!(infer_role("MODEL.GGUF"), Role::Other);
    assert_eq!(infer_role("MMPROJ-notes.bin"), Role::Other);
    assert_eq!(infer_role("Config.json"), Role::Other);
}

#[test]
fn detects_a_two_part_split() {
    let members = split_members(&paths(&[
        "model-00001-of-00002.gguf",
        "model-00002-of-00002.gguf",
    ]))
    .unwrap();

    assert_eq!(
        members,
        vec![
            (
                "model-00001-of-00002.gguf".to_owned(),
                vec!["model-00002-of-00002.gguf".to_owned()],
                2
            ),
            (
                "model-00002-of-00002.gguf".to_owned(),
                vec!["model-00001-of-00002.gguf".to_owned()],
                2
            ),
        ]
    );
}

#[test]
fn detects_a_three_part_split_in_any_input_order() {
    let members = split_members(&paths(&[
        "q4/model-00003-of-00003.gguf",
        "q4/model-00001-of-00003.gguf",
        "q4/model-00002-of-00003.gguf",
    ]))
    .unwrap();

    let summary: Vec<(&str, u32)> = members
        .iter()
        .map(|(path, _, total)| (path.as_str(), *total))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("q4/model-00001-of-00003.gguf", 3),
            ("q4/model-00002-of-00003.gguf", 3),
            ("q4/model-00003-of-00003.gguf", 3),
        ],
        "members must be sorted by path"
    );
    assert_eq!(
        members[0].1,
        vec![
            "q4/model-00002-of-00003.gguf".to_owned(),
            "q4/model-00003-of-00003.gguf".to_owned(),
        ]
    );
    assert_eq!(
        members[2].1,
        vec![
            "q4/model-00001-of-00003.gguf".to_owned(),
            "q4/model-00002-of-00003.gguf".to_owned(),
        ]
    );
}

#[test]
fn ignores_files_that_are_not_split_members() {
    let members = split_members(&paths(&[
        "model-00001-of-00002.gguf",
        "config.json",
        "model-00002-of-00002.gguf",
        "weights.gguf",
    ]))
    .unwrap();
    assert_eq!(members.len(), 2);

    assert!(
        split_members(&paths(&["config.json", "weights.gguf"]))
            .unwrap()
            .is_empty()
    );
    assert!(
        split_members(&paths(&["model-0001-of-0002.gguf"]))
            .unwrap()
            .is_empty(),
        "the pattern requires five digits"
    );
    assert!(
        split_members(&paths(&["model-00001-of-00002.GGUF"]))
            .unwrap()
            .is_empty(),
        "the suffix is lower case"
    );
}

#[test]
fn rejects_inconsistent_split_totals() {
    let error = split_members(&paths(&[
        "model-00001-of-00002.gguf",
        "model-00002-of-00003.gguf",
    ]))
    .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("same total"), "{message}");
    assert!(message.contains("model-00001-of-00002.gguf"), "{message}");
    assert!(message.contains("model-00002-of-00003.gguf"), "{message}");
}

#[test]
fn rejects_a_split_missing_a_part() {
    let error = split_members(&paths(&[
        "model-00001-of-00003.gguf",
        "model-00003-of-00003.gguf",
    ]))
    .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("missing"), "{message}");
    assert!(
        message.contains("model-00002-of-00003.gguf"),
        "the missing path must be named: {message}"
    );
}

#[test]
fn caps_the_missing_part_list() {
    let error = split_members(&paths(&["model-00001-of-00100.gguf"])).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("model-00002-of-00100.gguf"), "{message}");
    assert!(message.contains("and 94 more"), "{message}");
    assert!(
        !message.contains("model-00099-of-00100.gguf"),
        "the list must be capped: {message}"
    );
}

#[test]
fn rejects_split_part_numbers_out_of_range() {
    for (parts, hint) in [
        (
            ["model-00000-of-00002.gguf", "model-00002-of-00002.gguf"],
            "1..=2",
        ),
        (
            ["model-00002-of-00002.gguf", "model-00003-of-00002.gguf"],
            "1..=2",
        ),
    ] {
        let error = split_members(&paths(&parts)).unwrap_err();
        assert!(error.to_string().contains(hint), "{error}");
    }
}

#[test]
fn rejects_duplicate_split_part_numbers() {
    let error = split_members(&paths(&[
        "model-00001-of-00002.gguf",
        "model-00001-of-00002.gguf",
    ]))
    .unwrap_err();
    assert!(error.to_string().contains("both declare part"), "{error}");
}

#[test]
fn builds_a_single_weights_file() {
    let remote = vec![remote_file("stories260K.gguf", 1_185_376, 1)];
    let fetched = vec![fetched("stories260K.gguf", 1_185_376, 1, 2)];

    let manifest = build_manifest(
        source(),
        &remote,
        &fetched,
        &selection(&["stories260K.gguf"]),
    )
    .unwrap();

    assert_eq!(manifest.schema, SCHEMA_VERSION);
    assert_eq!(manifest.source, source());
    assert_eq!(
        manifest.files,
        vec![FileEntry {
            path: "stories260K.gguf".to_owned(),
            role: Role::Weights,
            group: None,
            size: 1_185_376,
            sha256: hash(1),
            blake3: hash(2),
            siblings: Vec::new(),
        }]
    );
    manifest.validate().unwrap();

    let canonical = manifest.canonical_bytes();
    let reparsed = Manifest::from_json_bytes(&canonical).unwrap();
    assert_eq!(reparsed, manifest);
    assert_eq!(reparsed.canonical_bytes(), canonical);
    assert_eq!(reparsed.hash(), manifest.hash());
}

#[test]
fn unselected_fetched_entries_are_ignored() {
    let remote = vec![remote_file("model.gguf", 10, 1)];
    let fetched = vec![
        fetched("model.gguf", 10, 1, 2),
        fetched("other.gguf", 20, 3, 4),
    ];

    let manifest =
        build_manifest(source(), &remote, &fetched, &selection(&["model.gguf"])).unwrap();
    assert_eq!(manifest.files.len(), 1);
}

#[test]
fn builds_a_three_part_split_with_symmetric_siblings_and_group() {
    let remote = vec![
        remote_file("model-00001-of-00003.gguf", 100, 1),
        remote_file("model-00002-of-00003.gguf", 200, 2),
        remote_file("model-00003-of-00003.gguf", 300, 3),
    ];
    let fetched = vec![
        fetched("model-00001-of-00003.gguf", 100, 1, 11),
        fetched("model-00002-of-00003.gguf", 200, 2, 12),
        fetched("model-00003-of-00003.gguf", 300, 3, 13),
    ];
    let mut selection = selection(&[
        "model-00003-of-00003.gguf",
        "model-00001-of-00003.gguf",
        "model-00002-of-00003.gguf",
    ]);
    selection.group = Some("q4-k-m".to_owned());

    let manifest = build_manifest(source(), &remote, &fetched, &selection).unwrap();
    manifest.validate().unwrap();

    let names: Vec<&str> = manifest
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    assert_eq!(
        names,
        vec![
            "model-00001-of-00003.gguf",
            "model-00002-of-00003.gguf",
            "model-00003-of-00003.gguf",
        ],
        "files must be sorted by path"
    );
    for file in &manifest.files {
        assert_eq!(file.role, Role::Weights);
        assert_eq!(file.group.as_deref(), Some("q4-k-m"));
    }
    assert_eq!(
        siblings(&manifest, "model-00001-of-00003.gguf"),
        ["model-00002-of-00003.gguf", "model-00003-of-00003.gguf"]
    );
    assert_eq!(
        siblings(&manifest, "model-00002-of-00003.gguf"),
        ["model-00001-of-00003.gguf", "model-00003-of-00003.gguf"]
    );
    assert_eq!(
        siblings(&manifest, "model-00003-of-00003.gguf"),
        ["model-00001-of-00003.gguf", "model-00002-of-00003.gguf"]
    );
}

#[test]
fn writes_the_group_only_on_weights_files() {
    let remote = vec![
        remote_file("model.gguf", 1, 1),
        remote_file("config.json", 2, 2),
        remote_file("tokenizer.json", 3, 3),
        remote_file("mmproj-f16.gguf", 4, 4),
        remote_file("README.md", 5, 5),
    ];
    let fetched = vec![
        fetched("model.gguf", 1, 1, 11),
        fetched("config.json", 2, 2, 12),
        fetched("tokenizer.json", 3, 3, 13),
        fetched("mmproj-f16.gguf", 4, 4, 14),
        fetched("README.md", 5, 5, 15),
    ];
    let mut selection = selection(&[
        "model.gguf",
        "config.json",
        "tokenizer.json",
        "mmproj-f16.gguf",
        "README.md",
    ]);
    selection.group = Some("stories".to_owned());

    let manifest = build_manifest(source(), &remote, &fetched, &selection).unwrap();
    manifest.validate().unwrap();

    let roles: Vec<&str> = manifest
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    assert_eq!(
        roles,
        vec![
            "README.md",
            "config.json",
            "mmproj-f16.gguf",
            "model.gguf",
            "tokenizer.json",
        ],
        "files are sorted by path"
    );
    for file in &manifest.files {
        if file.role == Role::Weights {
            assert_eq!(file.group.as_deref(), Some("stories"), "{}", file.path);
        } else {
            assert_eq!(file.group, None, "{} must not have a group", file.path);
            assert!(
                file.siblings.is_empty(),
                "{} must not have siblings",
                file.path
            );
        }
    }
    assert_eq!(
        manifest
            .files
            .iter()
            .find(|file| file.path == "model.gguf")
            .unwrap()
            .role,
        Role::Weights
    );
    assert_eq!(
        manifest
            .files
            .iter()
            .find(|file| file.path == "mmproj-f16.gguf")
            .unwrap()
            .role,
        Role::Mmproj
    );
}

#[test]
fn a_role_override_replaces_inference_for_the_whole_selection() {
    let remote = vec![remote_file("blob.bin", 7, 7)];
    let fetched = vec![fetched("blob.bin", 7, 7, 17)];
    let selection = Selection {
        files: vec!["blob.bin".to_owned()],
        group: Some("custom".to_owned()),
        role: Some(Role::Weights),
    };

    let manifest = build_manifest(source(), &remote, &fetched, &selection).unwrap();
    manifest.validate().unwrap();

    assert_eq!(manifest.files[0].role, Role::Weights);
    assert_eq!(manifest.files[0].group.as_deref(), Some("custom"));
    assert!(manifest.files[0].siblings.is_empty());
}

#[test]
fn mmproj_split_files_get_no_siblings() {
    // Siblings are only allowed on weights, so a partial pair of mmproj files
    // is a complete selection for the builder.
    let remote = vec![
        remote_file("mmproj-00001-of-00002.gguf", 10, 1),
        remote_file("mmproj-00002-of-00002.gguf", 20, 2),
    ];
    let fetched = vec![
        fetched("mmproj-00001-of-00002.gguf", 10, 1, 11),
        fetched("mmproj-00002-of-00002.gguf", 20, 2, 12),
    ];
    let mut selection = selection(&["mmproj-00001-of-00002.gguf"]);
    selection.group = Some("vision".to_owned());

    let manifest = build_manifest(source(), &remote, &fetched, &selection).unwrap();
    manifest.validate().unwrap();
    assert_eq!(manifest.files[0].role, Role::Mmproj);
    assert_eq!(manifest.files[0].group, None);
    assert!(manifest.files[0].siblings.is_empty());
}

#[test]
fn reports_a_selection_path_missing_from_the_source_listing() {
    let remote = vec![remote_file("a.gguf", 1, 1), remote_file("b.gguf", 2, 2)];
    let fetched = vec![fetched("a.gguf", 1, 1, 11), fetched("b.gguf", 2, 2, 12)];

    let error = build_manifest(source(), &remote, &fetched, &selection(&["c.gguf"])).unwrap_err();
    assert!(matches!(error, Error::Validation(_)), "{error}");
    let message = error.to_string();
    assert!(message.contains("c.gguf"), "{message}");
    assert!(message.contains("not in the source listing"), "{message}");
    assert!(message.contains("a.gguf"), "available files: {message}");
    assert!(message.contains("b.gguf"), "available files: {message}");
}

#[test]
fn reports_a_selection_path_without_a_fetched_entry() {
    let remote = vec![remote_file("a.gguf", 1, 1)];

    let error = build_manifest(source(), &remote, &[], &selection(&["a.gguf"])).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("a.gguf"), "{message}");
    assert!(message.contains("content-addressed entry"), "{message}");
    assert!(message.contains("none"), "fetched list is empty: {message}");
}

#[test]
fn rejects_fetched_metadata_that_disagrees_with_the_source() {
    let remote = vec![remote_file("a.gguf", 10, 1)];

    let wrong_size = vec![fetched("a.gguf", 11, 1, 11)];
    let error =
        build_manifest(source(), &remote, &wrong_size, &selection(&["a.gguf"])).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("a.gguf"), "{message}");
    assert!(message.contains("size 10"), "{message}");
    assert!(message.contains("11"), "{message}");

    let wrong_sha = vec![fetched("a.gguf", 10, 9, 11)];
    let error = build_manifest(source(), &remote, &wrong_sha, &selection(&["a.gguf"])).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("a.gguf"), "{message}");
    assert!(message.contains("sha256"), "{message}");
    assert!(message.contains(&hash(1).to_string()), "{message}");
    assert!(message.contains(&hash(9).to_string()), "{message}");
}

#[test]
fn rejects_an_empty_selection() {
    let remote = vec![remote_file("a.gguf", 1, 1)];
    let fetched = vec![fetched("a.gguf", 1, 1, 11)];

    let error = build_manifest(source(), &remote, &fetched, &selection(&[])).unwrap_err();
    assert!(error.to_string().contains("at least one path"), "{error}");
}

#[test]
fn rejects_duplicate_selection_paths() {
    let remote = vec![remote_file("a.gguf", 1, 1)];
    let fetched = vec![fetched("a.gguf", 1, 1, 11)];

    let error = build_manifest(
        source(),
        &remote,
        &fetched,
        &selection(&["a.gguf", "a.gguf"]),
    )
    .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("a.gguf"), "{message}");
    assert!(message.contains("more than once"), "{message}");
}

#[test]
fn rejects_a_zero_size_source_file() {
    let remote = vec![remote_file("empty.gguf", 0, 1)];
    let fetched = vec![fetched("empty.gguf", 0, 1, 11)];

    let error =
        build_manifest(source(), &remote, &fetched, &selection(&["empty.gguf"])).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("empty.gguf"), "{message}");
    assert!(message.contains("larger than zero"), "{message}");
}

#[test]
fn rejects_an_unselected_split_sibling() {
    let remote = vec![
        remote_file("model-00001-of-00003.gguf", 100, 1),
        remote_file("model-00002-of-00003.gguf", 200, 2),
        remote_file("model-00003-of-00003.gguf", 300, 3),
    ];
    let fetched = vec![
        fetched("model-00001-of-00003.gguf", 100, 1, 11),
        fetched("model-00002-of-00003.gguf", 200, 2, 12),
        fetched("model-00003-of-00003.gguf", 300, 3, 13),
    ];
    let selection = selection(&["model-00001-of-00003.gguf", "model-00003-of-00003.gguf"]);

    let error = build_manifest(source(), &remote, &fetched, &selection).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("missing"), "{message}");
    assert!(
        message.contains("model-00002-of-00003.gguf"),
        "the unselected sibling must be named: {message}"
    );
}

#[test]
fn rejects_an_invalid_path_from_the_source_listing() {
    let remote = vec![remote_file("weights/../evil.gguf", 10, 1)];
    let fetched = vec![fetched("weights/../evil.gguf", 10, 1, 11)];

    let error = build_manifest(
        source(),
        &remote,
        &fetched,
        &selection(&["weights/../evil.gguf"]),
    )
    .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("weights/../evil.gguf"), "{message}");
    assert!(message.contains("path"), "{message}");
}

#[test]
fn rejects_an_invalid_source() {
    let remote = vec![remote_file("a.gguf", 1, 1)];
    let fetched = vec![fetched("a.gguf", 1, 1, 11)];
    let mut source = source();
    source.revision = "main".to_owned();

    let error = build_manifest(source, &remote, &fetched, &selection(&["a.gguf"])).unwrap_err();
    assert!(error.to_string().contains("source.revision"), "{error}");
}

#[test]
fn write_canonical_writes_and_replaces_canonical_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let manifest = build_manifest(
        source(),
        &[remote_file("a.gguf", 1, 1)],
        &[fetched("a.gguf", 1, 1, 11)],
        &selection(&["a.gguf"]),
    )
    .unwrap();
    let path = directory.path().join("manifest.json");

    write_canonical(&manifest, &path).unwrap();

    assert_eq!(fs::read(&path).unwrap(), manifest.canonical_bytes());
    assert_eq!(
        fs::read_dir(directory.path()).unwrap().count(),
        1,
        "no temporary file may be left behind"
    );
    let reparsed = Manifest::from_path(&path).unwrap();
    assert_eq!(reparsed, manifest);
    reparsed.validate().unwrap();

    let mut changed = manifest.clone();
    changed.files[0].size += 1;
    write_canonical(&changed, &path).unwrap();
    assert_eq!(fs::read(&path).unwrap(), changed.canonical_bytes());
}

#[test]
fn write_canonical_reports_a_missing_parent_directory() {
    let directory = tempfile::tempdir().unwrap();
    let manifest = build_manifest(
        source(),
        &[remote_file("a.gguf", 1, 1)],
        &[fetched("a.gguf", 1, 1, 11)],
        &selection(&["a.gguf"]),
    )
    .unwrap();
    let path = directory.path().join("missing/manifest.json");

    let error = write_canonical(&manifest, &path).unwrap_err();
    assert!(matches!(error, Error::Io { .. }), "{error}");
    assert!(error.to_string().contains("manifest.json"), "{error}");
}

#[test]
fn builds_a_valid_manifest_from_a_real_store() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("store")).unwrap();
    let names = ["model-00001-of-00002.gguf", "model-00002-of-00002.gguf"];
    let contents = ["first shard contents", "second shard contents"];

    let mut remote = Vec::new();
    let mut fetched = Vec::new();
    for (index, name) in names.iter().enumerate() {
        let source_path = directory.path().join(format!("source-{index}.bin"));
        fs::write(&source_path, contents[index]).unwrap();
        let sha256 = sha256_file(&source_path).unwrap();
        let entry = store.put_file(&source_path, sha256).unwrap();
        remote.push(SourceFile {
            path: (*name).to_owned(),
            size: contents[index].len() as u64,
            sha256,
        });
        fetched.push(((*name).to_owned(), entry));
    }

    let manifest = build_manifest(source(), &remote, &fetched, &selection(&names)).unwrap();
    manifest.validate().unwrap();
    assert_eq!(
        siblings(&manifest, names[0]),
        [names[1]],
        "real split members must be siblings"
    );

    let out = directory.path().join("out");
    fs::create_dir(&out).unwrap();
    let manifest_path = out.join("manifest.json");
    write_canonical(&manifest, &manifest_path).unwrap();
    let reparsed = Manifest::from_path(&manifest_path).unwrap();
    assert_eq!(reparsed, manifest);

    let materialized = out.join("materialized").join(names[0]);
    store.materialize(&fetched[0].1, &materialized).unwrap();
    assert_eq!(fs::read(&materialized).unwrap(), contents[0].as_bytes());
    store.verify(&fetched[1].1).unwrap();
}
