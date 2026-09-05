---
status: accepted
---

# Keep GNU Linux release artifacts until musl is proven natively

`pcx` will continue to publish the existing GNU-linked `x86_64-linux` and
`aarch64-linux` archives. Fully static musl binaries improve portability, but
the audit did not prove a replacement on native hardware for both architectures
and found measurable size and runtime costs. Cross-build and emulated execution
are evidence of feasibility, not native support.

## Audit evidence

The 2026-09-05 audit used commit `de2552a`, the locked nixpkgs revision, Rust
1.97.1, `mcap` 0.25.0, and the production feature set with zstd and LZ4 enabled.
The ordinary musl cross package sets produced dynamically linked binaries with
Nix-store loader and `libgcc_s` references, so the portable candidates instead
used nixpkgs' fully static package sets. Both static targets compiled the
vendored `zstd-sys` and `lz4-sys` C dependencies successfully.

The x86_64 static target passed the full Rust test suite natively. The aarch64
static target cross-built successfully and, under QEMU user-mode emulation,
matched the GNU build's JSON and PCD output for both codecs, including selection
of message 65,535 from equivalent official-MCAP fixtures containing 65,536
messages and 14 MiB of decompressed chunk data. This was not a native aarch64
test.

Same-source, release-profile Nix builds after package fixup produced these size
results:

| Architecture | GNU binary | Static musl binary | Delta | GNU `.tar.xz` | Static musl `.tar.xz` | Delta |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| x86_64 | 1,965,512 B | 2,204,424 B | +12.2% | 611,376 B | 693,644 B | +13.5% |
| aarch64 | 1,963,304 B | 2,113,416 B | +7.6% | 557,132 B | 617,836 B | +10.9% |

On the native x86_64 audit host, Hyperfine measured `pcx --version` over 1,000
runs at 0.546 ms for GNU and 0.264 ms for static musl; this sub-millisecond
startup result is indicative rather than a release gate. Over 20 warm-cache
late-frame extractions, GNU versus static musl averaged 13.29 ms versus 16.66 ms
for zstd (+25.4%) and 12.87 ms versus 16.18 ms for LZ4 (+25.7%). Ten-run
aarch64 QEMU measurements also favored GNU by 6-7%, but emulated timings do not
predict native performance.

## Replacement gate

A future ADR may replace GNU archives only after the exact release candidates:

- have no ELF interpreter or required shared libraries and run outside Nix;
- build and pass the full production feature-set suite on native x86_64 and
  native aarch64 Linux, including zstd and LZ4 end-to-end fixtures;
- pass representative deployment smoke tests on supported target systems; and
- have their archive-size, startup, and representative runtime changes measured
  and explicitly accepted.

Side-by-side experimental musl archives must identify the libc target in their
names and cannot displace the GNU artifacts before that gate passes.

## Consequences

Release automation, archive names, and the GNU Nix packages remain unchanged.
The static candidates are not published or advertised as supported artifacts;
future toolchain or dependency changes can repeat the audit against the gate
above.
