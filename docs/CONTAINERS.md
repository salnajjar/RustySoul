# Container deployment

Public beta image: `ghcr.io/salnajjar/rustysoul:beta`; pin a version such as `:0.1.0-beta.5` for reproducible upgrades. Published binaries currently target Linux amd64. Docker Engine and rootless/rootful Podman are supported, with environment-based `run` commands as the default setup. See the repository README for first sign-in and account setup.

Registry images use Docker media types so Podman retains the embedded health check. The release workflow tests the published digest with Podman before creating the GitHub release, in addition to the Docker installation matrix.

For the simplest installation, use the [Docker/Podman run command](../README.md#quick-start-with-docker) with environment variables and volumes. No downloaded deployment files are required. The [environment guide](ENVIRONMENT.md) lists every override, host networking, first-install defaults and native startup. Compose and Quadlet below are optional alternatives for users who prefer them.

`compose.yaml` uses host networking. `compose.bridge.yaml` publishes loopback HTTP and the selected TCP transfer port. For bridge mode:

```sh
docker compose -f compose.bridge.yaml pull
docker compose -f compose.bridge.yaml up -d --no-build
```

Set `RUSTYSOUL_TRANSFER_PORT` in `.env` before creating bridge containers. Docker/Podman cannot dynamically add published ports to an existing container: update the publication and recreate it if the UI port changes. Host networking permits listener changes without recreating the container and supports UPnP multicast discovery more directly. Router forwarding is separate from container publication; diagnostics describe both.

Three separate volumes hold private state, downloads and shared files. For host folders and multiple libraries, see [storage mapping](STORAGE_MOUNTS.md). The runtime runs as UID/GID 10001 with a read-only root filesystem, dropped capabilities and no new privileges in the supplied manifests. Existing bind folders need appropriate permissions. Rootless Podman uses mapped IDs; named volumes simplify initial setup.

`deploy/rustysoul.container` is a rootless Podman Quadlet example. For this optional example, copy `deploy/container.toml` to `~/.config/rustysoul/rustysoul.toml` and create `~/.config/rustysoul/daemon.env` with mode 600 using `.env.example` and the [environment guide](ENVIRONMENT.md). Install the Quadlet at `~/.config/containers/systemd/rustysoul.container`, run `systemctl --user daemon-reload`, then `systemctl --user start rustysoul.service`. Its image can be pinned to a release tag. Configure login lingering if the service must run without an active user session. Avoid starting multiple instances on the same ports or database.

Upgrade by backing up state, pulling the desired image and recreating the existing service while preserving volumes. Do not use `down -v` unless intentionally deleting named-volume data. Never replace an established installation's service-management mechanism with a second competing service.

Build from source with `docker build -t rustysoul:local .`. The build installs frontend dependencies, builds the UI, runs unit tests and compiles a release binary. Runtime needs no Node or Rust toolchain. [Testing](TESTING.md) describes the four host/bridge and named/bind installation cases. Image labels link the exact revision and source repository; licences ship in `/usr/share/doc/rustysoul`.
