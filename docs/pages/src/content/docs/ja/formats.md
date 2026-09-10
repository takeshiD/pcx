---
title: フォーマットとプロトコル
description: 採用するフォーマット境界と忠実性の規則。
---

## v0.1 の範囲

| 境界 | 読み込み | 書き込み | 状態 |
| --- | --- | --- | --- |
| MCAP コンテナ | metadataとencoded record | 1 message container passthrough | 利用可能 |
| ROS 2 `sensor_msgs/msg/PointCloud2` | 厳密な CDR デコード | なし | 利用可能 |
| PCD | ASCII と little-endian binary | binary / ASCII | Static Cloudとして`pcx render`から読取可能 |
| PLY 1.0 | ASCII と両方の binary byte order の scalar vertex | ASCII と両方の binary byte order | adapter は利用可能、CLI command は未実装 |
| LAS/LAZ | bounded synchronous batch | bounded synchronous batch | Static Cloudとして`pcx render`から読取可能 |
| Terminal raster | 選択した1 MCAP Point FrameまたはPCD／LAS／LAZ Static Cloud | Unicode／ANSI、Kitty、Sixel | `pcx render`で利用可能 |
| PNG | なし | projection済みRGBA8可視化 | `pcx snapshot`で利用可能 |

LAS/LAZのconversion commandは後続です。common CPU projection、conservativeなterminal
capability selection、Unicode、Kitty、Sixel backendは`pcx render`から利用できます。
AWS/S3転送やcloud credentialは製品機能に含めません。

## projection済みPNG snapshot

`pcx snapshot`はcommon XY orthographic rasterをnon-interlaced RGBA8 PNGとして
streamingします。occupied cellはopaque white、empty cellはtransparentです。
encoderは固定8 KiB scratchを使いmanaged-memory preflightへ参加し、file outputは
atomicにcommitします。PNGは可視化用side outputだけであり、depth map、LiDAR range
image、点群の復元、PNG inputには対応しません。

## Terminal rendering

`pcx render`はMCAPからROS 2 `PointCloud2` Point Frameを1件選ぶか、PCD／LAS／LAZ Static
Cloudを1件読み取り、boundedかつterminal-neutralなrasterへprojectionします。
MCAPではTopic／Point Frame selectorが必須で、static Sourceでは拒否します。projectionは
synchronous、orthographic、
axis-alignedで、要求したrasterへfitするframe-localな処理です。Source Point Frameを
変更したり置き換えたりしません。

Unicode renderingは縦2 raster pixelを1 terminal cellへまとめ、`▀`、`▄`、`█`を
使います。interactive colorはANSI SGR truecolorです。`NO_COLOR`とすべてのnon-TTY
出力ではmonochrome occupancyを使います。non-TTYのbyte列はUTF-8 block glyph、space、
LFだけで、ANSI／graphics protocol escapeを含みません。

