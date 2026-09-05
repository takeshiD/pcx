#!/usr/bin/env bash
set -euo pipefail

fixture_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
scratch="$(mktemp -d)"
trap 'rm -rf -- "$scratch"' EXIT

rustc --edition 2024 "$fixture_root/generate.rs" -o "$scratch/generate-fixtures"
"$scratch/generate-fixtures" "$scratch/corpus"

nix shell nixpkgs#mcap-cli --command mcap recover \
  "$scratch/corpus/raw-valid-pointcloud2.mcap" \
  --compression zstd \
  --output "$scratch/corpus/valid/pointcloud2.mcap"

install -Dm644 "$scratch/corpus/valid/pointcloud2-little-endian.cdr" \
  "$fixture_root/valid/pointcloud2-little-endian.cdr"
install -Dm644 "$scratch/corpus/valid/pointcloud2-big-endian.cdr" \
  "$fixture_root/valid/pointcloud2-big-endian.cdr"
install -Dm644 "$scratch/corpus/valid/pointcloud2-binary.pcd" \
  "$fixture_root/valid/pointcloud2-binary.pcd"
install -Dm644 "$scratch/corpus/valid/pointcloud2-ascii.pcd" \
  "$fixture_root/valid/pointcloud2-ascii.pcd"
install -Dm644 "$scratch/corpus/valid/pointcloud2.mcap" \
  "$fixture_root/valid/pointcloud2.mcap"
install -Dm644 \
  "$scratch/corpus/malformed/cdr-representation-identifier-must-be-cdr.cdr" \
  "$fixture_root/malformed/cdr-representation-identifier-must-be-cdr.cdr"
install -Dm644 \
  "$scratch/corpus/malformed/cdr-point-data-sequence-must-not-be-truncated.cdr" \
  "$fixture_root/malformed/cdr-point-data-sequence-must-not-be-truncated.cdr"
install -Dm644 \
  "$scratch/corpus/malformed/pointcloud2-field-must-fit-point-step.cdr" \
  "$fixture_root/malformed/pointcloud2-field-must-fit-point-step.cdr"
install -Dm644 \
  "$scratch/corpus/malformed/pcd-points-must-equal-width-times-height.pcd" \
  "$fixture_root/malformed/pcd-points-must-equal-width-times-height.pcd"

cp "$fixture_root/valid/pointcloud2.mcap" \
  "$fixture_root/malformed/mcap-leading-magic-must-match.mcap"
printf '\x00' | dd of="$fixture_root/malformed/mcap-leading-magic-must-match.mcap" \
  bs=1 seek=0 conv=notrunc status=none

LC_ALL=C find "$fixture_root/valid" "$fixture_root/malformed" -type f -print0 \
  | sort -z \
  | xargs -0 sha256sum \
  | sed "s#${fixture_root}/##" \
  > "$fixture_root/SHA256SUMS"
