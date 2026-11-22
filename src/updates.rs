//! Anonymous release checks, off the protocol and HTTP workers. Never installs code.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const REPO: &str = "https://github.com/salnajjar/RustySoul";
const ENDPOINT: &str = "https://api.github.com/repos/salnajjar/RustySoul/releases/latest";
const INTERVAL: u64 = 6 * 3600;
const MAX_BODY: u64 = 512 * 1024;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Release {
    pub version: String,
    pub release_url: String,
    pub instructions_url: String,
    pub image: Option<String>,
    pub native_url: Option<String>,
    pub checksum_url: Option<String>,
}
#[derive(Clone, Serialize)]
pub struct View {
    pub current_version: &'static str,
    pub install_type: String,
    pub enabled: bool,
    pub checking: bool,
    pub checked_at: Option<u64>,
    pub next_check_at: Option<u64>,
    pub available: Option<Release>,
    pub error: Option<String>,
}
pub struct Monitor {
    view: Mutex<View>,
    request: mpsc::SyncSender<()>,
    allowed_at: AtomicU64,
}
pub struct Guard(Arc<AtomicBool>);
impl Drop for Guard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn install_type() -> String {
    let configured = std::env::var("RUSTYSOUL_INSTALL_TYPE").unwrap_or_default();
    detect_install(
        &configured,
        std::path::Path::new("/run/.containerenv").exists(),
        std::path::Path::new("/.dockerenv").exists(),
        std::env::var("RUSTYSOUL_PACKAGED_CONTAINER").is_ok(),
    )
    .into()
}
fn detect_install(configured: &str, podman: bool, docker: bool, packaged: bool) -> &str {
    match configured {
        "docker" | "podman" | "native" | "container" => configured,
        _ if podman => "podman",
        _ if docker => "docker",
        _ if packaged => "container",
        _ => "native",
    }
}
impl Monitor {
    pub fn view(&self, enabled: bool) -> View {
        let mut view = self.view.lock().unwrap_or_else(|p| p.into_inner()).clone();
        view.enabled = enabled;
        if !enabled {
            view.next_check_at = None;
        }
        view
    }
    pub fn check(&self, enabled: bool) -> bool {
        enabled
            && now() >= self.allowed_at.load(Ordering::Acquire)
            && self.request.try_send(()).is_ok()
    }
}
pub fn start(
    snapshot: tokio::sync::watch::Receiver<crate::model::Snapshot>,
) -> Result<(Arc<Monitor>, Guard)> {
    let (request, receiver) = mpsc::sync_channel(1);
    let monitor = Arc::new(Monitor {
        view: Mutex::new(View {
            current_version: env!("CARGO_PKG_VERSION"),
            install_type: install_type(),
            enabled: snapshot.borrow().network.check_for_updates,
            checking: false,
            checked_at: None,
            next_check_at: None,
            available: None,
            error: None,
        }),
        request,
        allowed_at: AtomicU64::new(0),
    });
    let worker = monitor.clone();
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = stopped.clone();
    std::thread::Builder::new()
        .name("release-check".into())
        .spawn(move || {
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .https_only(true)
                .max_redirects(0)
                .http_status_as_error(false)
                .timeout_global(Some(Duration::from_secs(10)))
                .timeout_connect(Some(Duration::from_secs(5)))
                .max_response_header_size(16384)
                .user_agent(concat!("RustySoul/", env!("CARGO_PKG_VERSION")))
                .build()
                .into();
            let mut etag = None;
            let mut next = 0;
            let mut was_enabled = false;
            while !stop.load(Ordering::Acquire) {
                let manual = receiver.recv_timeout(Duration::from_secs(1)).is_ok();
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let enabled = snapshot.borrow().network.check_for_updates;
                if !enabled {
                    was_enabled = false;
                    continue;
                }
                if !was_enabled {
                    next = 0;
                    was_enabled = true;
                }
                let timestamp = now();
                if timestamp < worker.allowed_at.load(Ordering::Acquire)
                    || (!manual && timestamp < next)
                {
                    continue;
                }
                worker
                    .allowed_at
                    .store(timestamp.saturating_add(60), Ordering::Release);
                worker
                    .view
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .checking = true;
                let fetched = fetch(&agent, etag.as_deref());
                let mut view = worker.view.lock().unwrap_or_else(|p| p.into_inner());
                view.checking = false;
                view.checked_at = Some(now());
                let delay = match fetched {
                    Ok(reply) => {
                        match apply(
                            &mut view,
                            &mut etag,
                            reply,
                            env!("CARGO_PKG_VERSION"),
                            std::env::consts::ARCH,
                        ) {
                            Ok(delay) => delay,
                            Err(_) => {
                                view.error = Some(
                                    "Could not read release information; will try again later."
                                        .into(),
                                );
                                INTERVAL
                            }
                        }
                    }
                    Err(_) => {
                        view.error =
                            Some("Could not check for releases; will try again later.".into());
                        INTERVAL
                    }
                };
                next = now().saturating_add(delay);
                if delay != INTERVAL {
                    worker.allowed_at.store(next, Ordering::Release);
                }
                view.next_check_at = Some(next);
            }
        })?;
    Ok((monitor, Guard(stopped)))
}
struct Reply {
    status: u16,
    etag: Option<String>,
    retry_after: Option<u64>,
    body: String,
}
fn fetch(agent: &ureq::Agent, etag: Option<&str>) -> Result<Reply> {
    fetch_at(agent, etag, ENDPOINT)
}
fn fetch_at(agent: &ureq::Agent, etag: Option<&str>, endpoint: &str) -> Result<Reply> {
    let mut request = agent
        .get(endpoint)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28");
    if let Some(etag) = etag {
        request = request.header("If-None-Match", etag);
    }
    let mut response = request.call()?;
    let status = response.status().as_u16();
    let etag = response
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .filter(|v| v.len() <= 256)
        .map(str::to_owned);
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    let body = if status == 200 {
        response
            .body_mut()
            .with_config()
            .limit(MAX_BODY)
            .read_to_string()?
    } else {
        String::new()
    };
    Ok(Reply {
        status,
        etag,
        retry_after,
        body,
    })
}
fn apply(
    view: &mut View,
    etag: &mut Option<String>,
    reply: Reply,
    current: &str,
    arch: &str,
) -> Result<u64> {
    match reply.status {
        200 => {
            let release = parse_release(&reply.body, current, &view.install_type, arch)?;
            view.available = release;
            *etag = reply.etag;
            view.error = None;
        }
        304 => {
            view.error = None;
        }
        404 => {
            view.available = None;
            *etag = None;
            view.error = None;
        }
        403 | 429 => {
            view.error =
                Some("Release checking is temporarily rate limited; will try again later.".into());
            return Ok(reply.retry_after.unwrap_or(3600).clamp(3600, 86400));
        }
        _ => {
            anyhow::bail!("Release server unavailable");
        }
    }
    Ok(INTERVAL)
}
#[derive(Deserialize)]
struct GitRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
}
fn parse_release(body: &str, current: &str, install: &str, arch: &str) -> Result<Option<Release>> {
    ensure!(
        body.len() <= MAX_BODY as usize,
        "Release response is too large"
    );
    let release: GitRelease = serde_json::from_str(body)?;
    ensure!(release.tag_name.len() <= 64, "Release version is too long");
    let tag = &release.tag_name;
    let version = semver::Version::parse(tag.strip_prefix('v').unwrap_or(tag))?;
    ensure!(tag == &format!("v{version}"), "Noncanonical release tag");
    if release.draft
        || release.prerelease
        || !version.pre.is_empty()
        || !version.build.is_empty()
        || version <= semver::Version::parse(current)?
    {
        return Ok(None);
    }
    let container = install != "native";
    let asset = "rustysoul-linux-x86_64.tar.gz";
    let has_native =
        !container && arch == "x86_64" && release.assets.iter().any(|a| a.name == asset);
    Ok(Some(Release {
        version: version.to_string(),
        release_url: format!("{REPO}/releases/tag/{tag}"),
        instructions_url: format!(
            "{REPO}/blob/{tag}/docs/{}",
            if container {
                "CONTAINERS.md"
            } else {
                "NATIVE.md"
            }
        ),
        image: container.then(|| format!("ghcr.io/salnajjar/rustysoul:{version}")),
        native_url: has_native.then(|| format!("{REPO}/releases/download/{tag}/{asset}")),
        checksum_url: (has_native
            && release
                .assets
                .iter()
                .any(|a| a.name == format!("{asset}.sha256")))
        .then(|| format!("{REPO}/releases/download/{tag}/{asset}.sha256")),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn http_checks_bound_body_time_and_keep_conditional_headers() {
        use std::io::{Read, Write};
        for mode in ["conditional", "oversized", "slow"] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                assert!(
                    !String::from_utf8_lossy(&request)
                        .to_lowercase()
                        .contains("authorization:")
                );
                match mode {
                    "conditional" => {
                        assert!(
                            String::from_utf8_lossy(&request)
                                .to_lowercase()
                                .contains("if-none-match: \"cached\"")
                        );
                        stream.write_all(b"HTTP/1.1 304 Not Modified\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                    }
                    "oversized" => {
                        let header = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            MAX_BODY + 1
                        );
                        stream.write_all(header.as_bytes()).unwrap();
                        let _ = stream.write_all(&vec![b' '; MAX_BODY as usize + 1]);
                    }
                    _ => {
                        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\nConnection: close\r\n\r\n{").unwrap();
                        std::thread::sleep(Duration::from_millis(750));
                    }
                }
            });
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .proxy(None)
                .http_status_as_error(false)
                .max_redirects(0)
                .timeout_global(Some(Duration::from_millis(300)))
                .build()
                .into();
            let started = std::time::Instant::now();
            let reply = fetch_at(&agent, Some("\"cached\""), &url);
            if mode == "conditional" {
                assert_eq!(reply.unwrap().status, 304);
            } else {
                assert!(reply.is_err());
            }
            assert!(started.elapsed() < Duration::from_secs(3));
            server.join().unwrap();
        }
    }
    fn release(tag: &str) -> String {
        serde_json::json!({"tag_name":tag,"assets":[{"name":"rustysoul-linux-x86_64.tar.gz"},{"name":"rustysoul-linux-x86_64.tar.gz.sha256"}],"html_url":"https://evil.example/","browser_download_url":"https://evil.example/"}).to_string()
    }
    #[test]
    fn stable_ordering_and_trusted_install_links() {
        let native = parse_release(&release("v0.1.0"), "0.1.0-beta.5", "native", "x86_64")
            .unwrap()
            .unwrap();
        assert_eq!(
            native.native_url.as_deref(),
            Some(
                "https://github.com/salnajjar/RustySoul/releases/download/v0.1.0/rustysoul-linux-x86_64.tar.gz"
            )
        );
        assert!(native.image.is_none());
        assert!(native.checksum_url.is_some());
        let podman = parse_release(&release("v0.1.0"), "0.1.0-beta.5", "podman", "x86_64")
            .unwrap()
            .unwrap();
        assert_eq!(
            podman.image.as_deref(),
            Some("ghcr.io/salnajjar/rustysoul:0.1.0")
        );
        assert!(podman.native_url.is_none());
        assert!(
            podman
                .instructions_url
                .ends_with("/v0.1.0/docs/CONTAINERS.md")
        );
        for v in ["v0.1.0", "v0.0.9", "v0.2.0-beta.1"] {
            assert!(
                parse_release(&release(v), "0.1.0", "native", "x86_64")
                    .unwrap()
                    .is_none()
            );
        }
        assert!(parse_release(&release("v1.0.0/../../etc"), "0.1.0", "native", "x86_64").is_err());
        assert!(
            parse_release(
                &" ".repeat(MAX_BODY as usize + 1),
                "0.1.0",
                "native",
                "x86_64"
            )
            .is_err()
        );
        assert!(
            parse_release(&release("v0.2.0"), "0.1.0", "native", "aarch64")
                .unwrap()
                .unwrap()
                .native_url
                .is_none()
        );
    }
    #[test]
    fn detection_allows_host_override() {
        assert_eq!(detect_install("", true, true, true), "podman");
        assert_eq!(detect_install("", false, true, true), "docker");
        assert_eq!(detect_install("", false, false, true), "container");
        assert_eq!(detect_install("", false, false, false), "native");
        assert_eq!(detect_install("native", true, true, true), "native");
    }
    #[test]
    fn conditional_checks_and_backoff_retain_known_release() {
        let mut view = View {
            current_version: "0.1.0",
            install_type: "docker".into(),
            enabled: true,
            checking: false,
            checked_at: None,
            next_check_at: None,
            available: None,
            error: None,
        };
        let mut etag = None;
        assert_eq!(
            apply(
                &mut view,
                &mut etag,
                Reply {
                    status: 200,
                    etag: Some("\"abc\"".into()),
                    retry_after: None,
                    body: release("v0.2.0")
                },
                "0.1.0",
                "x86_64"
            )
            .unwrap(),
            INTERVAL
        );
        let known = view.available.clone();
        assert_eq!(
            apply(
                &mut view,
                &mut etag,
                Reply {
                    status: 429,
                    etag: None,
                    retry_after: Some(1),
                    body: String::new()
                },
                "0.1.0",
                "x86_64"
            )
            .unwrap(),
            3600
        );
        assert_eq!(known, view.available);
        apply(
            &mut view,
            &mut etag,
            Reply {
                status: 304,
                etag: None,
                retry_after: None,
                body: String::new(),
            },
            "0.1.0",
            "x86_64",
        )
        .unwrap();
        assert_eq!(known, view.available);
        assert!(view.error.is_none());
        assert_eq!(etag.as_deref(), Some("\"abc\""));
        assert!(
            apply(
                &mut view,
                &mut etag,
                Reply {
                    status: 500,
                    etag: None,
                    retry_after: None,
                    body: String::new()
                },
                "0.1.0",
                "x86_64"
            )
            .is_err()
        );
        assert_eq!(known, view.available);
    }
}
