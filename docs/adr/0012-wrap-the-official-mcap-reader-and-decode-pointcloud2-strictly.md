# Wrap the official MCAP reader and decode PointCloud2 strictly

`pcx` will adapt the official MIT-licensed Rust `mcap` crate's sans-IO reader to bounded synchronous `Read + Seek` access rather than implementing the container or requiring whole-file buffering. Zstandard and LZ4 remain enabled for real-world recordings despite their cross-build cost, while a small strict CDR decoder handles only ROS 2 `sensor_msgs/msg/PointCloud2` without a ROS runtime or a general dynamic message engine; the v0.1 package exposes one consistent feature set across Cargo, Nix, Cachix, and release binaries.
