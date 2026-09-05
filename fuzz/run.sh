#!/usr/bin/env bash
set -euo pipefail

seconds="${1:-300}"
common_args=(-max_total_time="${seconds}" -timeout=10 -max_len=1048576)
export CARGO_NET_OFFLINE=true

corpus_root="$(mktemp -d -p /tmp pcx-fuzz-corpus.XXXXXX)"
cleanup() {
  if [[ -d "${corpus_root}" && "${corpus_root}" == /tmp/pcx-fuzz-corpus.* ]]; then
    rm -rf -- "${corpus_root}"
  fi
}
trap cleanup EXIT

mkdir -p \
  "${corpus_root}/mcap_probe" \
  "${corpus_root}/cdr" \
  "${corpus_root}/pointcloud2_layout" \
  "${corpus_root}/pcd_headers"

cp tests/fixtures/valid/pointcloud2.mcap "${corpus_root}/mcap_probe/"
cp tests/fixtures/malformed/mcap-leading-magic-must-match.mcap "${corpus_root}/mcap_probe/"

cp tests/fixtures/valid/pointcloud2-little-endian.cdr "${corpus_root}/cdr/"
cp tests/fixtures/valid/pointcloud2-big-endian.cdr "${corpus_root}/cdr/"
cp tests/fixtures/malformed/cdr-representation-identifier-must-be-cdr.cdr "${corpus_root}/cdr/"
cp tests/fixtures/malformed/cdr-point-data-sequence-must-not-be-truncated.cdr "${corpus_root}/cdr/"

cp tests/fixtures/valid/*.cdr "${corpus_root}/pointcloud2_layout/"
cp tests/fixtures/malformed/*.cdr "${corpus_root}/pointcloud2_layout/"

cp tests/fixtures/valid/*.pcd "${corpus_root}/pcd_headers/"
cp tests/fixtures/malformed/*.pcd "${corpus_root}/pcd_headers/"

for target in mcap_probe cdr pointcloud2_layout pcd_headers; do
  cargo fuzz run --fuzz-dir fuzz "${target}" "${corpus_root}/${target}" -- "${common_args[@]}"
done
