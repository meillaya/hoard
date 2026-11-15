#!/usr/bin/env python3
"""Differential oracle: compare ``hoard-gguf inspect --json`` against llama.cpp.

The reference parser is ``gguf-py/gguf/gguf_reader.py`` from the pinned
``llama.cpp`` commit (the commit is pinned once, in
``scripts/check-gguf-oracle.sh``). This script loads that reader from a
checkout/tarball extraction of the pinned tree, runs ``hoard-gguf inspect
--json`` on the same file, and compares the two views field by field.

Compared fields:

* file version, byte order, alignment, data-section offset, file size;
* tensor count, metadata (key/value) count;
* every metadata entry in file order: key, value type, and value (scalars
  exactly; arrays compare element type and length, plus the first three sample
  values for flat arrays, and nested-array presence/type/length only);
* every tensor: name, type id, type name (mapped from llama.cpp's
  ``GGMLQuantizationType``), shape, element count, byte size, and absolute
  data offset;
* derived aggregates: ``quant_mix`` groups and percentages, and the
  ``memory`` accounting.

A file either matches completely or the script prints every difference and
exits non-zero. Unknown quantization ids are reported explicitly instead of
being silently accepted.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import subprocess
import sys
import types
from pathlib import Path
from typing import Any

# llama.cpp's GGUFValueType enum member name -> inspect JSON value type name.
VALUE_TYPE_NAMES: dict[str, str] = {
    "UINT8": "u8",
    "INT8": "i8",
    "UINT16": "u16",
    "INT16": "i16",
    "UINT32": "u32",
    "INT32": "i32",
    "FLOAT32": "f32",
    "BOOL": "bool",
    "STRING": "string",
    "ARRAY": "array",
    "UINT64": "u64",
    "INT64": "i64",
    "FLOAT64": "f64",
}

# Inspect JSON type names whose values are JSON integers (not booleans).
INTEGER_TYPE_NAMES = {"u8", "i8", "u16", "i16", "u32", "i32", "u64", "i64"}
FLOAT_TYPE_NAMES = {"f32", "f64"}

# Metadata keys that are compared both as raw entries and as model fields.
MODEL_FIELDS = {
    "architecture": ("general.architecture", "string"),
    "name": ("general.name", "string"),
    "quantization_version": ("general.quantization_version", "u32"),
}


class OracleError(RuntimeError):
    """The comparison could not run or the reference parser rejected a file."""


class Reference:
    """The pinned llama.cpp gguf-py reference reader and its type enums."""

    def __init__(self, reader, quant_type, value_type, description: str):
        self.GGUFReader = reader
        self.GGMLQuantizationType = quant_type
        self.GGUFValueType = value_type
        self.description = description

    def quant_type_name(self, type_id: int) -> str:
        """Map a ggml type id to its inspect JSON name, or "unknown"."""
        try:
            return self.GGMLQuantizationType(type_id).name
        except ValueError:
            return "unknown"

    def value_type_name(self, type_id: int) -> str:
        """Map a metadata value type id to its inspect JSON name."""
        try:
            member = self.GGUFValueType(type_id)
        except ValueError as exc:
            raise OracleError(f"unknown metadata value type id {type_id}") from exc
        try:
            return VALUE_TYPE_NAMES[member.name]
        except KeyError as exc:
            raise OracleError(f"unmapped metadata value type {member.name}") from exc


def load_reference(tree: Path) -> Reference:
    """Import the reference reader from a pinned llama.cpp tree.

    ``gguf/__init__.py`` imports the writer/vocab/tensor-mapping modules, which
    pull in optional dependencies (PyYAML today). When that import fails we fall
    back to registering the package namespace and importing only the reference
    reader; the reader itself only needs numpy. Both paths execute the same
    pinned ``gguf_reader.py`` source file.
    """
    gguf_py = tree / "gguf-py"
    reader_py = gguf_py / "gguf" / "gguf_reader.py"
    if not reader_py.is_file():
        raise OracleError(
            f"{reader_py} is missing; pass --llama-cpp-tree pointing at a "
            "llama.cpp checkout or tarball extraction"
        )
    sys.path.insert(0, str(gguf_py))
    try:
        from gguf import GGUFReader  # type: ignore[import-not-found]
        from gguf.constants import GGMLQuantizationType, GGUFValueType  # type: ignore[import-not-found]

        return Reference(
            GGUFReader,
            GGMLQuantizationType,
            GGUFValueType,
            "canonical gguf-py package import",
        )
    except Exception as package_error:  # noqa: BLE001 - optional deps vary by tree
        for name in [name for name in sys.modules if name == "gguf" or name.startswith("gguf.")]:
            del sys.modules[name]
        package = types.ModuleType("gguf")
        package.__path__ = [str(gguf_py / "gguf")]
        sys.modules["gguf"] = package
        try:
            from gguf.gguf_reader import GGUFReader  # type: ignore[import-not-found]
            from gguf.constants import GGMLQuantizationType, GGUFValueType  # type: ignore[import-not-found]
        except Exception as reader_error:  # noqa: BLE001
            raise OracleError(
                f"cannot import gguf_reader from {tree}: package import failed "
                f"({package_error}); direct reader import failed ({reader_error})"
            ) from reader_error
        return Reference(
            GGUFReader,
            GGMLQuantizationType,
            GGUFValueType,
            f"gguf.gguf_reader direct import (package __init__ needs optional deps: {package_error})",
        )


def add_diff(diffs: list[str], field: str, expected: Any, actual: Any) -> None:
    diffs.append(f"{field}: reference={expected!r} hoard={actual!r}")


def compare_int(diffs: list[str], field: str, expected: int, actual: Any) -> None:
    if not isinstance(actual, int) or isinstance(actual, bool) or actual != int(expected):
        add_diff(diffs, field, int(expected), actual)


def scalar_matches(type_name: str, reference: Any, actual: Any) -> bool:
    """Compare one metadata scalar, keeping JSON types strict."""
    if type_name in INTEGER_TYPE_NAMES:
        if isinstance(actual, bool) or not isinstance(actual, int):
            return False
        return int(actual) == int(reference)
    if type_name == "bool":
        return isinstance(actual, bool) and bool(actual) == bool(reference)
    if type_name in FLOAT_TYPE_NAMES:
        if isinstance(actual, bool) or not isinstance(actual, (int, float)):
            return False
        if isinstance(reference, float) and math.isnan(reference):
            return isinstance(actual, float) and math.isnan(actual)
        return float(actual) == float(reference)
    if type_name == "string":
        return isinstance(actual, str) and actual == reference
    return False


def compare_metadata(
    ref: Reference, reader: Any, report: dict[str, Any], diffs: list[str]
) -> str:
    kv_count = int(reader.fields["GGUF.kv_count"].contents())
    entries = report["metadata"]["entries"]
    compare_int(diffs, "metadata.count", kv_count, report["metadata"]["count"])
    if len(entries) != kv_count:
        add_diff(diffs, "metadata.entries length", kv_count, len(entries))

    ref_fields = [(key, field) for key, field in reader.fields.items() if not key.startswith("GGUF.")]
    scalar_types: dict[str, int] = {}
    array_types: dict[str, int] = {}
    nested_arrays = 0
    for index, ((ref_key, field), entry) in enumerate(zip(ref_fields, entries)):
        path = f"metadata.entries[{index}]"
        if entry.get("key") != ref_key:
            add_diff(diffs, f"{path}.key", ref_key, entry.get("key"))
            continue
        if not field.types:
            add_diff(diffs, f"{path}.type", "<missing reference type>", entry.get("type"))
            continue
        ref_type_id = int(field.types[0])
        expected_type = ref.value_type_name(ref_type_id)
        if entry.get("type") != expected_type:
            add_diff(diffs, f"{path}.type", expected_type, entry.get("type"))
            continue

        if expected_type != "array":
            scalar_types[expected_type] = scalar_types.get(expected_type, 0) + 1
            reference_value = field.contents()
            if not scalar_matches(expected_type, reference_value, entry.get("value")):
                add_diff(diffs, f"{path}.value ({ref_key})", reference_value, entry.get("value"))
            continue

        array = entry.get("array")
        if not isinstance(array, dict):
            add_diff(diffs, f"{path}.array", "array object", array)
            continue
        # Array header parts are [kv_klen, kv_kdata, raw_kv_type, raw_itype, alen]
        # before the element parts; this is stable across the pinned reader.
        raw_elem_type = int(field.parts[3][0])
        length = int(field.parts[4][0])
        expected_elem = ref.value_type_name(raw_elem_type)
        array_types[expected_elem] = array_types.get(expected_elem, 0) + 1
        if array.get("element_type") != expected_elem:
            add_diff(diffs, f"{path}.array.element_type ({ref_key})", expected_elem, array.get("element_type"))
        compare_int(diffs, f"{path}.array.length ({ref_key})", length, array.get("length"))

        sample = array.get("sample")
        if not isinstance(sample, list):
            add_diff(diffs, f"{path}.array.sample", "list", sample)
            continue
        if expected_elem == "array":
            # Nested arrays are sampled without recursion: empty sample, and
            # truncated whenever there is at least one element.
            nested_arrays += 1
            if sample:
                add_diff(diffs, f"{path}.array.sample ({ref_key})", [], sample)
            expected_truncated = length > 0
            if array.get("sample_truncated") != expected_truncated:
                add_diff(
                    diffs,
                    f"{path}.array.sample_truncated ({ref_key})",
                    expected_truncated,
                    array.get("sample_truncated"),
                )
            continue

        reference_values = field.contents()
        expected_sample = reference_values[: len(sample)]
        for sample_index, (expected_value, actual_value) in enumerate(zip(expected_sample, sample)):
            if not scalar_matches(expected_elem, expected_value, actual_value):
                add_diff(
                    diffs,
                    f"{path}.array.sample[{sample_index}] ({ref_key})",
                    expected_value,
                    actual_value,
                )
        expected_truncated = length > len(sample)
        if array.get("sample_truncated") != expected_truncated:
            add_diff(
                diffs,
                f"{path}.array.sample_truncated ({ref_key})",
                expected_truncated,
                array.get("sample_truncated"),
            )

    summary = (
        f"metadata: {kv_count} entr{'y' if kv_count == 1 else 'ies'} match "
        f"({len(scalar_types)} scalar type(s), {sum(array_types.values())} array(s)"
        f"{f', {nested_arrays} nested' if nested_arrays else ''})"
    )
    return summary


def compare_model(ref: Reference, reader: Any, report: dict[str, Any], diffs: list[str]) -> None:
    for json_key, (metadata_key, expected_type) in MODEL_FIELDS.items():
        field = reader.fields.get(metadata_key)
        if field is None:
            expected = None
        else:
            expected = field.contents()
            if expected_type == "string" and not isinstance(expected, str):
                expected = None
            elif expected_type == "u32" and not isinstance(expected, int):
                expected = None
        actual = report["model"].get(json_key)
        if expected != actual:
            add_diff(diffs, f"model.{json_key}", expected, actual)


def compare_tensors(ref: Reference, reader: Any, report: dict[str, Any], diffs: list[str]) -> str:
    ref_tensors = reader.tensors
    entries = report["tensors"]["entries"]
    compare_int(diffs, "tensors.count", len(ref_tensors), report["tensors"]["count"])
    if len(entries) != len(ref_tensors):
        add_diff(diffs, "tensors.entries length", len(ref_tensors), len(entries))

    # Documented rule: tensors are sorted by offset then name.
    ordering = [(entry.get("offset"), entry.get("name")) for entry in entries]
    if ordering != sorted(ordering):
        add_diff(diffs, "tensors.entries order", "sorted by (offset, name)", ordering)

    hoard_by_name = {entry.get("name"): entry for entry in entries}
    ref_by_name = {tensor.name: tensor for tensor in ref_tensors}
    if set(hoard_by_name) != set(ref_by_name):
        add_diff(diffs, "tensors.names", sorted(ref_by_name), sorted(hoard_by_name))

    data_start = int(reader.data_offset)
    unknown_ids = 0
    type_counts: dict[str, int] = {}
    for ref_tensor in ref_tensors:
        name = ref_tensor.name
        entry = hoard_by_name.get(name)
        if entry is None:
            continue
        type_id = int(ref_tensor.tensor_type)
        type_name = ref.quant_type_name(type_id)
        if type_name == "unknown":
            unknown_ids += 1
        else:
            type_counts[type_name] = type_counts.get(type_name, 0) + 1
        path = f"tensors[{name!r}]"
        compare_int(diffs, f"{path}.type_id", type_id, entry.get("type_id"))
        if entry.get("type") != type_name:
            add_diff(diffs, f"{path}.type", type_name, entry.get("type"))
        expected_shape = [int(dim) for dim in ref_tensor.shape.tolist()]
        if entry.get("shape") != expected_shape:
            add_diff(diffs, f"{path}.shape", expected_shape, entry.get("shape"))
        compare_int(diffs, f"{path}.elements", int(ref_tensor.n_elements), entry.get("elements"))
        if type_name == "unknown":
            if entry.get("bytes") is not None:
                add_diff(diffs, f"{path}.bytes", None, entry.get("bytes"))
        else:
            compare_int(diffs, f"{path}.bytes", int(ref_tensor.n_bytes), entry.get("bytes"))
        data_offset = int(ref_tensor.data_offset)
        compare_int(diffs, f"{path}.file_offset", data_offset, entry.get("file_offset"))
        compare_int(diffs, f"{path}.offset", data_offset - data_start, entry.get("offset"))
        # Internal consistency of the absolute offset.
        if isinstance(entry.get("offset"), int) and isinstance(entry.get("file_offset"), int):
            if data_start + entry["offset"] != entry["file_offset"]:
                add_diff(
                    diffs,
                    f"{path}.file_offset consistency",
                    data_start + entry["offset"],
                    entry["file_offset"],
                )

    types = ", ".join(f"{name} x{count}" for name, count in sorted(type_counts.items())) or "none"
    return f"tensors: {len(ref_tensors)} match ({types}; unknown type ids: {unknown_ids})"


def compare_aggregates(ref: Reference, reader: Any, report: dict[str, Any], diffs: list[str]) -> str:
    ref_tensors = reader.tensors
    groups: dict[str, list[int]] = {}
    unknown_tensors = 0
    for tensor in ref_tensors:
        type_name = ref.quant_type_name(int(tensor.tensor_type))
        if type_name == "unknown":
            unknown_tensors += 1
            continue
        group = groups.setdefault(type_name, [0, 0, 0])
        group[0] += 1
        group[1] += int(tensor.n_elements)
        group[2] += int(tensor.n_bytes)
    total_bytes = sum(group[2] for group in groups.values())

    mix = report["quant_mix"]
    mix_by_name = {entry.get("type"): entry for entry in mix}
    if set(mix_by_name) != set(groups):
        add_diff(diffs, "quant_mix.types", sorted(groups), sorted(mix_by_name))
    for name, (count, elements, byte_count) in sorted(groups.items()):
        entry = mix_by_name.get(name)
        if entry is None:
            continue
        compare_int(diffs, f"quant_mix[{name}].tensors", count, entry.get("tensors"))
        compare_int(diffs, f"quant_mix[{name}].elements", elements, entry.get("elements"))
        compare_int(diffs, f"quant_mix[{name}].bytes", byte_count, entry.get("bytes"))
        expected_percent = (
            ((byte_count * 10_000 + total_bytes // 2) // total_bytes) / 100.0 if total_bytes else 0.0
        )
        if entry.get("percent") != expected_percent:
            add_diff(diffs, f"quant_mix[{name}].percent", expected_percent, entry.get("percent"))

    # Memory accounting, including alignment gaps between tensors (and before
    # the first one) computed from the reference offsets and byte sizes.
    sorted_tensors = sorted(ref_tensors, key=lambda tensor: (int(tensor.data_offset), tensor.name))
    padding = 0
    previous_end = 0
    for tensor in sorted_tensors:
        offset = int(tensor.data_offset) - int(reader.data_offset)
        padding += max(0, offset - previous_end)
        previous_end = offset + int(tensor.n_bytes)

    memory = report["memory"]
    compare_int(diffs, "memory.tensor_bytes", total_bytes, memory.get("tensor_bytes"))
    compare_int(diffs, "memory.file_bytes", int(reader.data.nbytes), memory.get("file_bytes"))
    compare_int(diffs, "memory.overhead_bytes", int(reader.data.nbytes) - total_bytes, memory.get("overhead_bytes"))
    compare_int(diffs, "memory.padding_bytes", padding, memory.get("padding_bytes"))
    return (
        f"quant_mix/memory: match (tensor_bytes={total_bytes}, padding={padding}, "
        f"unknown-type tensors={unknown_tensors})"
    )


def check_file(path: Path, ref: Reference, hoard_bin: Path) -> tuple[list[str], list[str]]:
    """Run both parsers on one file and return (differences, summary lines)."""
    if not path.is_file():
        raise OracleError(f"{path}: no such file")
    proc = subprocess.run(
        [str(hoard_bin), "inspect", "--json", str(path)],
        capture_output=True,
        text=True,
        timeout=300,
        check=False,
    )
    if proc.returncode != 0:
        raise OracleError(
            f"hoard-gguf inspect failed with exit code {proc.returncode}: {proc.stderr.strip()}"
        )
    try:
        report = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise OracleError(f"hoard-gguf did not print valid JSON: {exc}") from exc
    try:
        reader = ref.GGUFReader(str(path))
    except Exception as exc:  # noqa: BLE001 - the reference parser has many exception types
        raise OracleError(
            f"reference gguf-py parser rejected the file: {type(exc).__name__}: {exc}"
        ) from exc

    diffs: list[str] = []
    if report.get("schema") != 1:
        add_diff(diffs, "schema", 1, report.get("schema"))
    compare_int(diffs, "file.size", path.stat().st_size, report["file"].get("size"))
    compare_int(diffs, "file.version", int(reader.fields["GGUF.version"].contents()), report["file"].get("version"))
    expected_byte_order = "little" if reader.byte_order == "I" else "big"
    if report["file"].get("byte_order") != expected_byte_order:
        add_diff(diffs, "file.byte_order", expected_byte_order, report["file"].get("byte_order"))
    compare_int(diffs, "file.alignment", int(reader.alignment), report["file"].get("alignment"))
    compare_int(diffs, "file.data_start", int(reader.data_offset), report["file"].get("data_start"))
    compare_int(diffs, "tensors.count", int(reader.fields["GGUF.tensor_count"].contents()), report["tensors"].get("count"))

    notes = [
        f"file: GGUF v{int(reader.fields['GGUF.version'].contents())} "
        f"({expected_byte_order}-endian), alignment {int(reader.alignment)}, "
        f"data_start {int(reader.data_offset)}, size {path.stat().st_size}",
        compare_metadata(ref, reader, report, diffs),
        compare_model(ref, reader, report, diffs),
        compare_tensors(ref, reader, report, diffs),
        compare_aggregates(ref, reader, report, diffs),
    ]
    return diffs, notes


def parse_args(argv: list[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Compare hoard-gguf inspect --json with the pinned llama.cpp gguf-py reader."
    )
    parser.add_argument("--llama-cpp-tree", required=True, type=Path, help="directory of the pinned llama.cpp tree (contains gguf-py/)")
    parser.add_argument("--hoard-gguf", required=True, type=Path, help="path to the hoard-gguf binary")
    parser.add_argument("--commit", default=os.environ.get("LLAMA_CPP_COMMIT", "unknown"), help="pinned llama.cpp commit (for reporting)")
    parser.add_argument("files", nargs="+", type=Path, help="GGUF files to compare")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        ref = load_reference(args.llama_cpp_tree)
    except OracleError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    print(f"reference: llama.cpp gguf-py @ {args.commit}")
    print(f"reference import: {ref.description}")
    print(f"hoard-gguf: {args.hoard_gguf}")
    print()

    failures = 0
    for path in args.files:
        print(f"== {path}")
        try:
            diffs, notes = check_file(path, ref, args.hoard_gguf)
        except OracleError as exc:
            print(f"   FAIL: {exc}")
            failures += 1
            print()
            continue
        for note in notes:
            if note:
                print(f"   {note}")
        if diffs:
            failures += 1
            print(f"   FAIL: {len(diffs)} difference(s)")
            for diff in diffs:
                print(f"    {diff}")
        else:
            print("   PASS: all compared fields match")
        print()

    total = len(args.files)
    print(f"{total - failures}/{total} file(s) matched")
    return 0 if failures == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