Kittyはtransparent RGBAをbounded base64 chunkでstreamingします。Sixelはtransparent
backgroundのdeterministicなpalette imageをstreamingし、colorをquantizeせず、設定した
palette limitを超える場合は拒否します。どちらのgraphics backendもTTY stdoutが必要です。
automatic selectionは`TERM`だけをgraphics出力の根拠にせず、時間制限付きcapability
queryでKitty／Sixelが確認された場合だけ選択します。それ以外はUnicodeまたはsafeな
plain textへfallbackします。正確なbound、detection順、出力byte、interrupt時cleanupは
[terminal contract](https://github.com/takeshiD/pcx/blob/main/docs/TERMINAL.md)を参照してください。
現在のCLI process queryはunsupportedを返すため、`auto`はまだKitty／Sixelを許可しません。
TTY stdoutではどちらも明示指定できます。

### Sixel terminal protocol

Sixel adapterはcommon rasterからtransparent backgroundのdeterministicなimageを
streamingします。呼び出し側が設定したdimension、distinct color数、正確な
encoded payloadのboundを出力開始前に検証し、超過する場合は拒否します。Sixel
escapeを出力できるのは、shared capability policyがSixelを選択した場合だけです。
その他のbackendはDCS entry前に拒否し、各rendererがfallbackを担当します。encoder自身は
capability probingを行いません。

## 忠実な PLY subset

PLY の入出力は、順序付き scalar property を持つ単一の `vertex`
element のみを扱います。PLY 1.0 の `ascii`、`binary_little_endian`、
`binary_big_endian` を受け付け、binary payload は宣言された byte order
で decode／encode します。

| PLY scalar | 共通 Point Field |
| --- | --- |
| `char` / `int8` | signed 8-bit integer |
| `uchar` / `uint8` | unsigned 8-bit integer |
| `short` / `int16` | signed 16-bit integer |
| `ushort` / `uint16` | unsigned 16-bit integer |
| `int` / `int32` | signed 32-bit integer |
| `uint` / `uint32` | unsigned 32-bit integer |
| `float` / `float32` | IEEE-754 binary32 |
| `double` / `float64` | IEEE-754 binary64 |

未知の scalar property 名と順序は保持します。list property、face などの
非 vertex element、64-bit integer、`count > 1` の Point Field、organized
cloud、property 名から復元できない semantic は、未対応または lossy として
拒否します。binary float は bit pattern を保持します。PLY 1.0 には NaN と
infinity の portable な表記がないため ASCII 入出力では拒否し、negative zero
を含む finite value は round-trip します。

PLY は Point Frame metadata や organized shape を持ちません。読み込み時は
static cloud の既定値として timestamp zero、空の frame id、
`is_dense = false`、vertex count と同じ width、height one を設定します。
comment と `obj_info` は非 semantic な header annotation として受理しますが、
共通 point schema には含めません。書き込みでも同じ static cloud metadata
既定値を要求し、timestamp、frame identity、density、container time、organized
shape を暗黙に破棄せず拒否します。

reader は最大 64 KiB の header のみを parse し、point column の正確な allocation
量を提示します。十分な materialization budget が渡されるまで column を確保しません。
payload I/O は synchronous かつ固定 buffer で、encoded file 全体を読み込みません。

MCAP passthroughは選択したencoded Messageと正確なChannel／Schema関係に加え、
recording-levelのattachment、metadata、private recordを保持します。派生container
構造は固定されたbounded-memory policyで再構築します。

## LAS／LAZ mapping

LAS/LAZの座標はsemanticな`f64` X/Y/Z Point Fieldへmappingします。元のaxisごとの
scale／offset、CRS VLR／EVLR、完全なLAS headerは保持します。Classificationと
synthetic／key-point／withheld／overlap flagは分離し、Extra Dimensionはdescriptorと
ordered raw byteを保持します。

通常のreadはcaller指定のpoint数でbatchをboundします。official parserがheader
recordをallocateする前に、fixed-buffer probeが宣言された全VLR／EVLR headerを走査し、
payload length、padding、structure overheadをchecked arithmeticでadmitします。
terminal renderingでは
headerの宣言点数をwhole Static Cloudのbatch boundとし、point decode前にprojection
raster／encoderと合わせてplanningします。これによりglobalな範囲へ一度だけfitし、
`--memory-limit`に収まらないcloudはpartial batchごとのfitを行わず拒否します。
decode後もStatic Cloudはcommon-schema batchと完全なLAS headerを同時に所有し、
projection完了までSpatialMetadataを保持します。

## 忠実性の契約

未対応のレイアウトや曖昧な変換は拒否します。フィールドの暗黙削除、数値型の変更、座標の意味変更、メタデータの破棄は行いません。意図的なスキーマ変更は引数で明示し、構造化出力にも記録します。

ROS 2 デコーダは CDR のアラインメント、エンディアン、次元、フィールドオフセット、ストライド、バッファ長を検証します。ROS のインストールは不要です。

バイナリは stdout に出力でき、診断は常に stderr に出します。ファイルは隣接する一時ファイルに書き、成功時だけアトミックに置き換えます。
