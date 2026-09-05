# pcx Domain Language

`pcx` is a shell-native tool for inspecting, reducing, and transferring point-cloud recordings where the sensor data lives.

## Data

**Source**:
A readable point-cloud file or sensor-log container supplied to `pcx`.
_Avoid_: Dataset, input

**Topic**:
The user-facing name of a message sequence within a temporal Source, such as `/lidar/points`.
_Avoid_: Channel, stream

**Channel**:
An MCAP record that associates a Topic with its message encoding and schema. Use this term only when discussing the MCAP representation.
_Avoid_: Topic

**Point Frame**:
A time-stamped point cloud that retains its frame boundary and coordinate-frame identity.
_Avoid_: Scan, message

**Static Cloud**:
A point cloud with no temporal frame sequence.
_Avoid_: Static frame

**Point Field**:
A named per-point attribute, such as `x`, `intensity`, or `ring`, together with its data representation and optional meaning.
_Avoid_: Property, dimension

**Coordinate Transform**:
The per-axis scale and offset that relate stored integer coordinates to spatial coordinates without changing their coordinate reference system.
_Avoid_: Quantization settings

**Coordinate Reference System (CRS)**:
The spatial reference that gives coordinates their geodetic or projected meaning.
_Avoid_: Frame ID, projection string

**Classification**:
The ASPRS class assigned to a point, distinct from its synthetic, key-point, withheld, and overlap flags.
_Avoid_: Label, class flags

**Extra Dimension**:
A vendor- or application-defined Point Field described by LAS Extra Bytes metadata.
_Avoid_: Padding, unknown bytes

## Processing

**Selection**:
A restriction that chooses existing channels, time ranges, frames, fields, or spatial regions without inventing new point values.
_Avoid_: Filter

**Semantic Reduction**:
A reduction in retained information through selection, cropping, voxel sampling, or frame sampling.
_Avoid_: Compression

**Representation Reduction**:
An explicit, potentially lossy change to how retained values are represented, such as coordinate quantization.
_Avoid_: Compression

**Codec Compression**:
A reversible encoding that reduces the byte representation without changing point-cloud meaning.
_Avoid_: Reduction

**Container Passthrough**:
Channel or time selection that copies encoded container messages without decoding point fields.
_Avoid_: Conversion

## Product Workflow

**Investigation Artifact**:
A deliberately reduced output created for diagnosis or transfer from an edge machine.
_Avoid_: Export, sample
