# RustySoul v0.1.0

The first stable release adds durable upload retries and installation-specific update notices to the functionality released in beta.5.

## Changes since beta.5

- Automatically re-offer new interrupted and timed-out uploads with increasing delays and a bounded retry budget. Retries wait for recipients to be online, respect sharing restrictions and queue limits, and require fresh transfer acceptance and the normal resume handshake. Cancelled transfers and permanent refusals are not retried automatically. Pending retries survive restarts and expire after seven days.
- Retry saved failed uploads for one recipient or everyone from the Uploads page. Historical failures are retried only when requested. Cancellation includes pending retry timers. Transfer retry settings control downloads and uploads separately; both default to enabled.
- Check for new stable releases anonymously at startup and every six hours. Show a dismissible notice with release notes and update instructions for Docker, Podman or native installations. Settings > Status & privacy includes a toggle and a manual check. No automatic installation or account/library telemetry is performed.
- Release disconnected peer workers promptly even when a replacement arrives before lifecycle notifications are processed, preventing capacity from remaining occupied by dead workers.
- Publish the stable container as `ghcr.io/salnajjar/rustysoul:0.1.0` and `:latest`, with a matching native Linux bundle. Update installation instructions and the project website for the stable channel.

## Upgrade

Back up persistent state, then pull the pinned `ghcr.io/salnajjar/rustysoul:0.1.0` image or the stable `:latest` channel. Recreate the container with its existing volumes, environment and service management. Beta users should explicitly switch image tags; the old `:beta` tag remains a beta release. Native installations can use the executable and installer in the release bundle. Keep the existing application data; no account reset or library rebuild is required.

SQLite schema 18 migrates supported earlier installations and validates schema-11 through schema-18 backups. Preserve the matching database and executable/image together for rollback. Existing accounts, sessions, encrypted credentials, history, settings, library and traffic statistics are retained. Pending retries concern originally requested files only; the receiver may decline a new offer.

The release gates include Rust/SDK and browser regressions, CodeQL review, native smoke checks and Docker/Podman installation checks. Public artifacts target Linux amd64/x86-64; the native bundle requires glibc 2.36 or newer. See [installation](https://github.com/salnajjar/RustySoul#readme) and [release verification](https://github.com/salnajjar/RustySoul/blob/main/docs/TESTING.md).

# RustySoul v0.1.0-beta.5

This beta improves search completeness, transfer reliability, upload reporting and everyday navigation. It includes the changes tested on the daemon since beta.4.

## Changes since beta.4

- Keep unlimited search tabs in creation order. New searches appear on the right; scroll back to reuse, edit or close older searches. History survives restarts. Older results stay in SQLite, with bounded live previews and paged access. Legacy search-count settings are accepted and ignored.
- Collect search replies for two minutes and handle crowded peer pools, fragmented replies and server backpressure without silently losing results or blocking user controls.
- Show an **In queue** summary between **Transferring** and **Current speed** on downloads and uploads. It counts the entire pending queue independently of filters and scrolling. Upload users start collapsed, and the maximum-downloads-per-user setting remains editable.
- Make upload lifecycle recording atomic and retryable, report recent payload speed, reconcile missing transfers, and fail queued/offered/active uploads when the server confirms that the recipient is offline. A closed control socket alone does not stop a healthy file stream.
- Correlate positive and negative upload replies across replacement control connections, release declined slots promptly, and reconnect queued recipients when an idle control connection disappears. Transfer-offer answer deadlines begin after delivery. Bound direct/reverse connection negotiation separately and prevent stale callbacks from reviving cancelled attempts.
- Allow 60 seconds without upload payload or Linux TCP send-queue progress. Successful completion waits for queued bytes to drain; failed and cancelled Unix transfers discard unsent payload promptly. Linux send-queue progress extends a blocked writer's deadline without allowing idle probes to keep it alive indefinitely.
- Estimate automatic profile upload speed from the fastest 10% of the latest 200 distinct recipients, using each recipient's lifetime payload/time average. Parallel files contribute one streaming-time interval per recipient. Manual reporting and privacy overrides remain available.
- Let completed files inherit destination-directory permissions and ACLs while protecting incomplete payloads. Clean up abandoned incomplete-file stubs without removing live or referenced partial downloads.
- Restore Online status on UI activity after automatic Away, and use the top-right status menu to choose Online/Away or disconnect. Confirm private-room creation and membership; public rooms retain the network's public-room ownership rules.
- Preserve accounts, sessions, encrypted credentials, library data, history and settings through the schema-17 upgrade. Expanded protocol, transfer, storage and desktop/mobile regression checks cover these changes.

## Upgrade

Back up persistent application state before upgrading. Pull `ghcr.io/salnajjar/rustysoul:0.1.0-beta.5` for the pinned image, or `ghcr.io/salnajjar/rustysoul:beta` for the current beta, and recreate the container with its existing mounts and environment/configuration. Do not delete its data volume. Docker, Podman and native Linux installations remain supported; no Compose or TOML file is required for environment-based deployment.

SQLite schema 17 validates schema-11 through schema-17 backups and migrates supported older backups on restore. Roll back using the matching earlier database and executable/image together. Historical global upload measurements cannot reconstruct per-recipient streaming time, so the new speed estimator learns from uploads recorded after this upgrade; its initial estimate is 1 MiB/s until measurements are available. Existing reporting overrides and preferences are preserved.

The release workflow gates publication on Rust/SDK tests, UI and desktop/mobile browser checks, CodeQL review, native smoke checks and Docker/Podman installation checks. Public container and native artifacts target Linux amd64/x86-64; the native bundle requires glibc 2.36 or newer. See [installation](https://github.com/salnajjar/RustySoul#readme) and [release verification](https://github.com/salnajjar/RustySoul/blob/main/docs/TESTING.md).

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
