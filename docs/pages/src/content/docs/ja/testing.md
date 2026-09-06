---
title: テスト
description: パーサ、プランナ、CLI の信頼性設計。
---

テストは失敗コストに沿って層を分けます。

1. 単体テスト: checked arithmetic、スキーマ、選択、プランナ
2. fixture テスト: 最小 MCAP/CDR/PCD と破損入力
3. property test: レイアウト、予算、encode/decode 不変条件
4. CLI 結合テスト: 終了コード、stdout/stderr、アトミック出力
5. E2E: v0.1 実装後の 1 フレーム MCAP → PCD
6. 定期 fuzz: ネットワークサービスに依存しない不正入力検査
7. 定期 Criterion benchmark: native x86_64/aarch64 runner 上で probe、decode、operator、encode を個別に計測

固定の synthetic fixture に対する PCD 出力サイズと Planner 宣言の peak managed memory は、review 済み budget を決定論的な必須 gate として検証します。Hosted runner の wall-clock が 20% 以上変化した場合は調査対象ですが、architecture ごとの baseline が安定するまでは merge gate にしません。fixture、budget、比較規則の詳細は[テスト戦略](https://github.com/takeshiD/pcx/blob/main/docs/TESTING.md)を参照してください。

CI の最低条件は x86_64/aarch64 Linux のネイティブ runner 上での formatter、Clippy、typecheck、unit/integration test です。詳細は[テスト戦略](https://github.com/takeshiD/pcx/blob/main/docs/TESTING.md)を参照してください。
