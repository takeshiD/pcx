---
title: 開発状況とroadmap
description: 現在利用できる機能と今後の順序。
---

## Foundation — 利用可能

- `pcx` binaryを持つpublish可能な`pcx-cli` package;
- `--help`と`--version`;
- boundedな同期MCAP readと`pcx info` metadata出力;
- architecture、domain language、ADR、test strategy;
- Nix、CI、Pages、release automationの基盤。

## v0.1 MCAP vertical slice — 利用可能

1. Topic、Channel、Schema、message countのdiscovery;
2. `sensor_msgs/msg/PointCloud2`向けstrict ROS 2 CDR decode;
3. 可能な経路でzero-copy `PointView`;
4. indexまたはMCAP log timeによるPoint Frame選択;
5. binary／ASCII PCD出力。

## その後

- **Reduction:** field選択、crop、stats、frame単位voxel。
- **利用可能なFormat追加:** faithfulな1 message encoded MCAP passthrough。
- **利用可能なFormat追加:** strict ASCII／little-endian binary PCD reader adapter。
- **Formats:** PCD input CLI integration、PLY CLI integration、LAS/LAZ。

scalar-vertex PLY reader/writer adapter は共通 schema の背後に実装済みですが、
user-facing command はまだ公開していません。
- **Terminal rendering:** CPU rasterizer と Unicode backend は内部実装済み。capability selection、Kitty、Sixel は後続。
- **Hardening:** fuzz、benchmark、musl調査、resource tuning。

## 明示的な非目標

AWS/S3、cloud credential、network listener、daemon、desktop GUI、ROS/PCL/PDAL runtime、SLAM、meshing、ML inferenceは対象外です。
