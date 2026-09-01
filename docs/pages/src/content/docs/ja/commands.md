---
title: コマンド設計
description: 現在の機能と合意済み v0.1 インターフェース。
---

## 現在利用可能

```bash
pcx --help
pcx --version
```

現時点で実装済みなのはパッケージ基盤だけです。

## v0.1 で実装予定

```bash
pcx info INPUT.mcap
pcx topics INPUT.mcap [--json]
pcx extract INPUT.mcap --topic TOPIC --frame INDEX [-o OUTPUT.pcd]
```

`--frame` は選択トピックに一致するメッセージ内のゼロ始まりインデックスで、1フレームだけを選択します。トピック不在、範囲外、破損メッセージ、メモリ予算を保証できない処理は、出力を確定する前に失敗します。

診断は stderr、データと JSON は stdout に出力します。JSON は `schema_version` を持ちます。既存ファイルは `--force` なしでは上書きせず、割り込み時は一時ファイルを除去して `130` を返します。
