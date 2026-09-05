#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C

if [[ $# -ne 3 ]]; then
  echo "usage: scripts/package-release-archive.sh PCX_BINARY VERSION ARCH" >&2
  exit 2
fi

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
binary=$1
version=$2
arch=$3
archive="pcx-v${version}-${arch}.tar.xz"
temporary=$(mktemp -d)
trap 'rm -rf "${temporary}"' EXIT

actual_version=$("${binary}" --version)
if [[ "${actual_version}" != "pcx ${version}" ]]; then
  echo "binary version '${actual_version}' does not match archive version '${version}'" >&2
  exit 1
fi

install -Dm755 "${binary}" "${temporary}/pcx"
install -Dm644 "${repo_root}/LICENSE" "${temporary}/LICENSE"
install -Dm644 "${repo_root}/.github/release/README.md" "${temporary}/README.md"
install -Dm644 "${repo_root}/generated/completions/pcx.bash" \
  "${temporary}/share/bash-completion/completions/pcx"
install -Dm644 "${repo_root}/generated/completions/_pcx" \
  "${temporary}/share/zsh/site-functions/_pcx"
install -Dm644 "${repo_root}/generated/completions/pcx.fish" \
  "${temporary}/share/fish/vendor_completions.d/pcx.fish"
install -Dm644 "${repo_root}"/generated/man/*.1 \
  -t "${temporary}/share/man/man1"

files=(
  LICENSE
  README.md
  pcx
  share/bash-completion/completions/pcx
  share/fish/vendor_completions.d/pcx.fish
)
for manual in "${temporary}"/share/man/man1/*.1; do
  files+=("share/man/man1/${manual##*/}")
done
files+=(share/zsh/site-functions/_pcx)

tar --sort=name \
  --mtime="@${SOURCE_DATE_EPOCH:-0}" \
  --owner=0 --group=0 --numeric-owner \
  -C "${temporary}" -cJf "${archive}" "${files[@]}"
echo "${archive}"
