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

## v0.1 MCAP vertical slice — 計画中

1. Topic一覧;
2. `sensor_msgs/msg/PointCloud2`向けstrict ROS 2 CDR decode;
3. 可能な経路でzero-copy `PointView`;
4. indexまたはMCAP log timeによるPoint Frame選択;
5. binary／ASCII PCD出力。

## その後

- **Reduction:** field選択、crop、stats、frame単位voxel。
- **Formats:** PCD入力、PLY、LAS/LAZ、reduced MCAP出力。
- **Terminal rendering:** CPU rasterizer、Unicode、Kitty。Sixelは後続。
- **Hardening:** fuzz、benchmark、musl調査、resource tuning。

## 明示的な非目標

AWS/S3、cloud credential、network listener、daemon、desktop GUI、ROS/PCL/PDAL runtime、SLAM、meshing、ML inferenceは対象外です。
