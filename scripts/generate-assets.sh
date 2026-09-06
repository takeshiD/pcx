#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
mode=${1:-generate}
if [[ "${mode}" != "generate" && "${mode}" != "--check" ]]; then
  echo "usage: scripts/generate-assets.sh [--check]" >&2
  exit 2
fi

temporary=$(mktemp -d)
trap 'rm -rf "${temporary}"' EXIT

cd "${repo_root}"
cargo run --quiet --locked --example generate-assets -- "${temporary}"

if [[ "${mode}" == "--check" ]]; then
  diff --recursive --unified generated "${temporary}"
  exit
fi

mkdir -p generated/completions generated/man
find generated/completions generated/man -type f -delete
cp -a "${temporary}/completions/." generated/completions/
cp -a "${temporary}/man/." generated/man/
