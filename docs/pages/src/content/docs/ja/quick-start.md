---
title: Quick start
description: MCAP metadata調査と、残りのv0.1 workflow。
---

## 現在利用可能

```bash
pcx --help
pcx --version
pcx info run.mcap
```

## v0.1で予定

```bash
pcx topics run.mcap --json
pcx extract run.mcap \
  --topic /lidar/points \
  --frame 0 \
  -o frame.pcd
```

`--frame`はTopic選択後の0-based indexです。`--at 83.2s`はrecording開始からの指定時間以降で最初のPoint Frameを選びます。両方を同時には指定できません。

cloud clientではなくshellで転送します。

```bash
ssh robot 'pcx extract /data/run.mcap --topic /lidar/points --frame 0 -o -' > frame.pcd
```
