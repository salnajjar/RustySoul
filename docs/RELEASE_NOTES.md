# RustySoul v0.1.0-beta.4

RustySoul is a Linux Soulseek daemon with a responsive browser interface, authenticated API and SQLite persistence. This beta adds easier installation, richer discovery and search, clearer conversations, and stronger filesystem handling.

## Changes since beta.3

- Deploy Docker, Podman or the native daemon using environment variables. Compose, Quadlet and TOML configurations remain supported. Configure shared libraries and downloads through mounted folders and the documented environment settings.
- Ignore users in both rooms and private chats, manage share bans from user options, and receive persistent private-message unread notifications. Delete history selectively by age or date range.
- Click an entire private-conversation row to open it. Closing a conversation removes its sidebar row, selects the next unread conversation or the adjacent chat, and preserves its history. New messages or explicit reopening restore the conversation.
- Search all saved room/private history by message text, sender or both, with older-match paging and ignored-user filtering.
- Browse an interest in Discover and use its thumbs-up/down controls to add, switch or remove your own preference, including in shared-dislike views.
- Advertise supported file attributes for more common audio formats. Advanced search filters cover bitrate, duration, sample rate, bit depth, file size, CBR/VBR and available upload slots. Metadata appears in search and share views; sample-rate and bit-depth sorting are available. User/room searches correctly retain their selected scope.
- Expand audio metadata probing with bounded property-only reads and real synthetic format fixtures. Existing library metadata refreshes once per extractor version; later scans skip unchanged files. MP3 CBR reporting retains nominal bitrate for precise quality filters.
- Create a Soulseek account in Settings with password confirmation and verified login before saving credentials.
- Report your measured upload speed, choose a custom value or withhold it; own-profile queue statistics use the complete queue. Automatically use simultaneous downloads offered by a peer while respecting global slots.
- Harden credential-key and backup/status reads against symlinks and non-regular files, bound status-file reads, and retain the existing encrypted backup format. The stress tool generates its fixture-account password. Administrator passwords continue to use Argon2id; generated API keys retain their compatible fast digests.
- Add audited CodeQL review and a mandatory Critical gate to verification and publication. All security queries remain enabled, raw findings are retained, and intentional fixture/configuration flows have scoped, tested reasons. Public-username logging and the isolated mock-listener advisory remain visible. See [security analysis](https://github.com/salnajjar/RustySoul/blob/main/docs/SECURITY_ANALYSIS.md).

## Upgrade

Back up application state before upgrading. This release uses SQLite schema 16 and preserves existing accounts, sessions, encrypted credentials, settings, transfers, library data and history. Supported schema-11 through schema-15 backups are validated and migrated to schema 16 on restore. To roll back, restore the matching earlier database and image together. Messages purged by old releases are recoverable only from an existing backup.

Library metadata is refreshed automatically during the next scan. This reads file properties without changing media; large libraries may take longer on the first upgraded scan. Startup-index and scheduled-banner preferences are preserved. Subsequent unchanged scans retain the incremental cache.

Pull `ghcr.io/salnajjar/rustysoul:0.1.0-beta.4` for a pinned image, or `:beta` for the current beta. Recreate the container with its existing data, library and download volumes and its existing environment/configuration. Do not delete the persistent data volume when upgrading.

## Installation

Pull the image, mount persistent data plus your library/download folders, and set the deployment environment variables. No Compose or TOML file is required. The [README](https://github.com/salnajjar/RustySoul#readme) includes Docker and Podman run commands; the [environment reference](https://github.com/salnajjar/RustySoul/blob/main/docs/ENVIRONMENT.md) lists overrides and native examples.

Default administrator login: **admin / change-me-now**, overridable at deployment. Both fields must be replaced on first sign-in. Enter or create the separate Soulseek network account in Settings. Container HTTP listens on port **8484**; the documented host publication is loopback by default. Incoming file transfers default to TCP **2234**.

The native Linux x86-64 bundle includes the executable, installer, systemd service and documentation/licence notices. It requires glibc 2.36 or newer. Follow its README and verify the archive using its SHA-256 sidecar.

## Verification and beta scope

This release is gated by Rust application/integration tests, SDK tests, UI tests, desktop/mobile browser tests, CodeQL review, native smoke checks and Docker/Podman installation checks. Synthetic network fixtures are distinguished from external-client interoperability evidence in the testing guides. Published images and native assets target **Linux amd64/x86-64**.

Reference clients, credentials, installation-specific documents and generated test/build artifacts are excluded from the repository and release source. Independent review and broader real-world feedback remain useful for beta software.
