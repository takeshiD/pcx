---
title: Install
description: 現在利用できるsource／Nix install方法。
---

## crates.ioから

```bash
cargo install pcx-cli
pcx --version
```

## Sourceから

```bash
git clone https://github.com/takeshiD/pcx.git
cd pcx
cargo install --path . --locked
pcx --version
```

## Nix

```bash
nix run github:takeshiD/pcx -- --version
nix develop github:takeshiD/pcx
```

NixはBash、Zsh、Fishのcompletionを各shellの標準share directoryにinstallし、
`pcx`、`pcx-info`、`pcx-topics`、`pcx-extract`、`pcx-passthrough`のmanual pageも
installします。release archiveには同じfileが次のpathで含まれます。

```text
share/bash-completion/completions/pcx
share/zsh/site-functions/_pcx
share/fish/vendor_completions.d/pcx.fish
share/man/man1/pcx*.1
```

system prefix以外へ展開する場合、Bash fileをsourceするか、対応するZsh/Fishの
directoryをshellのcompletion pathへ追加します。manual pageには`share/man`を
`MANPATH`へ追加します。

## Shell help assetの再生成

`generated/`以下のcommit済みfileは、実際のclap command grammarとpackage version
から生成されます。どちらかを変更した場合は次を実行します。

```bash
./scripts/generate-assets.sh
```

CIは`./scripts/generate-assets.sh --check`を実行し、commit済みcompletionまたは
manual pageが古い場合に失敗します。generated diffはgrammar changeと一緒にreviewし、
CIから自動更新しません。

## 対応target

| Target | Support |
| --- | --- |
| x86_64 Linux | native buildと全test |
| aarch64 Linux | native buildと全test |
| macOS | 非対応・将来未定 |
| Windows | 非対応・将来未定 |
