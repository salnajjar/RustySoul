# First public beta

RustySoul is a Linux Soulseek daemon with a responsive browser interface, authenticated API and SQLite persistence. This beta includes streaming search, share trees/profiles, persistent transfers, priorities/retries, per-user limits, multi-room chat, incremental library indexing, traffic graphs, scoped API keys, maintenance and encrypted backup/restore.

## Installation

Use the Compose files in the source archive or repository. Copy `.env.example` to `.env`, set your library/download mount paths, then run `docker compose pull` and `docker compose up -d --no-build`. See the [README](https://github.com/salnajjar/RustySoul#readme) for Podman, remote access and first sign-in.

Default administrator login: **admin / change-me-now**. Both must be replaced on first sign-in; Soulseek credentials are entered separately in Settings. HTTP defaults to loopback port **8484**; incoming transfers default to TCP **2234**.

The native Linux x86-64 bundle includes the executable, installer, systemd service and licence notices. It requires glibc 2.36 or newer; follow its README. The SHA-256 sidecar verifies the downloaded archive.

## Beta limits

Published images and native assets currently target **Linux amd64/x86-64**. Back up persistent state and media before upgrades. Broader real-world network/client testing and an independent security review remain useful; this release makes no production-readiness guarantee. Tested external-client quirks and test-harness accommodations are documented in the interoperability guides. Reference clients, personal installation files, credentials and generated test artifacts are excluded from the source distribution.
