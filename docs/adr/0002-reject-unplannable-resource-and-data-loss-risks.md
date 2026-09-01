# Reject unplannable resource and data-loss risks

`pcx` will not silently discard fields or temporal and spatial metadata during conversion, and it will treat a requested memory limit as a hard execution contract. A job that cannot plan a compatible loss policy or guarantee the limit before execution is rejected; lossy conversion and temporary spooling require explicit user authorization.
