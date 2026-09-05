//! Narrow ROS 2 serialization support.
//!
//! This module intentionally contains only the CDR machinery needed by the
//! ROS 2 adapters in this crate. It is not a dynamic ROS message decoder.

pub mod cdr;
pub mod pointcloud2;
