# Native Linux installation

The same executable runs directly on Linux; Node, Docker and Podman are not required at runtime. The current x86-64 runtime bundle is built in Debian Bookworm and runs with glibc 2.36 or newer. It has been tested in the container runtime and as a native Linux service. Build from source for older distributions. Installation uses Python 3.11+, systemd and standard Linux account tools.

```sh
tar -xzf rustysoul-linux-x86_64.tar.gz
cd rustysoul
sudo python3 install-native.py
sudo systemctl enable --now rustysoul-native.service
```

Open http://127.0.0.1:8484, or use an SSH tunnel from another computer. The initial administrator login is admin / change-me-now, with mandatory username and password replacement. Soulseek credentials are entered and verified in the UI. Use an HTTPS reverse proxy for remote browser access, as with the container installation.

Choose media directories at first installation:

```sh
sudo python3 install-native.py \
  --library /srv/music \
  --downloads /srv/downloads/music
```

Repeat --library for multiple roots. Existing media ownership is preserved. The service account must have read access to libraries and write access to downloads before installation; default UID 10001 is also used by the container. Without these flags, installation creates empty /srv/rustysoul-native/library and downloads folders. --port chooses the HTTP port on first installation. File-transfer ports remain configurable in the UI.

The service is rustysoul-native.service, the executable /usr/local/bin/rustysoul-native, configuration /etc/rustysoul-native/rustysoul.toml, bootstrap environment /etc/rustysoul-native/daemon.env and private SQLite state /var/lib/rustysoul-native. The service runs as a dedicated non-root account. Its filesystem is read-only except for private state, temporary storage and the configured download directory. These restrictions use [systemd filesystem protections](https://freedesktop.org/software/systemd/man/latest/systemd.exec.html). If changing download paths, update ReadWritePaths in a systemd override and reload/restart the service as well as updating location_roots in configuration. Shared folders under /home are blocked by ProtectHome; use /srv or a deliberate service override.

Upgrade by extracting the new bundle, rerunning install-native.py and restarting rustysoul-native.service. Existing configuration, database, credential key and media are retained; prior executable and unit files are backed up. Installation does not start or stop any service automatically. To remove the executable/service, stop and disable the service first; retain private state and media until you deliberately choose to remove them.

The container deployment remains independent. Stop it before running a native instance with the same ports or Soulseek account. Do not point two instances at the same database. Moving an established container account to native requires an offline backup of both SQLite and credentials.key and explicit translation of container paths to host paths; the installer intentionally does not move or reset your existing container state.

For Linux without systemd, run the executable directly with an absolute configuration path under a dedicated account, using your preferred supervisor. Build prerequisites and the native configuration format are documented in the repository README.

Startup connection defaults to enabled once verified Soulseek credentials exist. Startup indexing defaults to disabled; the cached index, automatic hooks and reconciliation remain available. Both options are editable in Settings, with the same user/profile/buddy and paged share browsing interface as the container deployment.

Download storage protection defaults to a 1 GiB free-space floor and is managed under Settings -> File locations & library. It is stored in SQLite with the other administrator settings and survives native upgrades/restarts. See STORAGE_PROTECTION.md for partial-file waiting/resumption and the limits of this best-effort floor.

Encrypted browser backup/restore and stopped-service CLI recovery are described in BACKUPS.md. The supplied Restart=on-failure service restarts automatically after a confirmed UI restore.


The native executable includes the same durable transfer priority, manual queue order, Ctrl/Cmd/Shift selection and per-user cancellation controls. See TRANSFER_QUEUE.md in this bundle.
