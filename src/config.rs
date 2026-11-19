use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    #[serde(deserialize_with = "deserialize_bind")]
    pub bind: SocketAddr,
    pub allow_remote: bool,
    pub data_dir: PathBuf,
    pub download_dir: Option<PathBuf>,
    pub location_roots: Vec<PathBuf>,
    pub published_transfer_port: Option<u16>,
    pub soulseek: Soulseek,
    pub limits: Limits,
}
fn parse_bind(value: &str) -> Result<SocketAddr> {
    if value == "all" {
        return Ok("0.0.0.0:8484".parse()?);
    }
    if let Some(port) = value.strip_prefix("all:") {
        return Ok(format!("0.0.0.0:{port}").parse()?);
    }
    Ok(value.parse()?)
}
fn deserialize_bind<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<SocketAddr, D::Error> {
    let value = String::deserialize(deserializer)?;
    parse_bind(&value).map_err(serde::de::Error::custom)
}

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Soulseek {
    pub server: String,
    pub port: u16,
    pub listen_port: u16,
    pub shares: Vec<PathBuf>,
    pub auto_connect: bool,
}

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub search_results: usize,
    pub browse_files: usize,
    pub peers: usize,
    pub upload_slots: usize,
    pub download_bytes_per_second: u64,
    pub searches: usize,
    pub transfers: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8484".parse().unwrap(),
            allow_remote: false,
            data_dir: "data".into(),
            download_dir: None,
            location_roots: vec![],
            published_transfer_port: None,
            soulseek: Soulseek::default(),
            limits: Limits::default(),
        }
    }
}
impl Default for Soulseek {
    fn default() -> Self {
        Self {
            server: "server.slsknet.org".into(),
            port: 2242,
            listen_port: 2234,
            shares: vec![],
            auto_connect: true,
        }
    }
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            search_results: 100_000,
            browse_files: 1_000_000,
            peers: 64,
            upload_slots: 2,
            download_bytes_per_second: 0,
            searches: 8,
            transfers: 128,
        }
    }
}
impl Config {
    pub fn load_optional(path: Option<&Path>) -> Result<Self> {
        if let Some(path) = path {
            return Self::load(path);
        }
        let default = Path::new("rustysoul.toml");
        if default
            .try_exists()
            .context("Cannot check rustysoul.toml")?
        {
            return Self::load(default);
        }
        Self::finish(Self::default())
    }
    pub fn load(path: &Path) -> Result<Self> {
        let config: Self = toml::from_str(
            &std::fs::read_to_string(path)
                .with_context(|| format!("Cannot read {}", path.display()))?,
        )
        .context("Invalid configuration")?;
        Self::finish(config)
    }
    fn finish(mut config: Self) -> Result<Self> {
        config.apply_environment(|key| std::env::var(key).ok())?;
        config.validate()?;
        Ok(config)
    }
    fn apply_environment(&mut self, get: impl Fn(&str) -> Option<String>) -> Result<()> {
        self.apply_deployment_env(
            get("RUSTYSOUL_TRANSFER_PORT").as_deref(),
            get("RUSTYSOUL_PUBLISHED_TRANSFER_PORT").as_deref(),
            get("RUSTYSOUL_BIND").as_deref(),
        )?;
        macro_rules! scalar {
            ($key:literal, $field:expr) => {
                if let Some(value) = get($key) {
                    $field = value.parse().with_context(|| concat!("Invalid ", $key))?;
                }
            };
        }
        scalar!("RUSTYSOUL_ALLOW_REMOTE", self.allow_remote);
        scalar!("RUSTYSOUL_SOULSEEK_PORT", self.soulseek.port);
        scalar!("RUSTYSOUL_AUTO_CONNECT", self.soulseek.auto_connect);
        scalar!("RUSTYSOUL_MAX_PEERS", self.limits.peers);
        scalar!("RUSTYSOUL_UPLOAD_SLOTS", self.limits.upload_slots);
        scalar!(
            "RUSTYSOUL_DOWNLOAD_BYTES_PER_SECOND",
            self.limits.download_bytes_per_second
        );
        scalar!("RUSTYSOUL_MAX_SEARCH_RESULTS", self.limits.search_results);
        scalar!("RUSTYSOUL_MAX_BROWSE_FILES", self.limits.browse_files);
        scalar!("RUSTYSOUL_MAX_SEARCHES", self.limits.searches);
        scalar!("RUSTYSOUL_MAX_TRANSFERS", self.limits.transfers);
        if let Some(value) = get("RUSTYSOUL_SOULSEEK_SERVER") {
            self.soulseek.server = value;
        }
        if let Some(value) = get("RUSTYSOUL_DATA_DIR") {
            ensure!(
                Path::new(&value).is_absolute(),
                "RUSTYSOUL_DATA_DIR must be an absolute path"
            );
            self.data_dir = value.into();
        }
        if let Some(value) = get("RUSTYSOUL_DOWNLOAD_DIR") {
            self.download_dir = Some(value.into());
        }
        if let Some(value) = get("RUSTYSOUL_LIBRARY_DIRS") {
            self.soulseek.shares = environment_paths("RUSTYSOUL_LIBRARY_DIRS", &value)?;
        }
        if let Some(value) = get("RUSTYSOUL_LOCATION_ROOTS") {
            self.location_roots = environment_paths("RUSTYSOUL_LOCATION_ROOTS", &value)?;
        }
        Ok(())
    }
    fn apply_deployment_env(
        &mut self,
        port: Option<&str>,
        published: Option<&str>,
        bind: Option<&str>,
    ) -> Result<()> {
        if let Some(p) = port.filter(|p| !p.is_empty()) {
            self.soulseek.listen_port = p
                .parse()
                .context("RUSTYSOUL_TRANSFER_PORT must be a port")?;
        }
        if let Some(p) = published.filter(|p| !p.is_empty()) {
            let p: u16 = p
                .parse()
                .context("RUSTYSOUL_PUBLISHED_TRANSFER_PORT must be a port")?;
            self.published_transfer_port = Some(p);
            self.soulseek.listen_port = p;
        }
        if let Some(bind) = bind.filter(|b| !b.is_empty()) {
            self.bind = parse_bind(bind).context("Invalid RUSTYSOUL_BIND")?;
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.soulseek.listen_port >= 1024,
            "Transfer port must be 1024..65535"
        );
        if let Some(port) = self.published_transfer_port {
            ensure!(
                port >= 1024 && port != self.bind.port(),
                "Published transfer port must be 1024..65535 and different from HTTP"
            );
        }
        for root in &self.location_roots {
            ensure!(root.is_absolute(), "location_roots must be absolute paths");
        }
        if let Some(path) = &self.download_dir {
            ensure!(path.is_absolute(), "download_dir must be an absolute path");
        }
        ensure!(
            self.bind.ip().is_loopback() || self.allow_remote,
            "Remote HTTP binding requires allow_remote = true and a trusted TLS reverse proxy"
        );
        ensure!(
            !self.soulseek.server.trim().is_empty(),
            "Soulseek server cannot be empty"
        );
        ensure!(
            self.bind.port() > 0 && self.soulseek.port > 0 && self.soulseek.listen_port > 0,
            "Ports must be nonzero"
        );
        ensure!(
            (1..=256).contains(&self.limits.peers),
            "peers must be 1..256"
        );
        ensure!(
            (1..=16).contains(&self.limits.upload_slots),
            "upload_slots must be 1..16"
        );
        ensure!(
            (1..=16).contains(&self.limits.searches),
            "searches must be 1..16"
        );
        ensure!(
            (1..=1024).contains(&self.limits.transfers),
            "transfers must be 1..1024"
        );
        ensure!(
            (100..=1_000_000).contains(&self.limits.search_results)
                && (100..=2_000_000).contains(&self.limits.browse_files),
            "Search results must be 100..1000000; browse files must be 100..2000000"
        );
        ensure!(
            self.soulseek.shares.len() <= 32,
            "At most 32 share roots are supported"
        );
        for share in &self.soulseek.shares {
            ensure!(
                share.is_absolute() && share.is_dir(),
                "Share roots must be existing absolute directories"
            );
            ensure!(
                share.parent().is_some(),
                "Sharing the filesystem root is refused"
            );
        }
        Ok(())
    }
}

