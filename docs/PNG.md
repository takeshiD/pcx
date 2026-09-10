# Projected PNG Snapshot Contract

Status: implemented by the one-Point-Frame `pcx snapshot` command.

`pcx snapshot INPUT.mcap --topic TOPIC (--frame N | --at T) -o PATH|-`
strictly decodes one ROS 2 `sensor_msgs/msg/PointCloud2`, applies the common
frame-local CPU projection, and streams one deterministic PNG investigation
artifact. It does not read PNG and does not encode point-cloud, depth-map, or
LiDAR range-image data.

## Pixel mapping

The command uses the built-in XY orthographic view, nearest-depth collision
policy, non-finite-coordinate drop policy, and uniform white occupied color.
The requested raster defaults to 80×48 pixels. Occupied cells become RGBA8
`(255,255,255,255)` and empty cells become transparent `(0,0,0,0)`. PNG rows
follow the common raster's top-to-bottom row order. The output is non-interlaced
RGBA with 8 bits per channel and filter type None.

This is a side output: the selected Point Frame, complete Point Schema,
metadata, and source values remain unchanged. The PNG is only a projection and
cannot reconstruct them.

## Determinism and bounds

The encoder emits the PNG signature, one IHDR, consecutive IDAT chunks carrying
a zlib stream of stored DEFLATE blocks, and one IEND. Chunk CRC-32 and zlib
Adler-32 values are deterministic. Encoding uses a fixed 8 KiB scratch buffer
and never retains a second frame-sized pixel buffer. Dimensions, raster storage,
encoder scratch, retained input, and fixed planner overhead are checked against
`--memory-limit` before the output transaction starts.

An explicit output path or `-` is required. `-` is binary-safe stdout and cannot
be combined with `--force`. File output uses a sibling temporary file and an
atomic rename after successful flush and sync; existing output requires
`--force`. Cancellation removes temporary output and exits with status 130.
