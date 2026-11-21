# RustySoul

RustySoul is a Rust based Soulseek daemon with a focus on speed, low resource use, security and full compatibility with other Soulseek clients.

It has a modern React browser interface, an authenticated API and SQLite persistence, and it runs with Docker, Podman or as a native Linux service.

**[Explore the project website →](https://salnajjar.github.io/RustySoul/)** — screenshots, features and installation options.

**Current beta: v0.1.0-beta.5.** Core workflows have automated coverage and independent-client interoperability tests. This is beta software; back up persistent state before upgrading. See [verification](docs/TESTING.md), [feature coverage](docs/FEATURES.md) and [remaining work](docs/TODO.md).

## Quick start with Docker

Published images currently support **Linux amd64**. Pull the image and run it with environment variables and persistent volumes; no repository checkout, Compose file or user-supplied TOML file is needed.

```sh
docker pull ghcr.io/salnajjar/rustysoul:beta
docker run -d --name rustysoul --restart unless-stopped \
  --read-only --cap-drop ALL --security-opt no-new-privileges --pids-limit 256 \
  --tmpfs /tmp:rw,noexec,nosuid,size=16m \
  -p 127.0.0.1:8484:8484 -p 2234:2234 \
  -e RUSTYSOUL_BIND=all -e RUSTYSOUL_ALLOW_REMOTE=true \
  -e RUSTYSOUL_PUBLISHED_TRANSFER_PORT=2234 \
  -e RUSTYSOUL_DEFAULT_USERNAME=admin \
  -e RUSTYSOUL_DEFAULT_PASSWORD=change-me-now \
  -v rustysoul-data:/var/lib/rustysoul \
  -v rustysoul-downloads:/downloads \
  -v rustysoul-library:/library:ro \
  ghcr.io/salnajjar/rustysoul:beta
```

Open **http://127.0.0.1:8484**. For a server elsewhere, use `ssh -L 8484:127.0.0.1:8484 your-server` or an HTTPS reverse proxy. To publish HTTP on another host adapter, change the host address in `-p`. For a pinned image use `ghcr.io/salnajjar/rustysoul:0.1.0-beta.5`. Public pulls need no registry login.

Initial administrator credentials are **admin / change-me-now**, unless changed with the two deployment variables above. First sign-in requires replacing **both** the username and password (minimum eight characters). Existing accounts survive restarts and are never reset by these bootstrap variables. Soulseek credentials are entered and verified separately under **Settings → Accounts**.

### Storage and networking

Replace the named download/library volumes with `-v /your/downloads:/downloads` and `-v /your/music:/library:ro` to use existing host folders. The daemon runs as UID/GID **10001**; see [storage mapping](docs/STORAGE_MOUNTS.md) for permissions and rootless Podman. Keep the private state volume: it contains accounts, credentials, settings and history. Add more mounted libraries through Settings.

The bridge example publishes TCP **2234** and fixes the UI port to that publication. To change it, update the published port and `RUSTYSOUL_PUBLISHED_TRANSFER_PORT`, then recreate the container with the same volumes. Linux host networking supports UI port changes and UPnP without container port recreation; see the [environment deployment guide](docs/ENVIRONMENT.md) for a host-network example, all variables, GUI container managers and native operation. Slots, bandwidth limits, library locations and other preferences are editable in the UI.

Compose and Quadlet remain optional alternatives in [container deployment](docs/CONTAINERS.md). Existing configurations continue to work; environment variables override file values, and saved UI preferences retain precedence for their startup defaults.

## Podman and native installation

Use the same command with `podman` in place of `docker`. Named volumes also work with rootless Podman. For automatic startup through systemd, the optional `deploy/rustysoul.container` Quadlet example is available.

Native Linux bundles include the executable, installer, systemd service and notices; see [native installation](docs/NATIVE.md). Native startup can also use environment variables without a TOML file. Node is needed only to build the frontend.

## Features

- Streaming search, [file attributes and advanced quality filters](docs/SEARCH_AND_ATTRIBUTES.md), wishlist, user/folder grouping, sorting and batch downloads.
- [User profiles and presence](docs/PROFILE.md), pictures, measured/custom upload-speed reporting, buddies, share trees and interest discovery.
- Persistent transfer queues, retries, priorities, manual ordering, bulk cancellation and audio playback.
- Multiple shared libraries, filesystem monitoring and incremental indexing with live progress.
- Multi-room chat, unread indicators, private messaging, room boards and access controls. [Saved chat and room history](docs/CHAT_HISTORY.md) survives closing conversations and daemon restarts, with full-history text/sender search and all-history, age and date-range deletion.
- Per-user/all-time traffic, compact long-term graphs, sharing protections and privileges status.
- Themes, API integration keys, operational logs, database maintenance and encrypted backup/restore.

See [feature details](docs/FEATURES.md) and the [API reference](docs/API.md). Credentials are encrypted at rest using a private local key; authentication uses hashed passwords and protected persistent sessions. See [security](docs/SECURITY.md) for boundaries and reporting, and [security analysis](docs/SECURITY_ANALYSIS.md) for CI scanning, audited false-positive handling and release gates.

## Build and test

Use Linux, Rust **1.100**, Node **24**, Python 3.11+, a C/C++ toolchain, pkg-config and zlib development headers. Clang is used for release builds.

```sh
cd web
npm ci
npm run build
npm test
cd ..
cargo build --locked --release
./target/release/rustysoul --config rustysoul.toml
```

Build the container from source with `docker build -t rustysoul:local .`. [Testing instructions](docs/TESTING.md) cover Rust/UI/browser suites and optional independent-client matrices. External reference clients and local test output are not bundled. Releases are built and published by [GitHub Actions](.github/workflows/release.yml).

## Licence

RustySoul is distributed under [GPLv3](LICENSE). The vendored Soulseek SDK retains its MIT licence; dependency notices and licence texts are included in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Use Soulseek and shared content in accordance with applicable rights and service rules.