fn environment_paths(name: &str, value: &str) -> Result<Vec<PathBuf>> {
    if value.trim_start().starts_with('[') {
        serde_json::from_str(value)
            .with_context(|| format!("{name} must be a path or a JSON array of paths"))
    } else {
        ensure!(
            !value.is_empty(),
            "{name} must be a path or a JSON array of paths"
        );
        Ok(vec![value.into()])
    }
}

// Deliberately no Debug implementation: these values must never enter logs or API responses.
pub struct Secrets {
    pub api_token: String,
    pub username: String,
    pub password: String,
}
impl Secrets {
    pub fn from_env() -> Result<Self> {
        Self::new(
            // Internal compatibility value; production API access always uses Auth sessions.
            format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4()),
            std::env::var("SOULSEEK_USERNAME").unwrap_or_default(),
            std::env::var("SOULSEEK_PASSWORD").unwrap_or_default(),
        )
    }
    pub fn new(api_token: String, username: String, password: String) -> Result<Self> {
        ensure!(
            api_token.len() >= 32
                && api_token.len() <= 256
                && api_token.bytes().all(|b| b.is_ascii_graphic()),
            "RUSTYSOUL_API_TOKEN must be 32..256 printable ASCII characters; generate with openssl rand -hex 32"
        );
        if !username.is_empty() {
            crate::validation::username(&username).map_err(anyhow::Error::msg)?;
            ensure!(
                !password.is_empty() && password.len() <= 1024,
                "SOULSEEK_PASSWORD must contain 1..1024 bytes"
            );
        }
        Ok(Self {
            api_token,
            username,
            password,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn environment_only_configuration_covers_paths_network_limits_and_validates_errors() {
        let dir = tempfile::tempdir().unwrap();
        let env = std::collections::HashMap::from([
            ("RUSTYSOUL_BIND", "all:8585".to_string()),
            ("RUSTYSOUL_ALLOW_REMOTE", "true".into()),
            (
                "RUSTYSOUL_DATA_DIR",
                dir.path().join("state").to_string_lossy().into_owned(),
            ),
            (
                "RUSTYSOUL_DOWNLOAD_DIR",
                dir.path().join("downloads").to_string_lossy().into_owned(),
            ),
            (
                "RUSTYSOUL_LIBRARY_DIRS",
                serde_json::to_string(&vec![dir.path()]).unwrap(),
            ),
            (
                "RUSTYSOUL_LOCATION_ROOTS",
                serde_json::to_string(&vec![dir.path()]).unwrap(),
            ),
            ("RUSTYSOUL_SOULSEEK_SERVER", "127.0.0.1".into()),
            ("RUSTYSOUL_SOULSEEK_PORT", "12345".into()),
            ("RUSTYSOUL_AUTO_CONNECT", "false".into()),
            ("RUSTYSOUL_UPLOAD_SLOTS", "3".into()),
            ("RUSTYSOUL_MAX_PEERS", "12".into()),
            ("RUSTYSOUL_MAX_SEARCH_RESULTS", "1234".into()),
            ("RUSTYSOUL_MAX_BROWSE_FILES", "2345".into()),
            ("RUSTYSOUL_MAX_TRANSFERS", "77".into()),
            ("RUSTYSOUL_MAX_SEARCHES", "4".into()),
            ("RUSTYSOUL_DOWNLOAD_BYTES_PER_SECOND", "999".into()),
        ]);
        let mut c = Config::default();
        c.apply_environment(|k| env.get(k).cloned()).unwrap();
        c.validate().unwrap();
        assert_eq!(c.bind, "0.0.0.0:8585".parse().unwrap());
        assert_eq!(c.data_dir, dir.path().join("state"));
        assert_eq!(c.download_dir, Some(dir.path().join("downloads")));
        assert_eq!(c.soulseek.shares, vec![dir.path()]);
        assert_eq!(c.location_roots, vec![dir.path()]);
        assert!(!c.soulseek.auto_connect);
        assert_eq!(
            (c.soulseek.server.as_str(), c.soulseek.port),
            ("127.0.0.1", 12345)
        );
        assert_eq!(
            (
                c.limits.peers,
                c.limits.upload_slots,
                c.limits.transfers,
                c.limits.searches
            ),
            (12, 3, 77, 4)
        );
        assert_eq!(
            (
                c.limits.search_results,
                c.limits.browse_files,
                c.limits.download_bytes_per_second
            ),
            (1234, 2345, 999)
        );
        for (key, value) in [
            ("RUSTYSOUL_ALLOW_REMOTE", "yes"),
            ("RUSTYSOUL_LIBRARY_DIRS", "[broken"),
            ("RUSTYSOUL_DATA_DIR", "relative"),
            ("RUSTYSOUL_MAX_PEERS", "unlimited"),
        ] {
            assert!(
                Config::default()
                    .apply_environment(|k| if k == key { Some(value.into()) } else { None })
                    .is_err()
            );
        }
        assert!(Config::load_optional(Some(&dir.path().join("missing.toml"))).is_err());
    }
    #[test]
    fn supports_specific_adapter_all_aliases_and_install_time_ports() {
        for value in ["all", "all:8484", "0.0.0.0:8484", "192.168.1.1:8484"] {
            let c: Config = toml::from_str(&format!("bind='{value}'\nallow_remote=true")).unwrap();
            c.validate().unwrap();
            assert_eq!(c.bind.port(), 8484);
        }
        let mut c = Config::default();
        c.apply_deployment_env(Some("3344"), Some("4455"), Some("127.0.0.1:8485"))
            .unwrap();
        c.validate().unwrap();
        assert_eq!(c.soulseek.listen_port, 4455);
        assert_eq!(c.published_transfer_port, Some(4455));
        assert!(c.apply_deployment_env(Some("invalid"), None, None).is_err());
    }
    #[test]
    fn defaults_are_private_and_bounded() {
        Config::default().validate().unwrap();
    }
    #[test]
    fn refuses_public_bind_without_explicit_opt_in() {
        let c = Config {
            bind: "0.0.0.0:8484".parse().unwrap(),
            ..Default::default()
        };
        assert!(c.validate().is_err());
    }
    #[test]
    fn rejects_unknown_keys_and_unbounded_limits() {
        assert!(toml::from_str::<Config>("bnd = '127.0.0.1:8484'").is_err());
        for count in [0, 257, usize::MAX] {
            let mut c = Config::default();
            c.limits.peers = count;
            assert!(c.validate().is_err());
        }
    }
    #[test]
    fn rejects_weak_tokens_and_partial_credentials() {
        assert!(Secrets::new("default".into(), "".into(), "".into()).is_err());
        assert!(Secrets::new("a".repeat(32), "person".into(), "".into()).is_err());
        assert!(Secrets::new("a".repeat(32), "".into(), "".into()).is_ok());
    }
}
