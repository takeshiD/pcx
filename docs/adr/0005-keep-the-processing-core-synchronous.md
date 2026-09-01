# Keep execution synchronous

Planning, container IO, point decoding, operators, encoding, and local sinks will use a synchronous pull model. The product does not implement an object-storage or other network client, so no asynchronous runtime or network adapter belongs in the execution path; backpressure is expressed directly through bounded synchronous batches and byte writes.
