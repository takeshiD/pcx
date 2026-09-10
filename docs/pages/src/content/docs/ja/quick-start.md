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

## encoded messageをMCAPへcopy

```bash
pcx passthrough run.mcap --topic /lidar/points --frame 0 -o selected.mcap
```

このcontainer pathはPointCloud2をdecodeせず、encoded messageとrecording-level
recordを保持します。

## 1 Point Frameをterminalへrender

```bash
pcx render run.mcap --topic /lidar/points --frame 0
```

defaultの`--backend auto`はconservativeにfallbackし、現在のprocess queryは
Kitty／Sixelをautomaticには許可しません。interactiveなtext-cell renderingを固定するには
`--backend unicode`を使います。stdoutをredirectした場合、automatic outputは
ANSI／graphics protocol escapeを含まないdeterministicなmonochrome Unicodeです。

## Static Cloudをrender

```bash
pcx render tests/fixtures/valid/pointcloud2-ascii.pcd --width 32 --height 12
pcx render tests/fixtures/valid/las-pdal.las --width 32 --height 12
pcx render tests/fixtures/valid/las-pdal.laz --width 32 --height 12
```

Static Cloudには`--topic`、`--frame`、`--at`を指定しません。対応するPCD subsetと
LAS/LAZを読み取り、MCAP renderingと同じbounded projection／terminal output policyを
適用します。LAS/LAZはglobalな範囲へ一度だけfitし、宣言されたcloud全体が
`--memory-limit`に収まる必要があります。

## PNG snapshotを書く

```bash
pcx snapshot run.mcap --topic /lidar/points --frame 0 -o frame.png
```

PNGはempty pixelをtransparentにしたprojection済みRGBA8可視化で、depth mapや
losslessな点群fileではありません。

cloud clientではなくshellで転送します。

```bash
ssh robot 'pcx extract /data/run.mcap --topic /lidar/points --frame 0 -o -' > frame.pcd
```
