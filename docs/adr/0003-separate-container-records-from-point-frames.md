# Separate container records from point frames

The processing model will keep encoded container records separate from decoded point frames. Container-level channel and time selection can therefore preserve and copy MCAP records without point decoding, while point operations enter a semantic pipeline only after `PointCloud2` decoding; temporal point operations act on each point frame by default so timestamps and frame boundaries remain intact and memory remains bounded.
