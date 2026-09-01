## Outcome

<!-- What user-visible or engineering outcome does this change deliver? -->

## Design and scope

- Related issue:
- ADR added or updated, if needed:
- Explicitly out of scope:

## Verification

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --all-features --locked -- -D warnings`
- [ ] `cargo check --all-targets --all-features --locked`
- [ ] `cargo test --all-features --locked`
- [ ] Documentation and Nix checks relevant to this change pass
- [ ] Malformed/untrusted input behavior is tested where applicable
- [ ] No silent field or metadata loss was introduced
