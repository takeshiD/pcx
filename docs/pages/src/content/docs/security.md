---
title: Security
description: Threat boundary and responsible disclosure.
---

MCAP, CDR and point-cloud files are untrusted input. Parsers use checked arithmetic, validate all offsets and lengths, bound allocations before execution and reject unsupported layouts. `pcx` does not own cloud credentials and does not contain a network client.

Do not disclose a suspected vulnerability in a public issue. Follow the private reporting instructions in the repository [security policy](https://github.com/takeshiD/pcx/blob/main/SECURITY.md).
