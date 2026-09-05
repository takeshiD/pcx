---
title: Quick start
description: MCAP metadata調査と1 Point Frameの抽出。
---

## 現在利用可能

```bash
pcx --help
pcx --version
pcx info run.mcap
```

## 1 Point Frameを抽出

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
