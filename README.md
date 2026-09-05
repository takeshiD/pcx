# pcx

[Documentation](https://takeshid.github.io/pcx/) · [日本語](./README.ja.md)

`pcx` is a shell-native toolbox for inspecting and reducing point-cloud recordings on edge Linux systems.

> **Project status:** active development. The executable provides MCAP inspection, one-frame PCD extraction, and faithful one-message MCAP passthrough.

## Why pcx?

Large sensor recordings often live on robots and industrial PCs where a desktop viewer is unavailable and copying the whole file is wasteful. `pcx` is designed around one workflow:

```text
inspect -> reduce -> transfer with existing shell tools
```

The product aims to remain a single executable with bounded memory, binary-safe stdout, actionable stderr diagnostics, and no ROS runtime, GUI, daemon, AWS, or S3 client.

## Availability

| Capability                                         | Status                |
| ---                                                | ---                   |
| `pcx --help`, `pcx --version`                      | Available             |
| `pcx info` MCAP metadata                           | Available             |
| MCAP Topic listing with human and JSON output      | Available             |
| ROS 2 `PointCloud2` frame extraction               | Available             |
| Binary and ASCII PCD output                        | Available             |
| Strict PCD reader (CLI integration later)          | Available internally  |
| Encoded one-message MCAP passthrough               | Available             |
| Bounded synchronous LAS/LAZ library I/O            | Available             |
| Crop, field selection, frame-local voxel reduction | Planned               |
| PLY scalar-vertex adapter (CLI integration later)  | Available internally  |
| CPU projection, terminal selection, and Unicode    | Available internally  |
| LAS/LAZ CLI commands, Kitty, and Sixel              | Planned               |
| AWS/S3 upload and cloud credentials                | Out of scope          |
| macOS and Windows support                          | Undecided future work |

## Install

Until the first crates.io release, build from source:

```bash
git clone https://github.com/takeshiD/pcx.git
cd pcx
cargo install --path . --locked
pcx --version
```

The planned registry command is:

```bash
cargo install pcx-cli --locked
```

With Nix:

```bash
nix run github:takeshiD/pcx -- --version
nix develop github:takeshiD/pcx
```

## v0.1 workflow

MCAP metadata inspection, Topic discovery, and one-frame extraction are available now.

```bash
pcx info run.mcap
pcx topics run.mcap --json
pcx extract run.mcap \
  --topic /lidar/points \
  --frame 0 \
  -o frame.pcd
pcx passthrough run.mcap \
  --topic /lidar/points \
  --frame 0 \
  --compression zstd \
  -o selected.mcap
```

Choose exactly one of `--frame INDEX` and `--at DURATION`. Binary PCD is the
default; pass `--encoding ascii` for text PCD. `--memory-limit BYTES` is a hard
managed-memory budget. Output must be an explicit path or `-` for stdout.

`pcx passthrough` applies the same selector without decoding point fields. It
preserves the selected encoded Message and its Channel/Schema relationship,
plus recording-level attachments, metadata, and application-private records.
Derived container structure, statistics, and CRCs are regenerated
deterministically; attachment and metadata indexes are omitted to bound memory.

Transfer remains the shell's job:

```bash
ssh robot 'pcx extract /data/run.mcap --topic /lidar/points --frame 0 -o -' \
  > frame.pcd
```

## Architecture

The implementation is one publishable Rust package with deep internal modules. Encoded container records remain separate from decoded point frames:

```text
CLI -> JobSpec -> Planner -> Executor
                              |
                 +------------+-------------+
                 |                          |
        container passthrough        semantic pipeline
                                            |
                            CDR -> PointView/PointBatch
                                            |
                                    operator -> encoder
```

See the [architecture document](./docs/ARCHITECTURE.md), [test strategy](./docs/TESTING.md), [implementation roadmap](./docs/ROADMAP.md), [domain language](./CONTEXT.md), and [decision records](./docs/adr/).

## Development

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo check --all-targets --all-features
cargo test --all-features
nix flake check
```

The Starlight documentation lives in `docs/pages`:

```bash
npm --prefix docs/pages ci
npm --prefix docs/pages run check
npm --prefix docs/pages run build
```

## Supported systems

- `x86_64-linux`: native build and full test suite
- `aarch64-linux`: native build and full test suite
- macOS and Windows: not currently supported; future support is undecided

## Contributing

Start with [CONTRIBUTING.md](./CONTRIBUTING.md). Security-sensitive reports should follow [SECURITY.md](./SECURITY.md).

## License

MIT © 2026 tkcd. See [LICENSE](./LICENSE).
