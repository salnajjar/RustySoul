# RustySoul v0.1.0-beta.2

RustySoul is a Linux Soulseek daemon with a responsive browser interface, authenticated API and SQLite persistence. This beta includes streaming search, share trees/profiles, persistent transfers, priorities/retries, per-user limits, multi-room chat, incremental library indexing, traffic graphs, scoped API keys, maintenance and encrypted backup/restore.

## Changes since beta.1

- Private chats and room history are now retained in SQLite until explicitly deleted. Closing conversations, leaving rooms and restarting the daemon preserve history. Saved conversations are discoverable independently of the live preview, with older-message loading and confirmed per-conversation deletion.
- The share browser loads complete folder metadata without folder pagination. Expand and filter locally, resize the folder pane, and retain its width across peers and browser refreshes.
- Upload and download rows show their recorded start date/time and a live elapsed counter that freezes when the transfer finishes. Transfer timing is included in backups and survives restarts.
- Transfer-page speed cards show a five-second rolling average for a steadier indication of sustained bandwidth.
- A self-contained garden SVG supplies interface branding, favicons and installed-app icons. A web-app manifest and installation controls support installing RustySoul from the daemon's interface.
- Browser tests work with both local development and GitHub Actions, including concurrent fixture timing and isolated transfer destinations. Backup/restore and database maintenance support the updated history schema.

## Upgrade

Back up application state before upgrading. This release uses SQLite schema 14 and preserves existing accounts, sessions, credentials, settings and history. To roll back, restore the matching earlier database and image together. Previous releases retained only a 500-message live window; messages already purged by them are available only if present in an existing backup. Older transfer records have no invented start/end times.

Pull `ghcr.io/salnajjar/rustysoul:0.1.0-beta.2` for a pinned version, or `:beta` for the current beta. Recreate the container with its existing persistent volumes and configuration. Container updates do not automatically migrate existing native installations; use the native bundle's installer for those.

## Installation

Use the Compose files in the source archive or repository. Copy `.env.example` to `.env`, set your library/download mount paths, then run `docker compose pull` and `docker compose up -d --no-build`. See the [README](https://github.com/salnajjar/RustySoul#readme) for Podman, remote access and first sign-in.

Default administrator login: **admin / change-me-now**. Both must be replaced on first sign-in; Soulseek credentials are entered separately in Settings. HTTP defaults to loopback port **8484**; incoming transfers default to TCP **2234**.

The native Linux x86-64 bundle includes the executable, installer, systemd service and licence notices. It requires glibc 2.36 or newer; follow its README. The SHA-256 sidecar verifies the downloaded archive.

## Beta limits

Published images and native assets currently target **Linux amd64/x86-64**. Back up persistent state and media before upgrades. Broader real-world network/client testing and an independent security review remain useful; this release makes no production-readiness guarantee. Tested external-client quirks and test-harness accommodations are documented in the interoperability guides. Reference clients, personal installation files, credentials and generated test artifacts are excluded from the source distribution.
