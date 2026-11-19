# Security analysis and reviewed findings

The Verify workflow calls the reusable security workflow for Rust, JavaScript/TypeScript, Python and GitHub Actions. Release publication waits for Verify, including its security jobs. A weekly scan checks for new findings as CodeQL evolves. All default security queries remain enabled, including those for the vendored protocol implementation and tests.

CodeQL first writes its original SARIF. `scripts/review-codeql.py` writes a separate reviewed SARIF and a report giving the location and reason for every reviewed finding. Both versions and the review report are retained as workflow artifacts for seven days. Unreviewed Critical findings block verification and publishing, matching the requested release policy. High and Medium findings stay visible in the reviewed SARIF and GitHub; maintainers can opt into a stricter local gate with `--fail-on high`. Unknown rule metadata, scanner errors and CodeQL's low-analysis-quality diagnostic also fail the gate. Findings are uploaded before that gate so real regressions remain visible in GitHub.

## Test fixtures and intentional values

The hard-coded cryptographic-value query also identifies passwords used only by tests. A pinned, hash-locked Rust AST parser establishes whether a location belongs to `#[test]`, `#[tokio::test]`, a `#[cfg(test)]` module, its out-of-line child modules, or a Cargo integration-test fixture. This exception applies only to the hard-coded-value query. It does not exclude test files from analysis or hide other query results. Comments, raw strings, nearby test functions, and `cfg(any(test, unix))` cannot establish test-only scope. Unrecognized Rust syntax cannot establish an inline test exception. Production imports override test-directory conventions, and unresolved production `include!` paths disable whole-file test assumptions.

Other exceptions are listed in `.github/codeql/reviewed.json` and require unique source expressions plus explicit guards. They cover the environment-overridable first-install password, an encrypted empty-credential tombstone, and generated bearer-token digests. The bootstrap account must replace both username and password before operational API access; existing accounts never fall back to it. The empty record represents removal of credentials, not an account password. Review policy changes alongside their regression tests; do not add file-wide or query-wide exclusions to silence findings.

## Token hashing: alerts 223 and 224

Administrator passwords use salted Argon2id. Integration keys instead contain 32 bytes from the OS cryptographic random generator and use the `rsapi_` prefix followed by 64 hexadecimal digits. SHA-256 stores/indexes those 256-bit keys without persisting their plaintext. The startup compatibility token is also generated, and production authentication uses sessions and integration keys. These digests do not hash human passwords. Replacing them with a password KDF would introduce significant work per API request and invalidate existing keys without increasing their entropy.

The reviewed expressions remain limited to those token digests and require the corresponding generation/validation guards. A different password hash or another data flow is retained for review. The underlying weak-hashing query is enabled.

## Backup paths: alert 226 and related findings

CodeQL's Rust model treats Axum `State` as if it were request data. The reviewed status/inspect paths use private immutable `ApiState.data_dir`, copied from startup configuration. HTTP requests cannot select this directory. Free-space checks open that configured directory; workspaces are created under it by `tempfile`, and restore status uses a fixed filename.

The path exception requires the known endpoint, a State-derived data-directory flow, the reviewed sink function, and configuration guards. Additional request-derived flows, JSON/query extraction, missing guards and new sinks remain findings. The path-injection query stays enabled.

Linux backup/status and credential-key reads reject symlinks and non-regular files, check the opened file handle, and use no-follow/nonblocking open flags. Restore-status reads are capped at 8 KiB even if a file grows during reading. SQLite backup reads use `SQLITE_OPEN_NOFOLLOW`. These checks strengthen filesystem handling; they do not turn arbitrary HTTP paths into approved paths.

## Cryptographic buffers and protocol compatibility

Credential keys are generated from the OS random source and loaded from the private persisted key file. A zero-initialized read buffer was never a fixed encryption key; loading the bytes explicitly makes that distinction clearer to both readers and the scanner. Backup nonces combine the archive's random prefix with a unique little-endian chunk counter. The new construction preserves the existing nonce bytes and archive format. Username-only backup validation no longer supplies a dummy password. The isolated stress tool uses a fresh OS-random account password unless `STRESS_PASSWORD` is supplied.

Peer-registry debug events use actor identifiers instead of logging peer usernames. Functional transfer/user diagnostics remain available elsewhere in the private application logs. The Soulseek protocol's required login encoding is preserved.

## Running the review locally

Install development-only review tools with their recorded hashes:

```sh
python -m pip install --require-hashes -r .github/codeql/requirements.txt
python -m unittest discover -s scripts/tests -p test_review_codeql.py
```

Use the same CodeQL version/query suite as the workflow. Rust semantic extraction needs the project's Rust toolchain, `rust-src`, resolved Cargo dependencies and the built `web/dist` resources used by `include_dir`; an extraction with missing dependencies is not adequate security evidence.

After producing the original SARIF:

```sh
python scripts/review-codeql.py --input raw.sarif --output reviewed.sarif --report review.json
python scripts/review-codeql.py --input reviewed.sarif --gate-only
```

Never overwrite the original report. A review does not dismiss alerts through GitHub's API or change repository settings.

## Repository scanner setup

The checked-in workflow uses GitHub's Advanced setup. Default setup must be disabled when this workflow owns uploads; [GitHub rejects CodeQL SARIF uploads while Default setup is active](https://docs.github.com/en/code-security/reference/code-scanning/sarif-files/troubleshoot-sarif-uploads/default-setup-enabled). When migrating a fork from Default setup, keep its existing scanner enabled until the advanced workflow is present. Do not leave both scanners running or remove the existing scanner before its replacement is ready. Run the new workflow on the default branch, inspect the raw/reviewed artifacts and confirm the published alert states. Local verification does not close alerts on an unchanged GitHub commit.

The existing High advisory about logging public Soulseek usernames remains visible; public usernames in private operational logs are not passwords, and the logger query is not suppressed. The Medium listener advisory in the isolated container-installation fixture also remains visible. That fixture listens on all interfaces so bridge-network containers can reach its mock Soulseek server; it is not the daemon's HTTP binding and does not block the Critical gate.
