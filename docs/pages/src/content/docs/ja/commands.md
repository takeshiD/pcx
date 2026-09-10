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
pcx render INPUT.mcap --topic TOPIC (--frame INDEX | --at DURATION) \
  [--backend auto|unicode|kitty|sixel] [--width PIXELS] [--height PIXELS] \
  [--palette-limit COLORS] [--payload-limit BYTES] [--memory-limit BYTES]
pcx render INPUT.pcd \
  [--backend auto|unicode|kitty|sixel] [--width PIXELS] [--height PIXELS] \
  [--palette-limit COLORS] [--payload-limit BYTES] [--memory-limit BYTES]
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

## 1 Point Frameをrender

```bash
pcx render INPUT.mcap --topic TOPIC (--frame INDEX | --at DURATION) \
  [--backend auto|unicode|kitty|sixel] [--width PIXELS] [--height PIXELS] \
  [--palette-limit COLORS] [--payload-limit BYTES] [--memory-limit BYTES]
pcx render INPUT.pcd \
  [--backend auto|unicode|kitty|sixel] [--width PIXELS] [--height PIXELS] \
  [--palette-limit COLORS] [--payload-limit BYTES] [--memory-limit BYTES]
```

MCAPでは`pcx render`は`extract`と同じTopic／Point Frame selectorを使います。
PCDでは1件のStatic Cloudを読み取り、`--topic`、`--frame`、`--at`を拒否します。
選択したSourceをstrictにdecodeし、bounded rasterへdeterministicなCPU projectionを
行い、1枚のinline renderingをstdoutへstreamingします。
full-screen viewerやevent loopではないone-shot commandです。

default backendは`auto`です。redirected stdoutにはqueryを行わず、terminal control
sequenceを含まないdeterministicなUnicode occupancy textを出力します。TTYでは、
`TERM`が未設定、空、または`dumb`ならplain outputを使います。stdinがnon-interactive、
SSH、tmuxの場合はprobeせずUnicodeを使います。それ以外のinteractive sessionでは
時間制限付きcapability queryを実行でき、Kitty／Sixelが確認された場合だけその
protocolを選びます。未対応、malformed、失敗、timeout時はUnicodeへfallbackします。
現在のprocess queryはunsupportedを返すため、shipped `auto` pathがgraphics protocolを
automaticに選ぶことはありません。

`unicode`、`kitty`、`sixel`の明示指定はdetectionをskipし、TTY stdoutを要求します。
redirected stdoutとの不整合はANSI／Kitty／Sixel escapeを1 byteも書く前に失敗します。
`NO_COLOR`はUnicode truecolorをmonochrome block textにしますが、graphics protocolの
選択や許可には使いません。raster dimension、graphics payload limit、Sixel palette
size、encoder state、projection storageはbackendに応じて出力前に検証します。
rasterのdefaultは80×48 pixelで、Unicodeでは80 column×24 rowです。Sixel用の
`--palette-limit`はdefault 256 colors、Kitty／Sixel用の`--payload-limit`はdefault
64 MiBです。両graphics backendには固定の4096×4096 ceilingがあります。
`--memory-limit`はdefault 512 MiBで、managed Source decode、projection、raster、
encoder memoryをboundします。PCDはcase-insensitiveな`.pcd` filename extensionで
判定し、その他のpathは従来どおりMCAPとして扱います。

human-readableな診断はstderr、成功した`--json`のデータと`pcx render`の1 frameは
stdoutに出力します。`render`のautomaticなredirected outputにはterminal control
sequenceが含まれません。parse済みのJSON commandが失敗した場合、stdoutは空のまま、
version付きJSON errorをstderrへ出力します。schemaとcompatibility policyは
[`docs/json-schema`](https://github.com/takeshiD/pcx/tree/main/docs/json-schema)で
公開します。human-readable outputとdiagnostic messageの文言はcompatibility
contractではありません。既存ファイルは`--force`なしでは上書きせず、割り込み時は
一時ファイルを除去して`130`を返します。
