# Version JSON and define frame selection

Machine-readable command output carries a schema version and permits only additive changes within that version, while human-readable output is not a compatibility contract. A point frame is selected either by a zero-based index after Topic selection or as the first frame whose MCAP log time is at or after a duration relative to the recording start; the selectors are mutually exclusive and absence is reported explicitly.
