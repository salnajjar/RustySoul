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

RustySoul keeps unfinished downloads in `/downloads/.incomplete`. This private staging area and its per-transfer folders must remain owned by the daemon account (container UID 10001, or its mapped host UID with rootless Podman). The download root and completed media can retain their existing host ownership and ACLs. Exclude `.incomplete` from media importers and recursive ownership/permission resets. Granting `777` access does not let a different owner secure a staging folder and exposes partial files unnecessarily. If logs report **Cannot secure download staging folder** or **Permission denied**, check ownership inside the container, repair only RustySoul's staging area, then use **Retry failed downloads**; keep existing partial data.

Completed downloads use the destination folder's normal file-creation policy, including its default POSIX ACL, inherited setgid group and the daemon's umask where no default ACL is configured. Incomplete files and their staging folders remain private. Normal new downloads retain their payload inode at completion; older partials or a changed destination group use a filesystem reflink when available, with a bounded copy fallback. The original data is retained until preparation succeeds, and filename collisions still receive a suffix instead of overwriting an existing file. The daemon does not need extra capabilities or membership in every destination group.

RustySoul automatically removes empty transfer-key folders from `.incomplete` at startup and approximately once a minute, in small batches that keep the daemon responsive. It checks current and retained download locations, skips directories still in use, and does not follow staging symlinks. Cleanup is non-recursive and never deletes files: paused, failed, resumable and zero-byte partials remain available. Completed or cancelled transfers and cleared history no longer leave empty staging folders indefinitely. The private `.incomplete` root itself is retained. Cleanup totals appear in Logs & activity without a banner.
