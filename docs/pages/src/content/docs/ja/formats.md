---
title: フォーマットとプロトコル
description: 採用するフォーマット境界と忠実性の規則。
---

## v0.1 の範囲

| 境界 | 読み込み | 書き込み | 状態 |
| --- | --- | --- | --- |
| MCAP コンテナ | metadataとencoded record | 1 message container passthrough | 利用可能 |
| ROS 2 `sensor_msgs/msg/PointCloud2` | 厳密な CDR デコード | なし | 利用可能 |
| PCD | ASCII と little-endian binary | binary / ASCII | reader adapter は利用可能、input CLI command は未実装 |
| PLY 1.0 | ASCII と両方の binary byte order の scalar vertex | ASCII と両方の binary byte order | adapter は利用可能、CLI command は未実装 |
| LAS/LAZ | bounded synchronous batch | bounded synchronous batch | library adapterは利用可能、CLIは未公開 |

LAS/LAZとterminal renderingのCLI integrationは後続です。common CPU projection、
conservativeなterminal capability selection、Unicode、Kitty、Sixel backendは
内部adapterとして利用可能です。AWS/S3転送やcloud credentialは製品機能に含めません。

## Sixel terminal protocol

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

## 忠実性の契約

未対応のレイアウトや曖昧な変換は拒否します。フィールドの暗黙削除、数値型の変更、座標の意味変更、メタデータの破棄は行いません。意図的なスキーマ変更は引数で明示し、構造化出力にも記録します。

ROS 2 デコーダは CDR のアラインメント、エンディアン、次元、フィールドオフセット、ストライド、バッファ長を検証します。ROS のインストールは不要です。

バイナリは stdout に出力でき、診断は常に stderr に出します。ファイルは隣接する一時ファイルに書き、成功時だけアトミックに置き換えます。
