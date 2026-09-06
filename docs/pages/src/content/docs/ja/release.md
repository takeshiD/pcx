---
title: リリース
description: 厳密なタグ、検証ゲート、公開先。
---

自動リリースは `vX.Y.Z` または `vX.Y.Z-alpha.N` に厳密一致するタグだけを受け付けます。タグのバージョンは `Cargo.toml` と完全一致しなければなりません。

公開前に formatter、Clippy、typecheck、test、ドキュメント build、Nix check、`cargo package`、展開後 package の build、install smoke test を再実行します。

保護された GitHub `release` environment で承認後、既存の `pcx-cli` package を crates.io Trusted Publishing で公開し、x86_64/aarch64 Linux の closure を公開 Cachix `takeshid` に push し、tarball と SHA-256 を GitHub Release に追加します。各 tarball には実行ファイル、license、README、Bash/Zsh/Fish completion、manual page が含まれます。cache は `https://takeshid.cachix.org`、公開鍵は `takeshid.cachix.org-1:2GsGTUZ3djVzbGzXgeia+SRV1ZJYOXySHyNfBPsEjRA=` です。

対応する Linux tarball は引き続き GNU link 版です。static musl 候補が
これを置き換えるには、実際に公開する binary が native x86_64 と native
aarch64 の両方で全 feature の test、代表的な target system での smoke
test、明示的に受け入れられた size/runtime 比較を通過する必要があります。
cross-build や emulation の実行だけでは native 対応の根拠になりません。

environment secret は `CACHIX_AUTH_TOKEN` だけです。保護された publish job だけが GitHub OIDC を短時間有効な crates.io token と交換し、長期 registry token は保存しません。最初の crate version は同等の gate を通過後に maintainer が手動公開し、その後の自動 release 用に Trusted Publisher を設定します。
