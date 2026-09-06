# Terminal Capability and Rendering Contract

Status: capability selection and the Unicode, Kitty, and Sixel encoders are
implemented. CLI integration remains separate work.

## Capability selection

`pcx` treats terminal detection as a read-only policy decision. Detection does
not render, does not write stdout or stderr, and accepts environment, TTY, and
capability-query state through injectable seams.

The selection order is:

1. An explicit backend wins and skips detection. `plain` is always valid.
   `unicode`, `kitty`, and `sixel` require TTY stdout; redirected control output
   is rejected instead of silently falling back.
2. Automatic selection uses `plain` immediately when stdout is not a TTY. It
   never queries a redirected stream.
3. A TTY with non-TTY stdin uses `unicode` without a query because a response
   cannot be read safely.
4. Missing, empty, or `dumb` `TERM` uses `plain` without a query.
5. SSH and tmux sessions use `unicode` without a query. This conservative path
   avoids passthrough differences, delayed responses, and remote hangs.
6. Other interactive sessions may run an injected capability query for at
   most 100 ms. A positive typed response selects `kitty` or `sixel`;
   unsupported, malformed, failed, disconnected, or timed-out queries fall
   back to `unicode`.

Environment values are only compared as opaque data and are never embedded in
control sequences. A query returns a typed result; its implementation must cap
response bytes and apply its own I/O deadline. The outer detector also applies
the fixed 100 ms deadline, so an uncooperative query cannot block selection.

The deterministic automatic fallback order is therefore:

```text
confirmed Kitty or Sixel -> Unicode -> plain non-terminal output
```

The Sixel encoder consumes the canonical `Selection` and refuses every
non-Sixel backend before DCS entry. The selected backend's own renderer owns
rendering and any portable fallback; Sixel encoding does not duplicate
capability policy or infer support from environment claims.

## Sixel encoding

The encoder consumes the immutable row-major [`Raster`](./PROJECTION.md)
produced by common CPU projection. It does not inspect Point Fields, alter
Point Frame metadata, choose a camera, or perform capability detection.

It uses transparent-background Sixel (`P2=1`), declares a 1:1 raster, and
preserves the raster's exact pixel width and height. Empty cells remain
transparent. Occupied RGB8 colors are assigned palette indexes in first
row-major occurrence order. RGB channels are mapped to Sixel's inclusive
0–100 range by deterministic nearest-integer rounding.

Rows are encoded in top-to-bottom six-row bands. Within each band, palette
planes follow palette order, trailing unset columns are omitted, and runs of
four or more identical sixel characters use repeat introducers. The emitted
bytes are therefore deterministic for an identical raster and limits. No
source names, metadata, environment strings, or caller text enters the DCS;
only fixed protocol bytes and bounded unsigned decimal integers are emitted.

## Bounds and planning

`SixelLimits` rejects zero dimension limits and palette limits outside 1–256.
Before output begins, `SixelPlan`:

1. checks raster width and height against their explicit limits;
2. collects no more than the configured palette limit in a fixed 256-entry
   table and refuses additional distinct colors without quantization;
3. runs the encoder through an overflow-checked counting sink; and
4. refuses an exact encoded payload larger than the configured byte limit.

The default limits are 4096×4096 pixels, 256 colors, and 64 MiB of emitted
bytes. Callers may choose tighter limits. The payload is streamed directly and
never retained as an encoder buffer. `SixelPlan::encoder_memory_bound` exposes
the fixed plan/palette state before rasterization for the common managed-memory
planner; output and queue bounds remain the caller's sink responsibility.

## Failure and interruption

Cancellation before DCS entry writes nothing. After DCS entry is attempted,
the encoder makes a best-effort write of the Sixel string terminator and flushes
it on success, cancellation, or body I/O failure. This cleanup bypasses the
encoder's cancellation check so an interrupt cannot suppress its restoration
attempt. If both the body and cleanup fail, the cleanup failure is reported
because terminal state is then uncertain. Process integration maps the typed
interruption category to exit status 130.

This module does not change the existing command stream contract: data remains
on stdout, diagnostics remain on stderr, and binary PCD stdout is untouched.

## Unicode rendering boundary

The Unicode backend consumes one terminal-neutral [`Raster`](../src/ops/projection.rs)
and writes one complete UTF-8 text frame synchronously. It does not inspect a
Point Frame, decode a format, read process environment variables, probe a
terminal, enter an alternate screen, move the cursor, or retain an encoded
frame. Callers provide an explicit `Tty` or `NonTty` output kind. The selection
policy above chooses a backend; the still-separate integration layer maps that
selection and the actual sink to an encoder output kind.

The backend has no dependency on Kitty, Sixel, iTerm, or any other image
protocol. Truecolor mode uses only Select Graphic Rendition (SGR) control
sequences. Monochrome mode uses no control sequences.

## Cell dimensions and aspect ratio

One terminal cell represents a vertical pair of raster pixels:

```text
raster (column c, row 2r)     -> upper half of terminal cell (c, r)
raster (column c, row 2r + 1) -> lower half of terminal cell (c, r)
```

A `width x height` raster therefore produces exactly `width` terminal columns
and `ceil(height / 2)` terminal rows. An odd final raster row occupies the
upper half of the final cell row; its missing lower pixel is empty. Every text
row, including the last, ends with LF. There is no carriage return, wrapping,
border, padding, cursor movement, or terminal-size lookup.

For a target viewport of `C x R` terminal cells, projection should request a
`C x (2R)` raster. The aspect model is explicit: a terminal cell is assumed to
have physical width:height `1:2`, so each half-block subpixel is approximately
square and the projection's world-space aspect ratio is preserved on that
font. Terminal software cannot reliably measure glyph geometry; a font with a
different cell ratio stretches the displayed result, and the encoder does not
apply a hidden correction.

