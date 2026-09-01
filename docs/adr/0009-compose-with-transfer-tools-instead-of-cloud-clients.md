# Compose with transfer tools instead of cloud clients

`pcx` will not implement AWS, S3, object-storage upload, multipart transfer, or cloud credential handling. It emits investigation artifacts to local files or stdout and relies on existing tools such as `ssh` and `scp` for transport, preserving a small single binary, a synchronous execution model, and a security boundary with no cloud credentials inside `pcx`; Cachix remains release infrastructure rather than a product sink.
