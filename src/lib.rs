//! Library implementation for the `pcx` command-line application.
//!
//! The Rust API is internal during the 0.x series. The supported interface is
//! the `pcx` executable and its documented machine-readable output.

pub mod cli;
pub mod core;
pub mod mcap;
pub mod ops;
pub mod pcd;
pub mod ros2;
