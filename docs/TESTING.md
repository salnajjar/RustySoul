# Verification

Use Linux, Rust 1.100, Node 24, Python 3.11+, a compiler and zlib development headers. Build browser assets before Cargo commands. Run from the repository root:

```sh
cd web
npm ci
npm run build
npm test
npx playwright install --with-deps chromium
npm run test:e2e
cd ..
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --manifest-path vendor/soulseek-rs-lib/Cargo.toml
cargo build --locked --release
python3 scripts/smoke.py target/release/rustysoul
cd web && npm audit --omit=dev --audit-level=moderate
```

The pre-beta baseline is 162 application tests, 385 SDK unit tests, 74 UI unit/component tests and 136 desktop/mobile browser cases. Browser tests inject fixtures into the real frontend; they do not connect to a public Soulseek account. Tests include authenticated API boundaries, SQLite persistence, transfer scheduling/resumption, independent wire fixtures, malformed input handling and UI selection/layout. Current run results are authoritative; counts change as tests evolve.

## Containers

```sh
docker build -t rustysoul:ci .
python3 scripts/container-matrix.py --engine docker --image rustysoul:ci --network host --storage named
python3 scripts/container-matrix.py --engine docker --image rustysoul:ci --network host --storage bind --bind-data
python3 scripts/container-matrix.py --engine docker --image rustysoul:ci --network bridge --storage named
python3 scripts/container-matrix.py --engine docker --image rustysoul:ci --network bridge --storage bind --bind-data
```

The matrix creates isolated synthetic state, verifies first-login credential replacement, UI assets, authenticated API, mounts and restart persistence, then removes its test containers/volumes. Production services and media should never be used as fixtures. Rootless Podman and native systemd installation have also been exercised; CI primarily verifies Docker on Linux amd64.

## Independent implementations

All 87 SDK server cases were exercised against real isolated Soulfind servers without skips. The slskd and Nicotine+ matrices include direct/reverse connections, browse, queues, resume and fragmented traffic. These runners are opt-in, require external source/images and use private synthetic data. See [Soulfind/slskd](SOULFIND_SLSKD.md) and [Nicotine+](INTEROPERABILITY.md), including the tested versions and external-client boundaries. External source trees are not distributed here. Optional SDK end-to-end cases can skip when their server is unavailable; ordinary unit-test success is not an independent interoperability result.

## Fuzzing and extended checks

The upload retry fixture in `tests/fixtures/upload_retries.rs` uses independent loopback server/control/file sockets to verify a transient refusal, offline waiting without a payload connection, a fresh offer and byte-exact resumption. Unit checks cover durable budgets, cancellation, manual resets and the stopped-service lock. Release-check tests cover semantic version ordering, trusted installation links, conditional requests, response/time limits, backoff and API authorization; browser checks cover notices and settings. No production account or public Soulseek traffic is used by these fixtures.

The repository includes `fuzz/` targets and `scripts/seed-fuzz-corpus.py`. Install cargo-fuzz and nightly Rust, generate a corpus, then run individual targets with bounded time and memory appropriate to the test host. Generated corpora/crashes are local artifacts and excluded from Git. See `fuzz/Cargo.toml` for available targets. Long-running campaigns and resource/soak results must be recorded separately; a short CI pass cannot establish all network, router or large-library behavior.

GitHub Actions runs portable checks for pushes and releases. Do not add credentials, personal logs or production backups to workflow artifacts. An independent security review and broader field testing remain desirable before claiming production readiness.
