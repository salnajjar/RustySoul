# Deploy with environment variables

Pull the image, mount persistent storage, set variables and start it. No repository checkout, Compose file or user-supplied `container.toml` is required. See the [complete Docker command](../README.md#quick-start-with-docker); replace `docker` with `podman` for Podman. The image includes its default configuration internally.

Container managers can set the same environment variables, volume mappings and published ports in their UI. Environment paths refer to **inside the container**. Mount host folders with the container manager's volume settings or `-v`; a daemon environment variable cannot create host mounts or publish container ports.

## Storage and first sign-in

Persist `/var/lib/rustysoul` for accounts, encrypted Soulseek credentials, settings and SQLite history. Mount downloads at `/downloads` and shared music at `/library` (read-only). More volumes may be mounted and added through Settings. See [storage permissions](STORAGE_MOUNTS.md), particularly for rootless Podman; do not recursively change ownership of an established music library just to start the container.

`RUSTYSOUL_DEFAULT_USERNAME` and `RUSTYSOUL_DEFAULT_PASSWORD` set only the **initial** administrator login, defaulting to `admin` / `change-me-now`. Both must change on first sign-in. Updating these environment variables never replaces an existing administrator account or password. Enter Soulseek credentials in the UI under Accounts.

## Variables

Environment values override the optional TOML configuration. HTTP binding, state location and file-picker roots apply each start. Library/download locations and network limits are initial defaults: existing saved UI settings win. `RUSTYSOUL_PUBLISHED_TRANSFER_PORT` always enforces the container's fixed published port. Change settings in the UI after installation rather than expecting bootstrap variables to overwrite saved preferences.

Booleans accept `true` or `false`. Folder lists accept one absolute path or a JSON array, for example `RUSTYSOUL_LIBRARY_DIRS='["/library","/second-library"]'`; `[]` means none. Paths with commas or spaces are retained exactly. Invalid values stop startup with a configuration error.

| Variable | Container default / purpose |
| --- | --- |
| `RUSTYSOUL_DEFAULT_USERNAME` | `admin`; first install only |
| `RUSTYSOUL_DEFAULT_PASSWORD` | `change-me-now`; first install only, at least 8 characters |
| `RUSTYSOUL_BIND` | `127.0.0.1:8484`; an IP and port, `all`, or `all:8484` |
| `RUSTYSOUL_ALLOW_REMOTE` | Built-in container configuration permits remote binding; explicitly set `true` with a non-loopback bind, especially for native startup |
| `RUSTYSOUL_DATA_DIR` | `/var/lib/rustysoul`; must be an absolute path |
| `RUSTYSOUL_DOWNLOAD_DIR` | `/downloads`; initial download location |
| `RUSTYSOUL_LIBRARY_DIRS` | `["/library"]`; initial shared libraries |
| `RUSTYSOUL_LOCATION_ROOTS` | `["/downloads","/library"]`; folders accessible to the UI folder picker |
| `RUSTYSOUL_TRANSFER_PORT` | `2234`; initial TCP transfer listener |
| `RUSTYSOUL_PUBLISHED_TRANSFER_PORT` | Unset; set to the container-side TCP port in bridge mode to keep the UI aligned with the publication |
| `RUSTYSOUL_SOULSEEK_SERVER` | `server.slsknet.org`; primarily for alternate/test servers |
| `RUSTYSOUL_SOULSEEK_PORT` | `2242` |
| `RUSTYSOUL_AUTO_CONNECT` | `true`; initial connect-on-startup preference |
| `RUSTYSOUL_MAX_PEERS` | `64`; 1–256 |
| `RUSTYSOUL_UPLOAD_SLOTS` | `2`; 1–16 |
| `RUSTYSOUL_DOWNLOAD_BYTES_PER_SECOND` | `0` (unlimited); initial total download speed limit |
| `RUSTYSOUL_MAX_SEARCH_RESULTS` | `100000`; 100–1000000 |
| `RUSTYSOUL_MAX_BROWSE_FILES` | `1000000`; 100–2000000 |
| `RUSTYSOUL_MAX_SEARCHES` | Legacy setting accepted and ignored; search history has no count limit. |
| `RUSTYSOUL_MAX_TRANSFERS` | `128`; 1–1024 |
| `RUST_LOG` | `rustysoul=info` when unset; diagnostic logging filter |
| `RUSTYSOUL_INSTALL_TYPE` | Auto-detected; optional `docker`, `podman`, `container` or `native` override for [release update instructions](UPDATES.md) |

The HTTP and published transfer ports must differ. Transfer ports must be 1024–65535. Published host and container transfer ports should match so peers receive the correct address.

## Host networking on Linux

Host networking supports UPnP discovery and UI-selected listener changes. Leave `RUSTYSOUL_PUBLISHED_TRANSFER_PORT` unset. This example keeps HTTP on host loopback for a local browser, SSH forwarding or an HTTPS reverse proxy:

```sh
docker run -d --name rustysoul --restart unless-stopped \
  --network host --read-only --cap-drop ALL \
  --security-opt no-new-privileges --pids-limit 256 \
  --tmpfs /tmp:rw,noexec,nosuid,size=16m \
  -e RUSTYSOUL_BIND=127.0.0.1:8484 \
  -e RUSTYSOUL_TRANSFER_PORT=2234 \
  -e RUSTYSOUL_DEFAULT_USERNAME=admin \
  -e RUSTYSOUL_DEFAULT_PASSWORD=change-me-now \
  -v rustysoul-data:/var/lib/rustysoul \
  -v rustysoul-downloads:/downloads \
  -v rustysoul-library:/library:ro \
  ghcr.io/salnajjar/rustysoul:latest
```

For another adapter set `RUSTYSOUL_BIND=192.0.2.10:8484` and `RUSTYSOUL_ALLOW_REMOTE=true`. `all` means `0.0.0.0:8484`. Use an HTTPS reverse proxy for remote browser access as described in [connections](CONNECTIONS.md).

## Native operation and compatibility

With no `--config`, the daemon reads `rustysoul.toml` if it exists in its working directory; otherwise it uses private native defaults plus the same environment overrides. For example:

```sh
RUSTYSOUL_DATA_DIR=/var/lib/rustysoul \
RUSTYSOUL_DOWNLOAD_DIR=/srv/downloads \
RUSTYSOUL_LIBRARY_DIRS=/srv/music \
RUSTYSOUL_LOCATION_ROOTS='["/srv/downloads","/srv/music"]' \
RUSTYSOUL_BIND=127.0.0.1:8484 rustysoul
```

Native defaults use `data` in the working directory, no shared folders, and loopback HTTP. Explicit `--config /path/file.toml` remains supported; a missing or invalid explicitly supplied file is an error, and is never silently ignored. Existing Compose, Quadlet and native services can keep using their current files. Preserve their storage and service management when upgrading.

The full override table is available in beta.4 and later. The basic run commands also work with beta.3, which already supports binding, transfer-port and bootstrap-account variables.
