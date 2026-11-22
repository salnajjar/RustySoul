# Remaining work

The first stable release includes the implemented functionality in [FEATURES.md](FEATURES.md). Continuing validation and future improvements:

- Broaden field validation of network/active-transfer cancellation and retry across more clients, routers and distributions.
- Complete an independent security review and evaluate hard storage quotas/reservations beyond the current free-space floor.
- Consider explicit per-command completion identifiers/acknowledgements. Multiple administrator roles remain optional future scope; current deployments have one administrator and scoped integration keys.

Fuzz targets and resource-boundary checks are included. Extended campaigns and soak testing are separate from routine CI and should retain their own dated evidence.

The tested slskd immediate-retry cleanup timing and Nicotine+ fragmented initial-handshake behavior are external-client findings, outside RustySoul's implementation scope. Harness accommodations are documented in [INTEROPERABILITY.md](INTEROPERABILITY.md) and [SOULFIND_SLSKD.md](SOULFIND_SLSKD.md). They do not establish behavior in newer upstream releases.
