# Library and search update

- Administrator passwords now require at least eight characters. The account replacement gate remains in place for first deployment.

- Offline search input and submission are disabled. The sign-in prompt opens Soulseek account settings, which appear before administrator settings.

- Saving a Soulseek account first attempts an actual server login. Failed attempts leave the saved account unchanged; accepted attempts encrypt and save the credentials and retain the authenticated connection. The UI reports progress, success, or failure.

- Results are deduplicated and grouped by user, then parent folder. Users reorder by the number of currently matching files as snapshots arrive; username breaks ties. Folder groups have independent incremental display limits so a large folder does not hide other users' files.

- All / Lossless / Lossy filters apply live to returned results. Classification uses recognizable extensions. Ambiguous containers such as `.m4a` remain in All because extension alone does not establish AAC versus ALAC.

- Settings provides a download destination and shared-library roots. Paths must already exist and be accessible **inside the container**. The destination must be writable; library mounts can be read-only. Shared roots cannot overlap or expose the daemon's private credential/data directory. Existing transfers retain their original saved destination. Existing files are not moved when paths change.

- The container defaults to mounted `/downloads` and `/library`; see STORAGE_MOUNTS.md for mount configuration. Settings includes a mount-scoped folder browser and supports multiple disjoint share roots. Add explicit Podman/Docker mounts and location_roots entries for additional host directories.

- Filesystem events trigger debounced background scans. A 60-second reconciliation covers missed events and filesystems without functioning hooks. SQLite caches file size, modification/change timestamps, file identity where available, and parsed audio metadata. Metadata checks still walk configured directories; unchanged files skip content/header parsing and redundant metadata writes. Creation, modification, deletion, and manual re-indexing are covered by tests.

- A small MIT-licensed adapter to the vendored `soulseek-rs-lib` 20.0.0 installs cached immutable serving snapshots without invoking its full scanner. SQLite is the durable index; protocol searches use its in-memory serving snapshot.



Results now display peer-reported upload speed, with user sorting by matches, speed, name or available slots and file sorting by name, size, bitrate or duration. Select/deselect a file, folder, user or all matching files for a batch download. Filtered-out selections remain explicitly counted. Admission validates the entire batch and queue space before saving it; individual peer transfers can subsequently fail. Room lists sort by users, name or the latest message observed by this client; unknown timestamps sort last.

See [verification](TESTING.md) for current test instructions and coverage.
