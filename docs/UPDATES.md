# Release notifications and upgrades

RustySoul checks the project's latest stable GitHub release at daemon startup and every six hours. This runs in one background worker with a ten-second request deadline, a 512 KiB response limit and conditional requests. Failures stay in the update settings rather than producing repeated system banners. GitHub rate limits delay further checks. No account, credentials, searches or library data are sent; the request identifies the RustySoul version.

In **Settings > Status & privacy > Software updates**, disable automatic checks or request a check now. Manual checks share the daemon's rate limit. Disabling checks also hides the notice. A notice can be dismissed for that version; a later version can still appear. RustySoul never downloads or installs an update automatically.

**Docker/Podman:** the notice links to the new version's release notes and container instructions, and displays its exact image tag. Pull that image and recreate the existing container using its original data volume, mounts, environment and service management. Keep the data volume. Linux host networking and bridge installations retain their existing port settings. New stable installations default to `ghcr.io/salnajjar/rustysoul:latest`; a pinned version such as `:0.1.0` makes upgrades explicit. Older beta installations must switch their image tag to move to stable releases.

**Native Linux:** the notice links directly to the new release's x86-64 bundle when that asset is available, plus its checksum and the versioned native installation instructions. On other architectures it links to the native instructions instead of offering an incompatible executable. Stop the service before replacing the executable; retain the application state and service configuration.

Installation type is detected from container runtime markers, with a packaged-container fallback. Set `RUSTYSOUL_INSTALL_TYPE=docker`, `podman`, `container` or `native` to override detection if your runtime hides those markers. This changes the update instructions only. It does not alter storage or network configuration.

Back up application state before upgrading. Rollback requires the earlier executable/image **and its matching database**; an older binary must not be pointed at a newer schema. See [backups](BACKUPS.md), [containers](CONTAINERS.md) and [native installation](NATIVE.md).
