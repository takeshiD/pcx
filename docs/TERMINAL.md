# Terminal capability policy

Status: implemented selection policy; rendering backends remain separate work.

`pcx` treats terminal detection as a read-only policy decision. Detection does
not render, does not write stdout or stderr, and accepts environment, TTY, and
capability-query state through injectable seams.

## Selection order

1. An explicit backend wins and skips detection. `plain` is always valid.
   `unicode` and `kitty` require TTY stdout; redirected control output is
   rejected instead of silently falling back.
2. Automatic selection uses `plain` immediately when stdout is not a TTY. It
   never queries a redirected stream.
3. A TTY with non-TTY stdin uses `unicode` without a query because a response
   cannot be read safely.
4. Missing, empty, or `dumb` `TERM` uses `plain` without a query.
5. SSH and tmux sessions use `unicode` without a query. This conservative path
   avoids passthrough differences, delayed responses, and remote hangs.
6. Other interactive sessions may run an injected capability query for at
   most 100 ms. A positive response selects `kitty`; unsupported, malformed,
   failed, disconnected, or timed-out queries fall back to `unicode`.

Environment values are only compared as opaque data and are never embedded in
control sequences. A query returns a typed result; its implementation must cap
response bytes and apply its own I/O deadline. The outer detector also applies
the fixed 100 ms deadline, so an uncooperative query cannot block selection.

The deterministic automatic fallback order is therefore:

```text
confirmed Kitty -> Unicode -> plain non-terminal output
```

This module does not change the existing command stream contract: data remains
on stdout, diagnostics remain on stderr, and binary PCD stdout is untouched.
