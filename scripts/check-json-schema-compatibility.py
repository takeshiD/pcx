#!/usr/bin/env python3
"""Validate pcx JSON schemas/goldens and reject breaking in-version edits."""

from __future__ import annotations

import json
import pathlib
import subprocess
import sys
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[1]
SCHEMA_ROOT = ROOT / "docs/json-schema"
GOLDEN_ROOT = ROOT / "tests/golden/json"
EXPECTED_V1_SCHEMAS = {"error.schema.json", "info.schema.json", "topics.schema.json"}


class ContractError(Exception):
    pass


def load(path: pathlib.Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ContractError(f"{path.relative_to(ROOT)}: {error}") from error


def json_type(value: Any, expected: str) -> bool:
    return {
        "null": value is None,
        "object": isinstance(value, dict),
        "array": isinstance(value, list),
        "string": isinstance(value, str),
        "boolean": isinstance(value, bool),
        "integer": isinstance(value, int) and not isinstance(value, bool),
        "number": isinstance(value, (int, float)) and not isinstance(value, bool),
    }[expected]


def validate_instance(value: Any, schema: dict[str, Any], location: str) -> None:
    expected_types = schema.get("type")
    if expected_types is not None:
        if isinstance(expected_types, str):
            expected_types = [expected_types]
        if not any(json_type(value, expected) for expected in expected_types):
            raise ContractError(f"{location}: expected type {expected_types}, got {type(value).__name__}")
    if "const" in schema and value != schema["const"]:
        raise ContractError(f"{location}: expected constant {schema['const']!r}")
    if "enum" in schema and value not in schema["enum"]:
        raise ContractError(f"{location}: {value!r} is outside {schema['enum']!r}")
    if "minimum" in schema and isinstance(value, (int, float)) and not isinstance(value, bool):
        if value < schema["minimum"]:
            raise ContractError(f"{location}: {value} is below minimum {schema['minimum']}")
    if isinstance(value, dict):
        for required in schema.get("required", []):
            if required not in value:
                raise ContractError(f"{location}: missing required property {required!r}")
        properties = schema.get("properties", {})
        for name, child in properties.items():
            if name in value:
                validate_instance(value[name], child, f"{location}.{name}")
        if schema.get("additionalProperties") is False:
            extras = value.keys() - properties.keys()
            if extras:
                raise ContractError(f"{location}: unexpected properties {sorted(extras)!r}")
    if isinstance(value, list) and "items" in schema:
        for index, item in enumerate(value):
            validate_instance(item, schema["items"], f"{location}[{index}]")


def validate_additive_objects(schema: Any, location: str) -> None:
    if isinstance(schema, dict):
        if "properties" in schema and schema.get("additionalProperties") is not True:
            raise ContractError(
                f"{location}: object schemas must set additionalProperties to true "
                "so version-1 consumers accept additive fields"
            )
        for name, child in schema.items():
            validate_additive_objects(child, f"{location}.{name}")
    elif isinstance(schema, list):
        for index, child in enumerate(schema):
            validate_additive_objects(child, f"{location}[{index}]")


def validate_schema(path: pathlib.Path, schema: dict[str, Any]) -> None:
    relative = path.relative_to(ROOT)
    if schema.get("$schema") != "https://json-schema.org/draft/2020-12/schema":
        raise ContractError(f"{relative}: must declare JSON Schema draft 2020-12")
    if schema.get("type") != "object":
        raise ContractError(f"{relative}: top-level schema must describe an object")
    version = int(path.parent.name.removeprefix("v"))
    actual_version = schema.get("properties", {}).get("schema_version", {}).get("const")
    if actual_version != version:
        raise ContractError(f"{relative}: schema_version const must be {version}")
    required = set(schema.get("required", []))
    if not {"schema_version", "command"} <= required:
        raise ContractError(f"{relative}: schema_version and command must be required")
    validate_additive_objects(schema, str(relative))


def compare_additive(old: Any, new: Any, location: str) -> None:
    if not isinstance(old, dict) or not isinstance(new, dict):
        if old != new:
            raise ContractError(f"{location}: existing schema value changed from {old!r} to {new!r}")
        return

    for keyword in ("$schema", "$id", "type", "const", "enum", "minimum", "maximum"):
        if keyword in old and new.get(keyword) != old[keyword]:
            raise ContractError(f"{location}: existing {keyword} changed")

    old_required = set(old.get("required", []))
    new_required = set(new.get("required", []))
    if not old_required <= new_required:
        raise ContractError(f"{location}: required properties removed: {sorted(old_required - new_required)}")

    old_properties = old.get("properties", {})
    new_properties = new.get("properties", {})
    for name, old_child in old_properties.items():
        if name not in new_properties:
            raise ContractError(f"{location}: property {name!r} was removed")
        compare_additive(old_child, new_properties[name], f"{location}.{name}")

    if "items" in old:
        if "items" not in new:
            raise ContractError(f"{location}: array item contract was removed")
        compare_additive(old["items"], new["items"], f"{location}[]")


def load_from_git(revision: str, relative: pathlib.Path) -> dict[str, Any] | None:
    result = subprocess.run(
        ["git", "show", f"{revision}:{relative.as_posix()}"],
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        return None
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise ContractError(f"{revision}:{relative}: {error}") from error


def validate_current() -> dict[pathlib.Path, dict[str, Any]]:
    schema_paths = sorted(SCHEMA_ROOT.glob("v*/*.schema.json"))
    if not schema_paths:
        raise ContractError("no versioned JSON schemas found")
    v1_names = {path.name for path in schema_paths if path.parent.name == "v1"}
    if v1_names != EXPECTED_V1_SCHEMAS:
        raise ContractError(f"v1 schema inventory is {sorted(v1_names)!r}, expected {sorted(EXPECTED_V1_SCHEMAS)!r}")

    schemas = {path.relative_to(ROOT): load(path) for path in schema_paths}
    for relative, schema in schemas.items():
        validate_schema(ROOT / relative, schema)

    golden_mapping = {
        "info.json": "info.schema.json",
        "topics.json": "topics.schema.json",
        "info-error.json": "error.schema.json",
        "topics-error.json": "error.schema.json",
    }
    actual_goldens = {path.name for path in (GOLDEN_ROOT / "v1").glob("*.json")}
    if actual_goldens != golden_mapping.keys():
        raise ContractError(f"v1 golden inventory is {sorted(actual_goldens)!r}, expected {sorted(golden_mapping)!r}")
    for golden_name, schema_name in golden_mapping.items():
        golden = load(GOLDEN_ROOT / "v1" / golden_name)
        schema = schemas[pathlib.Path("docs/json-schema/v1") / schema_name]
        validate_instance(golden, schema, f"tests/golden/json/v1/{golden_name}")
    return schemas


def compare_with_base(revision: str, current: dict[pathlib.Path, dict[str, Any]]) -> None:
    if not revision or set(revision) == {"0"}:
        return
    result = subprocess.run(
        ["git", "ls-tree", "-r", "--name-only", revision, "docs/json-schema"],
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise ContractError(f"could not inspect base revision {revision}: {result.stderr.strip()}")
    for name in result.stdout.splitlines():
        if not name.endswith(".schema.json"):
            continue
        relative = pathlib.Path(name)
        old = load_from_git(revision, relative)
        if old is None:
            raise ContractError(f"could not read {revision}:{relative}")
        if relative not in current:
            raise ContractError(f"{relative}: versioned schema was removed")
        compare_additive(old, current[relative], str(relative))


def main() -> int:
    try:
        current = validate_current()
        if len(sys.argv) > 1:
            compare_with_base(sys.argv[1], current)
    except ContractError as error:
        print(f"JSON schema compatibility error: {error}", file=sys.stderr)
        return 1
    print("JSON schemas and reviewed goldens are compatible")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
