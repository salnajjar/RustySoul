# Encrypted backup and restore

Settings > Backup & restore creates and restores a portable snapshot of RustySoul application state. The same release executable supports stopped-service CLI recovery. This release validates schema 11 through 18 backups against their exact expected structures. Schema-11/12/13/14/15/16/17 restores migrate to schema 18 on startup; it is not an arbitrary SQLite importer or an automatic downgrade/migration tool.

## Browser workflow

1. Enter your current administrator password and a separate backup passphrase of at least eight characters, confirm the passphrase, then choose **Create encrypted backup**.
2. Download the `.rsb` file and keep it separately from its passphrase. Forgotten passphrases cannot be recovered. Downloading does not change the live account or network connection.
3. To restore, select the file and its passphrase, then choose **Validate backup**. Review the source administrator name, timestamp, transfer counts and exact folder mappings.
4. Enter the current installation's administrator password and type `RESTORE`. Confirming replaces application state and restarts the daemon through its supervisor. Sign in with the administrator username/password from the backup.

Validation and creation can be cancelled. Only one expensive backup operation, including an active file download, runs at once. Temporary server copies are removed on discard, replacement or process exit; marked workspaces left after an abnormal exit are removed at startup. Keep the downloaded file for later recovery. Successful restore revokes all browser sessions, including sessions saved in the backup, and all external API keys. External API access is disabled after restore.

## Included and excluded

Included: administrator password hash, encrypted Soulseek credentials and their encryption key, saved settings/profile/picture, rooms and retained chat, buddies/rules, searches/results, transfer history, saved local priorities/queue order, retry state, bandwidth counters and retained graph buckets. The original compacted database includes the file index for snapshot completeness; restore discards and rebuilds that index from the mounted library.

Excluded: installation-specific external API keys/settings (`api-access.sqlite3`), library/media bytes, completed or partial download bytes, external config files, environment/default passwords, certificates, container mounts, HTTP bindings and systemd/Quadlet definitions. Continue backing up media and deployment configuration separately. A restored history row does not recreate a missing file or reconcile files downloaded since that snapshot. Startup housekeeping avoids moving/deleting staging media based on restored historical records; explicit new transfers still use normal finalization.

Restore requires all recorded folders to fit the destination's allowed location roots and its library/download access checks. Use identical container mount paths on another server, or prepare the native installation with matching paths first. The UI does not silently remap folders. The restored Soulseek startup connection and UPnP are disabled; unfinished downloads are paused and active upload history is interrupted. Review Network settings before reconnecting. The one-time index rebuild does not enable the ordinary index-on-startup preference.

## Offline recovery

Stop the daemon before **any** CLI backup operation, including inspection. The exclusive data-folder lock refuses a running installation. Run the executable as the same service UID and use the same config/mounts. Passphrases are accepted through stdin or a protected file, never as a command-line argument.

Native example (the protected passphrase file must be readable by the service account):

```sh
sudo systemctl stop rustysoul-native.service
sudo -u rustysoul /usr/local/bin/rustysoul-native \
  --config /etc/rustysoul-native/rustysoul.toml \
  --inspect-backup /var/lib/rustysoul-native/recovery.rsb \
  --backup-passphrase-file /var/lib/rustysoul-native/backup-passphrase
# After reviewing the summary, repeat with --restore-backup and --confirm-restore.
sudo systemctl start rustysoul-native.service
```

For the deployed rootful Podman installation, preserve the existing Quadlet. Stop its unit, run a temporary container using the stopped container's mounts and image, then start the unit again. In Bash:

```sh
sudo systemctl stop rustysoul.service
read -rsp 'Backup passphrase: ' backup_pass; printf '\n'
printf '%s\n' "$backup_pass" | sudo podman run --rm -i --network none \
  --read-only --cap-drop ALL --security-opt no-new-privileges \
  --volumes-from rustysoul --entrypoint rustysoul \
  localhost/rustysoul:ui-credentials \
  --config /etc/rustysoul/rustysoul.toml \
  --inspect-backup /var/lib/rustysoul/recovery.rsb --backup-passphrase-stdin
unset backup_pass
# Repeat with --restore-backup and --confirm-restore after inspection.
sudo systemctl start rustysoul.service
```

Place `recovery.rsb` inside the mapped private data directory first, readable by UID 10001. For CLI creation replace inspection with `--create-backup /var/lib/rustysoul/new-backup.rsb`; an existing output file is never overwritten. Inspection/creation exit without opening the network. A bare process must be restarted manually; the normal main executable, container restart policy and native systemd service handle UI-triggered restart (exit 75).

## Boundaries and recovery data

The database limit is 2 GiB. The upload envelope allows 4 MiB more for encryption/metadata; the nginx example allows multipart overhead separately. SQLite snapshotting, upload and cryptographic framing stream through bounded buffers. Temporary disk space is needed for snapshot/encrypted copies and, on restore, a rollback snapshot; capacity checks retain an extra 64 MiB floor. Concurrent external writers can still consume space. Export/validation work has cancellation and deadlines; creation, validation/upload and restore preparation share a 30-minute HTTP deadline through both application middleware layers (matching the deployed proxy). Ordinary requests retain their shorter deadline.

Restore uses a journal, fsync and a stopped-daemon database/key swap. Startup can complete a swap interrupted between database and key installation; invalid staging recovers the previous pair. `restore-last-result.json` reports the outcome. The newest two successful rollback sets remain under the private data directory as `restore-history/<uuid>/rollback-ready/` (database plus matching key). Failed restore directories use `restore-rejected-<uuid>` and may retain a private rollback pair for diagnosis; review/remove these only with the service stopped and a separate verified backup. These local rollback sets contain unencrypted private application state protected by directory permissions; they are not portable encrypted exports. Recover database and key together, never one without the other.

Encryption uses Argon2id with a random salt and ChaCha20-Poly1305 authenticated 64 KiB frames with unique per-frame nonces, authenticated framing and a required final frame. SQLite integrity/relationships, exact schema, credentials, administrator hash parameters, saved settings and folder/queue safety are validated before staging. This is an application-level implementation tested with corruption, interrupted-swap and container recovery fixtures; it has not had an independent cryptographic/security audit.
