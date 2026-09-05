---
title: コマンド設計
description: 現在の機能と合意済み v0.1 インターフェース。
---

## 現在利用可能

```bash
pcx --help
pcx --version
pcx info INPUT.mcap [--json]
pcx topics INPUT.mcap [--json]
pcx extract INPUT.mcap --topic TOPIC (--frame INDEX | --at DURATION) \
  -o OUTPUT.pcd|- [--encoding binary|ascii] [--memory-limit BYTES] [--force]
```

`pcx info`はPoint FrameをdecodeせずにMCAP Sourceをstreamingで調査します。human outputとversion付きJSONはstdoutへ出力され、成功時のstderrは空です。

`pcx topics`は各MCAP Channelについて、user-facingなTopic、Schema、encoding、message count、metadataに基づくROS 2 PointCloud2 candidate statusを表示します。candidate statusはmessage payloadのdecodeやvalidation成功を意味しません。

## 1 frameの抽出

```bash
pcx extract INPUT.mcap --topic TOPIC (--frame INDEX | --at DURATION) \
  -o OUTPUT.pcd|- [--encoding binary|ascii] [--memory-limit BYTES] [--force]
```

`--frame`は選択Topicに一致するmessage内の0-based indexです。`--at`はrecording開始から`83.2s`のようなduration以降で最初のframeを選びます。selectorの一方とfileまたはstdout sinkを明示します。binary PCDがdefaultです。Topic不在、範囲外、破損message、memory budgetを保証できない処理は、outputを確定する前に失敗します。

診断は stderr、データと JSON は stdout に出力します。JSON は `schema_version` を持ちます。既存ファイルは `--force` なしでは上書きせず、割り込み時は一時ファイルを除去して `130` を返します。
