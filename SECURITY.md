# Security policy

`pcx` processes untrusted binary recordings and may run on production edge systems. Parser vulnerabilities, unbounded allocations, path traversal, unintended file overwrite, credential exposure, and terminal escape injection are security-sensitive.

Please do not open a public issue for a suspected vulnerability. Report it privately through GitHub Security Advisories for `takeshiD/pcx`. Include the affected version or commit, a minimal reproducer when safe, and the expected impact.

The project does not implement AWS, S3, cloud credentials, a network listener, or a daemon.
