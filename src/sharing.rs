use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub mode: String,
    pub min_files: u32,
    pub min_folders: u32,
    pub exempt_buddies: bool,
    pub exceptions: Vec<String>,
    pub allow_unknown: bool,
    pub check_seconds: u64,
    pub cache_seconds: u64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: "sharing".into(),
            min_files: 1,
            min_folders: 0,
            exempt_buddies: true,
            exceptions: vec![],
            allow_unknown: false,
            check_seconds: 30,
            cache_seconds: 300,
        }
    }
}
impl Settings {
    pub fn validate(&self) -> std::result::Result<(), String> {
        if !matches!(self.mode.as_str(), "off" | "sharing" | "buddies")
            || self.min_files > 100_000_000
            || self.min_folders > 10_000_000
            || !(5..=120).contains(&self.check_seconds)
            || !(30..=3600).contains(&self.cache_seconds)
            || self.exceptions.len() > 128
        {
            return Err("Choose a valid sharing policy, a 5–120 second check timeout, and a 30–3600 second cache duration.".into());
        }
        for u in &self.exceptions {
            crate::validation::username(u)?;
        }
        Ok(())
    }
    pub fn protocol(&self, buddies: &[String]) -> soulseek_rs::UploadPolicy {
        soulseek_rs::UploadPolicy {
            mode: match self.mode.as_str() {
                "off" => 0,
                "buddies" => 2,
                _ => 1,
            },
            min_files: self.min_files,
            min_folders: self.min_folders,
            buddies: buddies.iter().map(|u| u.to_ascii_lowercase()).collect(),
            exceptions: self
                .exceptions
                .iter()
                .map(|u| u.to_ascii_lowercase())
                .collect(),
            exempt_buddies: self.exempt_buddies,
            allow_unknown: self.allow_unknown,
            check_seconds: self.check_seconds,
            cache_seconds: self.cache_seconds,
        }
    }
}
pub fn load(db: &Connection) -> Result<Settings> {
    db.query_row("SELECT json FROM sharing_policy WHERE id=1", [], |r| {
        r.get::<_, String>(0)
    })
    .optional()?
    .map(|s| serde_json::from_str(&s).map_err(Into::into))
    .unwrap_or_else(|| Ok(Settings::default()))
}
pub fn save(db: &Connection, settings: &Settings) -> Result<()> {
    settings.validate().map_err(anyhow::Error::msg)?;
    db.execute(
        "INSERT INTO sharing_policy VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",
        [serde_json::to_string(settings)?],
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sharing_required_is_default_and_choices_persist() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let store = crate::store::Store::open(&path).unwrap();
        let mut s = load(&store.connection).unwrap();
        assert_eq!((s.mode.as_str(), s.min_files), ("sharing", 1));
        s.mode = "buddies".into();
        s.exceptions.push("Friend".into());
        save(&store.connection, &s).unwrap();
        drop(store);
        let store = crate::store::Store::open(&path).unwrap();
        assert_eq!(load(&store.connection).unwrap().mode, "buddies");
    }
}
