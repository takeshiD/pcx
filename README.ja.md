# pcx

[ドキュメント](https://takeshid.github.io/pcx/ja/) · [English](./README.md)

`pcx`は、edge Linux上にある点群recordingをshellから調査・縮小するためのtoolboxです。

> **開発状況:** 現在は基盤構築段階です。実行ファイルが提供するのは`--help`と`--version`だけで、以下のMCAP・点群commandはv0.1向けの計画であり、まだ実装されていません。

## なぜpcxか

robotやindustrial PC上の巨大なsensor recordingを、内容が分からないままworkstationへ全量転送するのは非効率です。`pcx`は次のworkflowを中心に設計されています。

```text
inspect -> reduce -> 既存のshell toolでtransfer
```

bounded memory、binary-safeなstdout、stderrへの明確な診断を備えた単一実行ファイルを目指します。ROS runtime、GUI、daemon、AWS、S3 clientは持ちません。

## 機能状況

| 機能 | 状態 |
| --- | --- |
| `pcx --help`, `pcx --version` | 利用可能 |
| MCAP metadata／topic一覧 | v0.1で予定 |
| ROS 2 `PointCloud2` frame抽出 | v0.1で予定 |
| binary／ASCII PCD出力 | v0.1で予定 |
| crop、field選択、frame単位voxel | 計画中 |
| PLY、LAS/LAZ、terminal rendering | 計画中 |
| AWS/S3 upload、cloud credential | 対象外 |
| macOS／Windows | 将来候補・時期未定 |

## Install

最初のcrates.io releaseまではsourceからinstallします。

```bash
git clone https://github.com/takeshiD/pcx.git
cd pcx
cargo install --path . --locked
pcx --version
```

将来のregistry install commandは次の予定です。

```bash
cargo install pcx-cli --locked
```

Nixを利用する場合:

```bash
nix run github:takeshiD/pcx -- --version
nix develop github:takeshiD/pcx
```

## v0.1で予定するworkflow

次のinterfaceは確定した設計ですが、現在はまだ利用できません。

```bash
pcx info run.mcap
pcx topics run.mcap --json
pcx extract run.mcap \
  --topic /lidar/points \
  --frame 0 \
  -o frame.pcd
```

transferは既存のshell toolに委ねます。

```bash
ssh robot 'pcx extract /data/run.mcap --topic /lidar/points --frame 0 -o -' \
  > frame.pcd
```

## Architecture

実装は一つのpublish可能なRust packageとし、内部をdeep moduleで分離します。encoded container recordとdecoded point frameは別のpipelineとして扱います。

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

[Architecture](./docs/ARCHITECTURE.md)、[テスト戦略](./docs/TESTING.md)、[実装ロードマップ](./docs/ROADMAP.md)、[用語集](./CONTEXT.md)、[ADR](./docs/adr/)も参照してください。

## Development

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo check --all-targets --all-features
cargo test --all-features
nix flake check
```

Starlight documentationは`docs/pages`にあります。

```bash
npm --prefix docs/pages ci
npm --prefix docs/pages run check
npm --prefix docs/pages run build
```

## 対応system

- `x86_64-linux`: native buildと全test
- `aarch64-linux`: native buildと全test
- macOS／Windows: 現在は非対応、将来対応は未定

## Contributing

[CONTRIBUTING.md](./CONTRIBUTING.md)を参照してください。security上の問題は[SECURITY.md](./SECURITY.md)の手順で報告してください。

## License

MIT © 2026 tkcd。詳細は[LICENSE](./LICENSE)を参照してください。