The block elements `▀` (U+2580), `▄` (U+2584), and `█` (U+2588) are each
required to occupy one terminal cell. Unicode capability selection and a
possible ASCII fallback remain capability-layer policy rather than a property
guessed by this encoder.

## Occupancy and color policy

The mapping is fixed for each terminal cell:

| Upper raster pixel | Lower raster pixel | Monochrome glyph | Truecolor encoding |
| --- | --- | --- | --- |
| empty | empty | space | space |
| occupied | empty | `▀` | upper RGB as foreground on `▀` |
| empty | occupied | `▄` | lower RGB as foreground on `▄` |
| occupied | occupied, equal RGB | `█` | shared RGB as foreground on `█` |
| occupied | occupied, different RGB | `▀` | upper RGB as foreground and lower RGB as background |

`Monochrome` intentionally preserves occupancy, not luminance or RGB. It emits
only spaces, block glyphs, and LF. `TrueColor` emits each RGB8 component as an
unsigned decimal SGR parameter. It performs no palette lookup, quantization,
dithering, gamma conversion, alpha blending, background detection, or
data-dependent normalization. SGR is reset after every occupied cell, which
prevents style leakage to empty cells, later lines, and subsequent shell
output.

`NonTty` is an absolute normalization boundary: requested truecolor becomes
monochrome. Redirected output therefore contains no ESC, CSI, OSC, DCS, APC,
BEL, carriage return, or other C0 control byte except LF. This keeps logs,
snapshots, and pipes readable and prevents terminal behavior when redirected
bytes are later displayed. The selection policy rejects explicit Unicode on a
redirected sink; the encoder's non-TTY normalization is an additional safety
boundary for direct callers and future integration.

## Determinism and snapshots

For identical raster dimensions, occupancy, RGB bytes, requested color policy,
and output kind, the encoded bytes are identical on `x86_64-linux` and
`aarch64-linux`. Encoding is row-major, uses fixed UTF-8 glyphs, formats RGB8
values in canonical base-10 without padding, emits LF line endings, and reads
no locale, `TERM`, color environment variable, terminal response, clock, or
hash iteration order.

Reviewed truecolor snapshots use this pinned profile:

- explicit `Tty` output kind and `TrueColor` policy;
- fixed raster dimensions and RGB values;
- LF line endings;
- every ESC byte replaced by the four visible ASCII characters `\x1b` before
  comparison.

That normalization makes control sequences reviewable in a text diff without
changing their parameters or glyph bytes. CI never regenerates golden files.
Non-TTY snapshots compare the raw UTF-8 bytes because they contain no escape
sequences.

## Memory and output bounds

The encoder streams each cell directly to the caller's synchronous `Write`
sink and owns zero raster-sized scratch bytes. It does not allocate a `String`,
encoded frame, palette, row buffer, or output queue. Caller-owned writer
buffers remain part of the caller's managed-memory plan.

Before writing, the render plan checks a conservative complete-frame byte
bound. For `C` columns and `R` emitted rows the bounds are:

```text
Monochrome or NonTty: C * R * 3  + R bytes
TTY TrueColor:        C * R * 43 + R bytes
```

Three bytes cover the longest UTF-8 block glyph. Forty-three bytes cover a
cell with two `255;255;255` colors, the SGR introducer and terminator, the
glyph, and reset. The final `R` accounts for one LF per row. Checked arithmetic
rejects an unrepresentable bound before any output byte is written. Actual
output is no larger than the declared bound.

## Control-sequence safety

Raster and Point Frame data are untrusted. The encoder never writes schema
names, field names, frame IDs, timestamps, source indices, depths, or arbitrary
input bytes. Its output alphabet is limited to fixed spaces/block glyphs/LF,
plus fixed SGR syntax whose only variable pieces are decimal digits derived
from RGB8 integers. An input byte can therefore never become a control byte or
an SGR delimiter.

Tests project a Point Frame carrying OSC clipboard text, BEL, cursor-clearing
CSI, carriage return, and forged newline content in its frame ID, then prove
that none reaches either output mode. Property tests cover arbitrary RGB8
pairs and accept only CSI SGR sequences in TTY output; they reject OSC, DCS,
APC, PM, and any unexpected C0/DEL byte. Non-TTY tests additionally prove that
ESC is absent.

## Kitty encoding

The Kitty backend consumes the common CPU raster as streaming 32-bit RGBA.
Occupied pixels retain their projected RGB color with full opacity; empty cells
are transparent. Direct transmission uses RFC 4648 base64 payload chunks of at
most 4096 bytes. Non-final chunks are exactly 4096 bytes and therefore aligned
to four bytes as required by the protocol.

Dimensions default to a strict 4096 by 4096 ceiling and the complete base64
payload defaults to 64 MiB. Both limits are explicit inputs to encoder
preflight. The encoder uses a fixed 7298-byte workspace and exposes that bound
to the shared memory planner; it never materializes a second whole raster or
payload.

Kitty escape bytes are gated by the selection policy above. `auto` requires a
positive typed query, while explicit `kitty` still requires TTY stdout.
Selections of `sixel`, `unicode`, or `plain` return a portable-fallback outcome
and write no bytes. Environment claims such as `TERM=xterm-kitty` never
authorize Kitty output by themselves.

Each image has a caller-owned non-zero ID. Transmission uses `C=1`, so the
terminal cursor does not move. Cancellation before the first chunk writes
nothing; cancellation or failure after transmission begins attempts an
ID-scoped hard delete, which also aborts an incomplete chunked upload. A caller
can use the same bounded delete command during normal terminal teardown.
