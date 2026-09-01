# Prove the MCAP workflow in v0.1

The first releasable slice will prove the product's edge-diagnostics promise with MCAP `info`, channel listing, and extraction of one ROS 2 `PointCloud2` frame to PCD. Static format breadth, voxel reduction, rendering, and LAS/LAZ support follow this slice rather than defining it, because a PCD/PLY-only release would not validate the central `inspect -> reduce -> transfer` workflow. Transfer itself is composed from existing shell tools and cloud-specific clients remain out of scope under ADR-0009.

Only the CLI is a public compatibility surface during v0.x. Internal Rust modules and library APIs may evolve without a stability promise; the single package is published for installation of its `pcx` binary rather than as a supported Rust library API.
