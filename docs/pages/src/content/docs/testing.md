---
title: Testing
description: Confidence model for parsers, planners and CLI behavior.
---

The test suite is layered around the cost of failure.

1. Unit tests cover checked arithmetic, schema validation, selection and planners.
2. Fixture tests cover minimal MCAP, CDR and PCD examples, including malformed inputs.
3. Property tests exercise layouts, budgets and encode/decode invariants.
4. CLI integration tests assert exit codes, stdout/stderr separation and atomic files.
5. End-to-end tests run one-frame MCAP-to-PCD conversion once the v0.1 pipeline exists.
6. Scheduled fuzzing targets untrusted parsers without requiring network services.

The CI floor is formatting, Clippy, type checking and unit/integration tests on native x86_64 and aarch64 Linux runners. See the complete [test strategy](https://github.com/takeshiD/pcx/blob/main/docs/TESTING.md).
