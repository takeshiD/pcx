# Use view and columnar point representations

Decoded point data will support both a low-copy view over reference-counted source bytes and an owned, schema-driven columnar batch. Inspection and direct extraction can retain the view while point operators materialize columns on demand; semantic equivalence covers field names, types, counts, values, timestamps, and frame identity rather than padding or byte layout, and operations that change point count convert organized clouds into explicit unorganized clouds.
