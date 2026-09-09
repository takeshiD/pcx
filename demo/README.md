# CLI demo

[`quickstart.tape`](./quickstart.tape) is the source for the README demo. It
uses the real release-profile `pcx` binary and the committed PointCloud2 MCAP
fixture; it does not contact a hosted recording service.

From the repository root, regenerate and validate the demo with:

```bash
cargo build --release --locked
PATH="$PWD/target/release:$PATH" vhs validate 'demo/*.tape'
PATH="$PWD/target/release:$PATH" vhs demo/quickstart.tape
```

Review both the tape and generated GIF whenever commands or visible output
change. VHS 0.11 or newer is recommended.
