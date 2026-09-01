---
title: セキュリティ
description: 脅威境界と脆弱性報告。
---

MCAP、CDR、点群ファイルは信頼できない入力として扱います。パーサは checked arithmetic を使い、オフセットと長さを検証し、実行前に割り当て上限を保証し、未対応レイアウトを拒否します。`pcx` はクラウド認証情報を扱わず、ネットワーククライアントを含みません。

脆弱性の疑いを公開 Issue に記載せず、リポジトリの[セキュリティポリシー](https://github.com/takeshiD/pcx/blob/main/SECURITY.md)に従って非公開で報告してください。
