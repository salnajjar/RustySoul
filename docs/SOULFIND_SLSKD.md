# Soulfind and slskd interoperability

`scripts/verify-soulfind-slskd.py` creates temporary Podman pods on the Linux host. Each client case runs the actual Soulfind server, slskd client and a separate RustySoul instance. No production account, library, database, configuration, history or download directory is mounted. The internal test network has no default route, no published ports and no connection to public Soulseek. All application containers run as UID 10001 with a read-only root, bounded tasks/memory and temporary writable state.

The upstream images are [Soulfind's official image](https://github.com/soulfind-dev/soulfind#container-image) and [slskd's official image](https://github.com/slskd/slskd/blob/master/docs/docker.md). The tested Soulfind image identifies itself as Nov-1-2026, and slskd as 0.26.2.0+e472ecd7. The runner saves exact image IDs/digests in `images.json`; tags alone do not identify a reproducible test run. Soulfind is a local test server, not a replacement for the public service.

## Run the checks

On the Linux host with Podman, `nsenter`, `ip` and Python installed:

```sh
sudo podman pull ghcr.io/soulfind-dev/soulfind:latest
sudo podman pull docker.io/slskd/slskd:latest
sudo podman build -f deploy/Dockerfile.interop-runner -t localhost/rustysoul:interop-runner .
sudo python3 scripts/verify-soulfind-slskd.py \
  --rust-image localhost/rustysoul:local --output artifacts/soulfind-slskd
```

The default matrix covers direct and forced reverse connections, both with normal traffic and delayed seven-byte stream-prefix writes. `--mode direct` or `--mode reverse` selects one case; add `--fragment` to that case to enable the proxy. Unlike the Nicotine+ reference snapshot, no initial-frame coalescing exception is needed for slskd.

The server remains real in every case. A relay passes Soulfind's messages through, changing only peer reachability: direct cases withhold reverse requests; reverse cases advertise unavailable direct ports while passing actual server-brokered P/F requests. Fragmented cases use transparent loopback peer proxies. The report includes actual address replies and brokered connection types.

Assertions cover authenticated network logins, bidirectional browse, Unicode filenames, empty files, exact downloaded bytes, real queue-position responses, reopening upload admission, RustySoul partial pause/disconnect/reconnect/resume, and slskd active cancellation/requeue. Tests inspect completed files through isolated mounts, not just success statuses. Each run records client and server logs, images and a JSON result, then removes its pods, networks and temporary data.

Soulfind discovers its local address by UDP-connecting to 10.255.255.255 when a client uses loopback. The runner routes that one address to loopback inside the pod namespace; it does not add an external/default route. slskd extracts its .NET bundle into its temporary filesystem. No production network or container profile is changed.

## Required-server SDK suite

Build the existing SDK's server test executable and pass its Cargo-reported executable path:

```sh
cargo test --locked --manifest-path vendor/soulseek-rs-lib/Cargo.toml --test e2e --no-run
sudo python3 scripts/verify-soulfind-slskd.py \
  --sdk /absolute/path/to/the/e2e-executable --output artifacts/soulfind-sdk
```

The runner extracts the official image's static Soulfind binary. Every SDK case starts a fresh actual server/database in the isolated namespace. `SOULSEEK_E2E_REQUIRED=1` prevents missing-server skips; the runner also rejects skip messages from optional privilege/admin helpers and requires all 87 cases to pass. Its image includes SQLite's CLI for those helpers. `--sdk-test NAME` runs one exact case for diagnosis.

The host's default container AppArmor profile prevented the SDK harness from signalling/reaping its own child servers. Only the isolated SDK controller runs with `apparmor=unconfined`, retaining its non-root UID, empty capabilities, read-only root, resource limits and isolated network. RustySoul, slskd, Soulfind containers and the production daemon keep their existing profiles.

## Findings and boundaries

slskd uses the legacy direction-0 TransferRequest. RustySoul previously treated it as a direction-1 offer for an existing local download and rejected it as unrequested. It now replies Queued and submits the requested filename to the same shared-file resolution, sharing policy, privilege, concurrency and upload queue used by modern QueueUpload messages. The remote token never becomes a local download reservation. Unknown directions are rejected. A cancelled incoming download offer receives the standard Cancelled denial rather than the generic terminal-file denial.

The previously optional SDK suite contained assertions predating offered uploads at queue position zero, typed upload timeouts and explicit negative user lookups. These assertions now check the current behavior: waiting places remain 1/2 with a separately visible offered slot, unanswered offers become TimedOut, and unknown accounts leave the watch list while retaining `exists=false` without fabricated presence/stats.

slskd 0.26.2 publishes an errored upload state before releasing its per-file task lock. Requeueing during that interval can leave its replacement queued. The resume fixture waits for the actual previous slskd task's completion log before requeueing; it does not change slskd's source or reset its state. This peer retry-timing limitation remains a boundary of these tests. Arbitrary timing races, other versions, real NAT/firewall combinations and public-server behavior are separate coverage.

This is an external-client compatibility note, not an outstanding RustySoul defect. Repairing slskd is outside our implementation scope. Waiting for task cleanup is a test-fixture accommodation, not a general production workaround built into RustySoul. The observation applies to the tested version and does not establish the status of newer upstream versions.
