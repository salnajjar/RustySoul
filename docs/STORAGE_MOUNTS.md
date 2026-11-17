# Storage mapping

Container paths are independent of host paths. Configure mounts at installation, then select their container paths in Settings -> File locations & library.

| Purpose | Container path | Access |
| --- | --- | --- |
| SQLite, credential key, private state | `/var/lib/rustysoul` | Read/write, UID/GID 10001 |
| Downloads and partial files | `/downloads` | Read/write, UID/GID 10001 |
| Shared library | `/library` | Read-only |

Compose defaults to separate named volumes. Set `RUSTYSOUL_DOWNLOADS_SOURCE=/srv/downloads/music` and `RUSTYSOUL_LIBRARY_SOURCE=/srv/music` in `.env` to bind existing folders. Create them first and grant the container account only the access it needs. Preserve existing media ownership; ACLs are suitable where ownership belongs to another service. Rootless Podman maps container IDs through a user namespace; use `podman unshare` or appropriate ACLs when preparing bind mounts.

Add further read-only mounts, such as `/srv/other-music:/library-extra:ro`, before adding those paths to the configured location roots and selecting them in the UI browser. The daemon can only browse paths allowed by configuration and visible inside its filesystem namespace. A UI path cannot create a container mount.

Changing mounts requires recreating the container; changing an indexed library within an existing allowed mount is done in Settings. The cached index supports incremental scans, filesystem hooks and a manual re-index. Startup indexing is disabled by default. Never share private state or the credential key.

Preserve state and downloads during upgrades. Use the encrypted [backup workflow](BACKUPS.md), or stop the daemon before copying SQLite and its credential key. Do not run two instances against one database. Media files are not included in application backups. See [storage protection](STORAGE_PROTECTION.md) for free-space limits.
