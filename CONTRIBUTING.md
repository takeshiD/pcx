# Contributing to pcx

Thank you for helping build `pcx`. The project is in its foundation stage, so discuss changes that alter command semantics, data fidelity, or module boundaries in an issue before implementation.

## Development environment

Use either Nix:

```bash
nix develop
```

or a Rust stable toolchain with `rustfmt` and `clippy`. Documentation additionally requires Node.js and npm.

## Development workflow

Follow the repository-wide agent and contribution rules in [AGENTS.md](./AGENTS.md), including the required one-issue/one-branch/one-worktree/one-pull-request workflow and naming conventions.

## Required checks

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo check --all-targets --all-features
cargo test --all-features
./scripts/generate-assets.sh --check
nix flake check
npm --prefix docs/pages ci
npm --prefix docs/pages run check
npm --prefix docs/pages run build
```

Shell completions and manual pages under `generated/` come from the clap
grammar and package version. Run `./scripts/generate-assets.sh` after changing
either, and review the generated diff before committing it. CI only checks for
stale files; it never updates them.

## Design rules

- Keep format-specific behavior out of CLI handlers.
- Never load a complete recording into managed memory.
- Never silently drop point fields or temporal/spatial metadata.
- Keep stdout binary-safe and diagnostics on stderr.
- Preserve the distinction between a user-facing Topic, an MCAP Channel, and an internal record stream.
- Add malformed-input tests with every parser fix.

Read [CONTEXT.md](./CONTEXT.md), [docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md), and the [ADRs](./docs/adr/) before architectural changes.

## Pull requests

Keep each pull request focused. Include the issue reference, scope and non-scope, tests added, and any user-visible compatibility impact. Golden files are never updated automatically in CI; review their diffs explicitly.
