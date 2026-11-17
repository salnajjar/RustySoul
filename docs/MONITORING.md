# System status and monitoring

Open **Settings & status → Status & privacy → System status**. The page shows
daemon uptime, CPU, resident memory and thread count; cgroup memory/task budgets
and CPU quota when available; SQLite and WAL sizes; available space at the data
and current download folders; queue capacity, shared-library totals and transfer
counts/speeds across the entire retained history.

Resource sampling runs on one background thread every five seconds. Requests
and the protocol control loop read its cached result. Sampling neither scans
your library nor creates a new database history. Missing measurements remain
unavailable, and a delayed sample or interrupted browser connection is labelled.
There are shortcuts to logs, network settings and file locations.

CPU percentages use one fully occupied core as 100%, so multi-core work may
exceed 100%. Resident memory is the daemon's RSS. Cgroup memory can also include
charged cache and other processes. For nested cgroups, the memory/task pair
with the smallest observed remaining headroom is shown; a shared parent budget
is labelled. Limit probing uses cgroup v2; other setups may omit these readings. CPU quota uses the smallest finite ancestor quota. Unreported
limits are not interpreted as zero or as proof that the host has no limits.

Filesystem capacity is measured for the configured paths. Available bytes
exclude space reserved from the daemon user. Two folders on the same filesystem
share that capacity; their free-space values must not be added. Existing
downloads retain their original destinations when Settings changes the current
destination. The readings describe available capacity, not a disk reservation
or download quota. The page flags less than 5% available space and at least 85%
usage of a reported cgroup limit. Those indicators do not change settings or
automatically stop transfers.

## Metrics endpoint

`GET /api/v1/metrics` returns UTF-8 Prometheus text format 0.0.4 with the explicit
content type `text/plain; version=0.0.4; charset=utf-8`. It uses the same session
authentication and mandatory initial-account replacement gate as the API.
The browser's **View metrics** link works with its existing protected cookie.
An external collector can use an existing API session token:

```sh
curl --fail --silent --show-error \
  -H "Authorization: Bearer $SESSION_TOKEN" \
  https://your-rustysoul-host/api/v1/metrics
```

Use the trusted HTTPS address and protect the collector's credential. API
sessions carry administrator permissions, expire after 30 days, and are revoked
when the account changes; renewal is the collector's responsibility. No new
permanent or public monitoring credential is created.

The endpoint exposes fewer than 100 bounded series, including:

| Metric | Meaning |
|---|---|
| `rustysoul_network_connected` | 1 when connected to Soulseek, otherwise 0 |
| `rustysoul_system_sample_timestamp_seconds` / `rustysoul_system_sample_age_seconds` | Last sample time and its current age |
| `rustysoul_uptime_seconds` | Daemon uptime, reset at startup |
| `rustysoul_process_cpu_seconds_total` / `rustysoul_process_cpu_percent` | Process CPU counter and recent usage |
| `rustysoul_process_resident_memory_bytes` / `rustysoul_process_threads` | Process RSS and threads |
| `rustysoul_cgroup_memory_*` / `rustysoul_cgroup_tasks_*` / `rustysoul_cgroup_cpu_cores` | Reported limiting cgroup usage/budgets |
| `rustysoul_storage_data_*` / `rustysoul_storage_downloads_*` | Total, used and available filesystem bytes by fixed role |
| `rustysoul_database_bytes` / `rustysoul_database_wal_bytes` | Current SQLite main-file and WAL sizes |
| `rustysoul_download_queue_used` / `rustysoul_download_queue_capacity` | Unfinished records and configured queue capacity |
| `rustysoul_library_files` / `rustysoul_library_folders` / `rustysoul_library_scanning` | Current serving index and scan state |
| `rustysoul_transfers{direction,state}` | Retained record counts with fixed direction/state labels |
| `rustysoul_download_speed_bytes_per_second` / `rustysoul_upload_speed_bytes_per_second` | Aggregate active speeds across all history rows |
| `rustysoul_payload_downloaded_bytes_total` / `rustysoul_payload_uploaded_bytes_total` | Durable lifetime payload volumes |

Labels never include usernames, filenames, paths, rooms, search terms, errors,
message contents or session credentials. Unknown transfer states map to `other`.
Unavailable readings are omitted instead of exported as fabricated zeroes.
Counters describe file payload, not total network traffic, and lifetime counters
survive ordinary daemon restarts and transfer-history clearing. Process CPU
time resets on restart. A collector should handle normal counter resets after
a database restore. Polling metrics does not trigger extra sampling or engine
commands. Store any long-term operational history in your monitoring system.

The format follows the [Prometheus exposition specification](https://prometheus.io/docs/instrumenting/exposition_formats/).
