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
pcx passthrough INPUT.mcap --topic TOPIC (--frame INDEX | --at DURATION) \
  -o OUTPUT.mcap|- [--compression none|zstd|lz4] [--memory-limit BYTES] [--force]
```

`pcx info`はPoint FrameをdecodeせずにMCAP Sourceをstreamingで調査します。human outputとversion付きJSONはstdoutへ出力され、成功時のstderrは空です。

`pcx topics`は各MCAP Channelについて、user-facingなTopic、Schema、encoding、message count、metadataに基づくROS 2 PointCloud2 candidate statusを表示します。candidate statusはmessage payloadのdecodeやvalidation成功を意味しません。

## 1 frameの抽出

```bash
pcx extract INPUT.mcap --topic TOPIC (--frame INDEX | --at DURATION) \
  -o OUTPUT.pcd|- [--encoding binary|ascii] [--memory-limit BYTES] [--force]
```

`--frame`は選択Topicに一致するmessage内の0-based indexです。`--at`はrecording開始から`83.2s`のようなduration以降で最初のframeを選びます。selectorの一方とfileまたはstdout sinkを明示します。binary PCDがdefaultです。Topic不在、範囲外、破損message、memory budgetを保証できない処理は、outputを確定する前に失敗します。

## encoded MCAP passthrough

`pcx passthrough`はPointCloud2やpoint fieldをdecodeせず、encoded messageを1件
選択します。Message payload、sequence、time、正確なChannelとoptional Schemaの
関係、recording-levelのattachment／metadata、application-private recordを保持
します。Container構造、statistics、CRCは再生成し、writer memoryをboundするため
attachment／metadata indexは省略します。意味が未定義のunknown future standard
recordは明示的に拒否します。compressionはsingle-threaded deterministic zstdが
defaultで、`none`とdeterministic LZ4も選択できます。

human-readableな診断はstderr、成功した`--json`のデータはstdoutに出力します。parse済みのJSON commandが失敗した場合、stdoutは空のまま、version付きJSON errorをstderrへ出力します。schemaとcompatibility policyは[`docs/json-schema`](https://github.com/takeshiD/pcx/tree/main/docs/json-schema)で公開します。human-readable outputとdiagnostic messageの文言はcompatibility contractではありません。既存ファイルは`--force`なしでは上書きせず、割り込み時は一時ファイルを除去して`130`を返します。
