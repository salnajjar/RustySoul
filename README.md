# RustySoul

RustySoul is a Rust based Soulseek daemon with a focus on speed, low resource use, security and full compatibility with other Soulseek clients.

It has a modern React browser interface, an authenticated API and SQLite persistence, and it runs with Docker, Podman or as a native Linux service.

**[Explore the project website →](https://salnajjar.github.io/RustySoul/)** — screenshots, features and installation options.

**Current beta: v0.1.0-beta.2.** Core workflows have automated coverage and independent-client interoperability tests. This is beta software; back up persistent state before upgrading. See [verification](docs/TESTING.md), [feature coverage](docs/FEATURES.md) and [remaining work](docs/TODO.md).

## Quick start with Docker

Published images currently support **Linux amd64**. Download this repository (or a release source archive), then:

```sh
cp .env.example .env
chmod 600 .env
docker compose pull
docker compose up -d --no-build
```

The public image is `ghcr.io/salnajjar/rustysoul:beta`. For a pinned version, set `RUSTYSOUL_IMAGE=ghcr.io/salnajjar/rustysoul:0.1.0-beta.2` in `.env`. No registry login is required for public pulls:

```sh
docker pull ghcr.io/salnajjar/rustysoul:0.1.0-beta.2
```

Open **http://127.0.0.1:8484**. Initial administrator credentials are **admin / change-me-now**, unless changed in `.env` before installation. On first sign-in you must replace **both** the username and password (minimum eight characters). Soulseek account credentials are separate: enter and verify them under **Settings -> Accounts**. Verified credentials can connect automatically on startup.

For another computer, use `ssh -L 8484:127.0.0.1:8484 your-server`, then open the local address. For remote browser access, configure an HTTPS reverse proxy. Preserve Host/Origin, disable buffering for `/api/v1/events` and allow idle connections longer than 15 seconds. [HTTP binding and connection settings](docs/CONNECTIONS.md) explain adapter binding and remote access.

### Storage and ports

Compose uses three named volumes by default: private application state at `/var/lib/rustysoul`, downloads at `/downloads`, and an initially empty shared library at `/library`. To use existing host folders, set `RUSTYSOUL_LIBRARY_SOURCE=/path/to/music` and `RUSTYSOUL_DOWNLOADS_SOURCE=/path/to/downloads` in `.env` before starting. Libraries are mounted read-only; downloads must be writable by UID/GID **10001**. Multiple libraries and subfolders can be configured in the UI once mounted. See [storage mapping](docs/STORAGE_MOUNTS.md).

The default Compose deployment uses Linux host networking for incoming transfers and UPnP discovery. HTTP binds to loopback and the transfer listener defaults to TCP **2234**. Change transfer ports, slots, per-user/total speed limits and UPnP under **Settings -> Network**. [Bridge networking](docs/CONTAINERS.md) is also available; its published transfer port must match the UI setting and requires container recreation to change. Back up application state and downloads before upgrades; `docker compose down -v` deletes named volumes.

## Podman and native installation

For rootless Podman:

```sh
podman pull ghcr.io/salnajjar/rustysoul:beta
mkdir -p ~/.config/containers/systemd ~/.config/rustysoul
install -m 600 .env ~/.config/rustysoul/daemon.env
install -m 644 deploy/container.toml ~/.config/rustysoul/rustysoul.toml
install -m 644 deploy/rustysoul.container ~/.config/containers/systemd/
systemctl --user daemon-reload
systemctl --user start rustysoul.service
```

The [container guide](docs/CONTAINERS.md) covers storage and networking. Native Linux release bundles include the executable, installer, systemd service and notices; see [native installation](docs/NATIVE.md). Node is only needed to build the frontend, not at runtime.

## Features

- Streaming search, format filters, wishlist, user/folder grouping, sorting and batch downloads.
- User profiles, pictures, presence, buddies, share trees and interest discovery.
- Persistent transfer queues, retries, priorities, manual ordering, bulk cancellation and audio playback.
- Multiple shared libraries, filesystem monitoring and incremental indexing with live progress.
- Multi-room chat, unread indicators, private messaging, room boards and access controls. [Saved chat and room history](docs/CHAT_HISTORY.md) survives closing conversations and daemon restarts, with explicit per-conversation deletion.
- Per-user/all-time traffic, compact long-term graphs, sharing protections and privileges status.
- Themes, API integration keys, operational logs, database maintenance and encrypted backup/restore.

See [feature details](docs/FEATURES.md) and the [API reference](docs/API.md). Credentials are encrypted at rest using a private local key; authentication uses hashed passwords and protected persistent sessions. See [security](docs/SECURITY.md) for boundaries and reporting.

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
