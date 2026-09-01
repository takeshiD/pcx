# Publish one Rust package with deep modules

The Rust implementation will be one publishable package named `pcx-cli` with library and binary targets, and the installed executable remains `pcx`. Core processing, MCAP, ROS 2 message handling, and PCD live in deep modules behind `src/lib.rs` rather than separate workspace packages, preserving test seams while allowing `cargo install pcx-cli` without publishing internal implementation crates; the unavailable crates.io name `pcx` prevents using `cargo install pcx`.
