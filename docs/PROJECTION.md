# CPU Point Projection Contract

Status: implemented for terminal-neutral raster consumers.

This contract projects exactly one decoded Point Frame into a bounded CPU
raster. It deliberately defines no terminal escape protocol, capability
detection, Unicode mapping, Sixel encoding, or Kitty graphics encoding.

## Input and fidelity

Projection accepts either a validated `PointView` or an already materialized
`PointBatch`. X, Y, and Z must each be an unambiguous scalar `f32` or `f64`
Point Field identified by semantic meaning rather than field position. The
optional intensity policy requires one unambiguous scalar numeric Intensity
Point Field.

Projection is a side output: it borrows the Point Frame without changing it.
The raster retains the complete ordered Point Schema, all Point Frame metadata,
the original organized dimensions, and the source point index for every
occupied cell. Unknown Point Fields and their values remain in the input and
are neither rewritten nor silently discarded.

## View and fitted bounds

The camera is axis-aligned and orthographic. Callers explicitly choose three
distinct signed source axes:

- `right`: increasing camera-space values move toward larger raster columns;
- `up`: increasing values move toward smaller raster row numbers;
- `away`: increasing values move farther from the camera.

The built-in XY view uses +X right, +Y up, and +Z away. No trigonometry,
perspective division, hidden pose, or inferred sensor convention is used.

Finite camera-space right/up bounds are fitted into the requested non-zero
width and height. The fit preserves world-space aspect ratio and centers the
unused rows or columns as letterboxing. Raster storage is row-major from the
top-left. Continuous positions are rounded to the nearest pixel; an exact
half-pixel rounds toward the larger column or larger bottom-up row coordinate.

If one fitted axis has zero extent, all points are centered on that axis while
the other axis uses its full available extent. If both axes have zero extent,
all points address the deterministic center cell. For even dimensions the
larger bottom-up coordinate is selected, which means the upper of the two
middle raster rows. A frame with no finite coordinate triple produces an empty
raster with no fitted bounds.

## Invalid coordinates, collisions, and depth

NaN or infinity in any coordinate is handled by one explicit policy:

- `Drop` excludes that point from both fitted bounds and rasterization;
- `Reject` fails the Point Frame and reports the first invalid source index.

Every cell is a z-buffer entry. Smaller camera-space `away` depth is nearer and
wins occlusion. Depths within `16 * f64::EPSILON * max(abs(a), abs(b), 1)` are
a tie; the lower source point index wins because points are visited in input
order and ties never replace an occupied cell. This also makes exact collisions
and signed-zero depth deterministic.

## Color and intensity

`Uniform` assigns one explicit RGB8 value to every occupied cell. `Intensity`
uses an explicit finite, strictly increasing range, clamps finite values to
that range, maps it linearly to inclusive grayscale 0 through 255, and rounds
to the nearest integer. NaN and infinity receive the caller's explicit invalid
RGB8 color. There is no data-dependent automatic range, palette lookup,
dithering, gamma correction, or terminal color quantization in this layer.
Integer intensity values use Rust's defined cast to `f64` before mapping, so
integers above the exactly representable `f64` range receive the corresponding
rounded `f64` value consistently on both supported architectures.

## Numeric and cross-architecture policy

Both supported targets (`x86_64-linux` and `aarch64-linux`) use the same fixed
scalar algorithm and input order. `f32` coordinates are promoted exactly to
`f64`; `f64` values are used directly. The implementation uses no reduction in
hash iteration order, SIMD-width-dependent traversal, parallel traversal,
trigonometry, or fused multiply-add. Signed zero is canonicalized to positive
zero in raster depth. Range differences use a scaled fallback when direct
subtraction would overflow, and the depth tolerance above resolves values near
an occlusion boundary. These rules make raster dimensions, occupancy, source
indices, depth bits, and RGB bytes identical on the two supported native
architectures for identical input and options.

## Managed memory

Dimensions are checked before a plan can be created. The exact requested
row-major pixel allocation is `width * height * size_of::<Option<RasterPixel>>()`
on the target and is declared as bounded operator scratch. The shared operator
contract also declares X/Y/Z materialization and Intensity materialization when
selected. `ProjectionPlan::memory_requirements_for_view` combines retained
source bytes, those conservative materialization bounds, the raster allocation,
and caller-provided encoder/output/queue bounds through
`PipelineMemoryRequirements`. The existing `Planner` must accept that complete
peak under the requested memory limit before execution begins.
