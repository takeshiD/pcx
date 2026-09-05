# JSON compatibility contract

The schemas in `v1/` are the normative contract for every current `pcx`
machine-readable command. `info --json` and `topics --json` each own a success
schema. Both commands use `error.schema.json` after argument parsing when
execution fails. `extract` intentionally emits PCD data and has no JSON mode or
JSON schema.

Successful JSON is written to stdout. Structured error JSON is written to
stderr and stdout remains empty. Every object has this top-level version and
command context:

```json
{
  "schema_version": 1,
  "command": "info"
}
```

## Version policy

Within `schema_version: 1`, compatibility changes are additive only. A new
property may be added, and consumers must ignore properties they do not
recognize. Existing properties, required guarantees, types, constant values,
and enum values may not be removed or changed. A change that cannot obey those
rules requires a new schema-version directory and a corresponding
`schema_version` value in the CLI.

The error `category` values and their meanings are contractual. The `message`
property supplies human-readable context, but its exact wording is not stable.
Human command output, help text, and non-JSON diagnostics are not part of this
contract.

## Review and CI

Reviewed examples live in `tests/golden/json/v1/`. Process-level tests compare
real CLI output with those examples. Volatile diagnostic message text is
normalized before comparison. Golden files are changed only through an
ordinary reviewed source diff; CI never rewrites them.

`scripts/check-json-schema-compatibility.py` validates the current schemas and
goldens. In pull requests it also compares versioned schemas with the base
revision and rejects destructive changes. Run the local check with:

```bash
nix develop --command python3 scripts/check-json-schema-compatibility.py
```
