//! Stream network responses into SQLite; send only bounded pages to browsers.
use crate::model::{FileView, SearchScope, SearchView};
use anyhow::{Result, ensure};
use rusqlite::{Connection, params, params_from_iter, types::Value};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub fn format(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "flac" | "wav" | "wave" | "aiff" | "aif" | "alac" | "ape" | "wv" | "tta" | "tak"
        | "dsf" | "dff" => "lossless",
        "mp3" | "aac" | "opus" | "ogg" | "oga" | "wma" | "mp2" | "ac3" => "lossy",
        _ => "unknown",
    }
}
pub fn folder(path: &str) -> String {
    path.replace('/', "\\")
        .rsplit_once('\\')
        .map(|(p, _)| p.into())
        .unwrap_or_default()
}
pub fn migrate_keys(db: &Connection) -> Result<()> {
    let mut stmt =
        db.prepare("SELECT search_id,username,path FROM search_results WHERE search_key=''")?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    let tx = crate::store::write_transaction(db)?;
    for (id, user, path) in rows {
        tx.execute("UPDATE search_results SET folder=?1,format=?2,search_key=?3 WHERE search_id=?4 AND username=?5 AND path=?6",params![folder(&path),format(&path),format!("{user} {path}").to_lowercase(),id,user,path])?;
    }
    tx.commit()?;
    Ok(())
}
pub fn create(db: &Connection, view: &SearchView) -> Result<()> {
    db.execute("INSERT INTO searches(id,query,collecting) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET query=excluded.query,collecting=excluded.collecting",params![view.id,view.query,view.collecting])?;
    db.execute("INSERT INTO search_options VALUES(?1,?2,?3) ON CONFLICT(search_id) DO UPDATE SET scope=excluded.scope,wishlist_id=excluded.wishlist_id",params![view.id,serde_json::to_string(&view.scope)?,view.wishlist_id])?;
    Ok(())
}
pub fn append(
    db: &Connection,
    id: &str,
    files: impl IntoIterator<Item = FileView>,
    limit: usize,
) -> Result<(usize, bool)> {
    let tx = crate::store::write_transaction(db)?;
    let mut total = tx.query_row(
        "SELECT COUNT(*) FROM search_results WHERE search_id=?1",
        [id],
        |r| r.get::<_, i64>(0),
    )? as usize;
    let mut truncated = false;
    let mut affected = BTreeSet::new();
    {
        let mut insert=tx.prepare_cached("INSERT OR IGNORE INTO search_results(search_id,username,path,size,free_slots,speed,attributes,folder,format,search_key) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)")?;
        for f in files {
            if crate::validation::username(&f.username).is_err()
                || crate::validation::remote_path(&f.path).is_err()
                || f.size > 16 * 1024_u64.pow(4)
            {
                continue;
            }
            if total >= limit {
                truncated = true;
                break;
            }
            affected.insert(f.username.clone());
            total += insert.execute(params![
                id,
                f.username,
                f.path,
                i64::try_from(f.size)?,
                f.free_slots,
                f.speed,
                serde_json::to_string(&f.attributes)?,
                folder(&f.path),
                format(&f.path),
                format!("{} {}", f.username, f.path).to_lowercase()
            ])?;
        }
    }
    for user in affected {
        update_stats(&tx, id, Some(&user))?;
    }
    tx.commit()?;
    Ok((total, truncated))
}
pub fn preview(db: &Connection, id: &str) -> Result<Vec<FileView>> {
    let mut stmt=db.prepare("SELECT username,path,size,free_slots,speed,attributes FROM search_results WHERE search_id=?1 ORDER BY username,path LIMIT 100")?;
    let mut rows = stmt.query([id])?;
    let mut files = vec![];
    while let Some(r) = rows.next()? {
        files.push(FileView {
            username: r.get(0)?,
            path: r.get(1)?,
            size: r.get::<_, i64>(2)? as u64,
            free_slots: r.get(3)?,
            speed: r.get(4)?,
            attributes: serde_json::from_str(&r.get::<_, String>(5)?)?,
        });
    }
    Ok(files)
}
pub fn update_stats(db: &Connection, id: &str, user: Option<&str>) -> Result<()> {
    db.execute(
        "DELETE FROM search_user_stats WHERE search_id=?1 AND (?2 IS NULL OR username=?2)",
        params![id, user],
    )?;
    db.execute("INSERT INTO search_user_stats SELECT search_id,username,COUNT(*),COUNT(DISTINCT folder),MAX(speed),MAX(free_slots) FROM search_results WHERE search_id=?1 AND (?2 IS NULL OR username=?2) GROUP BY search_id,username",params![id,user])?;
    Ok(())
}
pub fn ensure_stats(db: &Connection) -> Result<()> {
    let mut stmt=db.prepare("SELECT id FROM searches WHERE EXISTS(SELECT 1 FROM search_results r WHERE r.search_id=searches.id) AND NOT EXISTS(SELECT 1 FROM search_user_stats s WHERE s.search_id=searches.id)")?;
    let ids = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    for id in ids {
        update_stats(db, &id, None)?;
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SearchQuery {
    pub filter: String,
    pub exclude: String,
    pub format: String,
    pub user_sort: String,
    pub file_sort: String,
    pub offset: usize,
    pub username: Option<String>,
    pub min_bitrate: Option<u32>,
    pub max_bitrate: Option<u32>,
    pub min_duration: Option<u32>,
    pub max_duration: Option<u32>,
    pub min_sample_rate: Option<u32>,
    pub max_sample_rate: Option<u32>,
    pub min_bit_depth: Option<u32>,
    pub max_bit_depth: Option<u32>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    pub vbr: Option<bool>,
    pub free_slots: bool,
}
impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            filter: String::new(),
            exclude: String::new(),
            format: "all".into(),
            user_sort: "matches".into(),
            file_sort: "name".into(),
            offset: 0,
            username: None,
            min_bitrate: None,
            max_bitrate: None,
            min_duration: None,
            max_duration: None,
            min_sample_rate: None,
            max_sample_rate: None,
            min_bit_depth: None,
            max_bit_depth: None,
            min_size: None,
            max_size: None,
            vbr: None,
            free_slots: false,
        }
    }
}
#[derive(Default, Serialize)]
pub struct UserStats {
    pub count: usize,
    pub folders: usize,
    pub speed: u32,
    pub slots: u8,
}
#[derive(Serialize)]
pub struct Page {
    pub files: Vec<FileView>,
    pub total: usize,
    pub offset: usize,
    pub users: BTreeMap<String, UserStats>,
}
pub fn page(db: &Connection, id: &str, q: &SearchQuery) -> Result<Page> {
    ensure!(
        q.filter.len() <= 256
            && q.exclude.len() <= 256
            && q.offset <= 1_000_000
            && matches!(q.format.as_str(), "all" | "lossless" | "lossy"),
        "Invalid search result query"
    );
    if let Some(u) = &q.username {
        crate::validation::username(u).map_err(anyhow::Error::msg)?;
    }
    match q.user_sort.as_str() {
        "matches" => (),
        "speed" => (),
        "name" => (),
        "available" => (),
        _ => anyhow::bail!("Invalid user order"),
    };
    let file_order = match q.file_sort.as_str() {
        "name" => "f.path COLLATE NOCASE,f.path",
        "name_desc" => "f.path COLLATE NOCASE DESC,f.path DESC",
        "smallest" => "f.size,f.path",
        "largest" => "f.size DESC,f.path",
        "bitrate" => "COALESCE(json_extract(f.attributes,'$.0'),0) DESC,f.path",
        "duration" => "COALESCE(json_extract(f.attributes,'$.1'),9223372036854),f.path",
        "sample_rate" => "COALESCE(json_extract(f.attributes,'$.4'),0) DESC,f.path",
        "bit_depth" => "COALESCE(json_extract(f.attributes,'$.5'),0) DESC,f.path",
        _ => anyhow::bail!("Invalid file order"),
    };
    let mut values = vec![
        Value::Text(id.into()),
        Value::Text(q.filter.to_lowercase()),
        Value::Text(q.format.clone()),
    ];
    let mut where_clause =
        "search_id=?1 AND instr(search_key,?2)>0 AND (?3='all' OR format=?3)".to_string();
    let mut criteria = false;
    for (code, min, max) in [
        ("0", q.min_bitrate, q.max_bitrate),
        ("1", q.min_duration, q.max_duration),
        ("4", q.min_sample_rate, q.max_sample_rate),
        ("5", q.min_bit_depth, q.max_bit_depth),
    ] {
        ensure!(
            min.zip(max).is_none_or(|(a, b)| a <= b),
            "Minimum attribute value must not exceed maximum"
        );
        for (operator, value) in [(">=", min), ("<=", max)] {
            if let Some(value) = value {
                criteria = true;
                values.push(Value::Integer(i64::from(value)));
                where_clause.push_str(&format!(
                    " AND json_extract(attributes,'$.{code}'){operator}?{}",
                    values.len()
                ));
            }
        }
    }
    ensure!(
        q.min_size.zip(q.max_size).is_none_or(|(a, b)| a <= b),
        "Minimum file size must not exceed maximum"
    );
    for (operator, value) in [(">=", q.min_size), ("<=", q.max_size)] {
        if let Some(value) = value {
            criteria = true;
            values.push(Value::Integer(
                i64::try_from(value).map_err(|_| anyhow::anyhow!("Invalid file size"))?,
            ));
            where_clause.push_str(&format!(" AND size{operator}?{}", values.len()));
        }
    }
    if let Some(value) = q.vbr {
        criteria = true;
        values.push(Value::Integer(i64::from(value)));
        where_clause.push_str(&format!(
            " AND json_extract(attributes,'$.2')=?{}",
            values.len()
        ));
    }
    if q.free_slots {
        criteria = true;
        where_clause.push_str(" AND free_slots>0");
    }
    for term in q.exclude.split_whitespace().take(32) {
        values.push(Value::Text(term.to_lowercase()));
        where_clause.push_str(&format!(" AND instr(search_key,?{})=0", values.len()));
    }
    if let Some(u) = &q.username {
        values.push(Value::Text(u.clone()));
        where_clause.push_str(&format!(" AND username=?{}", values.len()));
    }
    let tx = crate::store::write_transaction(db)?;
    let cached =
        !criteria && q.filter.is_empty() && q.exclude.trim().is_empty() && q.format == "all";
    let (stats_sql, stats_values) = if cached {
        let mut params = vec![Value::Text(id.into())];
        let condition = if let Some(u) = &q.username {
            params.push(Value::Text(u.clone()));
            " AND username=?2"
        } else {
            ""
        };
        (
            format!(
                "SELECT username,matches,folders,speed,slots FROM search_user_stats WHERE search_id=?1{condition}"
            ),
            params,
        )
    } else {
        (
            format!(
                "SELECT username,COUNT(*) matches,COUNT(DISTINCT folder) folders,MAX(speed) speed,MAX(free_slots) slots FROM search_results WHERE {where_clause} GROUP BY username"
            ),
            values.clone(),
        )
    };
    let stats_order = match q.user_sort.as_str() {
        "matches" => "matches DESC",
        "speed" => "speed DESC",
        "available" => "slots DESC",
        _ => "username COLLATE NOCASE",
    };
    let mut stmt = tx.prepare(&format!(
        "SELECT * FROM ({stats_sql}) ORDER BY {stats_order},username COLLATE NOCASE,username"
    ))?;
    let stats = stmt
        .query_map(params_from_iter(&stats_values), |r| {
            Ok((
                r.get::<_, String>(0)?,
                UserStats {
                    count: r.get::<_, i64>(1)? as usize,
                    folders: r.get::<_, i64>(2)? as usize,
                    speed: r.get(3)?,
                    slots: r.get(4)?,
                },
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    let total = stats.iter().map(|(_, s)| s.count).sum();
    let mut page = Page {
        files: vec![],
        total,
        offset: q.offset,
        users: BTreeMap::new(),
    };
    let mut skip = q.offset;
    for (username, stats) in stats {
        if skip >= stats.count {
            skip -= stats.count;
            continue;
        }
        let mut params = values.clone();
        params.push(Value::Text(username.clone()));
        let sql = format!(
            "SELECT f.username,f.path,f.size,f.free_slots,f.speed,f.attributes FROM search_results f WHERE {where_clause} AND f.username=?{} ORDER BY f.folder COLLATE NOCASE,f.folder,{file_order} LIMIT {} OFFSET {skip}",
            params.len(),
            100 - page.files.len()
        );
        let mut stmt = tx.prepare(&sql)?;
        let mut rows = stmt.query(params_from_iter(&params))?;
        while let Some(r) = rows.next()? {
            page.files.push(FileView {
                username: r.get(0)?,
                path: r.get(1)?,
                size: r.get::<_, i64>(2)? as u64,
                free_slots: r.get(3)?,
                speed: r.get(4)?,
                attributes: serde_json::from_str(&r.get::<_, String>(5)?)?,
            });
        }
        page.users.insert(username, stats);
        skip = 0;
        if page.files.len() == 100 {
            break;
        }
    }

    Ok(page)
}

pub fn scope(db: &Connection, id: &str) -> Result<SearchScope> {
    use rusqlite::OptionalExtension;
    Ok(db
        .query_row(
            "SELECT scope FROM search_options WHERE search_id=?1",
            [id],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_default())
}

pub fn wishlist_id(db: &Connection, id: &str) -> Result<Option<String>> {
    use rusqlite::OptionalExtension;
    Ok(db
        .query_row(
            "SELECT wishlist_id FROM search_options WHERE search_id=?1",
            [id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn attribute_filters_apply_before_counting_grouping_and_pagination_with_unknowns_excluded() {
        let store = crate::store::Store::open(std::path::Path::new(":memory:")).unwrap();
        let db = &store.connection;
        create(
            db,
            &SearchView {
                id: "quality".into(),
                query: "music".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let mut files = Vec::new();
        for i in 0..330 {
            files.push(FileView {
                username: if i < 160 { "alice" } else { "bob" }.into(),
                path: format!("Music/{i:04}.flac"),
                size: 1024,
                free_slots: 1,
                speed: 500,
                attributes: if i == 0 {
                    BTreeMap::new()
                } else {
                    [(1, 120), (4, 96000), (5, 24)].into()
                },
            });
        }
        files.push(FileView {
            username: "alice".into(),
            path: "lossy.mp3".into(),
            size: 10,
            free_slots: 0,
            speed: 20,
            attributes: [(0, 320), (1, 30), (2, 0)].into(),
        });
        append(db, "quality", files, 1000).unwrap();
        let mut q = SearchQuery {
            min_sample_rate: Some(48000),
            min_bit_depth: Some(24),
            min_duration: Some(100),
            max_duration: Some(200),
            min_size: Some(100),
            max_size: Some(2000),
            free_slots: true,
            ..Default::default()
        };
        let p = page(db, "quality", &q).unwrap();
        assert_eq!(p.total, 329);
        assert_eq!(p.files.len(), 100);
        assert_eq!(p.users["bob"].count, 170);
        q.offset = 300;
        assert_eq!(page(db, "quality", &q).unwrap().files.len(), 29);
        q = SearchQuery {
            min_bitrate: Some(320),
            max_bitrate: Some(320),
            vbr: Some(false),
            ..Default::default()
        };
        let p = page(db, "quality", &q).unwrap();
        assert_eq!(p.total, 1);
        assert_eq!(p.files[0].path, "lossy.mp3");
        q.free_slots = true;
        assert_eq!(page(db, "quality", &q).unwrap().total, 0);
        q.min_bitrate = Some(500);
        assert!(page(db, "quality", &q).is_err());
        q = SearchQuery {
            max_bit_depth: Some(16),
            ..Default::default()
        };
        assert_eq!(
            page(db, "quality", &q).unwrap().total,
            0,
            "Unknown is not zero bit depth"
        );
        q = SearchQuery {
            max_size: Some(u64::MAX),
            ..Default::default()
        };
        assert!(page(db, "quality", &q).is_err());
    }
    #[test]
    fn empty_files_remain_searchable_browseable_and_downloadable() {
        let store = crate::store::Store::open(std::path::Path::new(":memory:")).unwrap();
        let db = &store.connection;
        create(
            db,
            &crate::model::SearchView {
                id: "empty".into(),
                query: "empty".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let empty = FileView {
            username: "peer".into(),
            path: "Music/empty.txt".into(),
            size: 0,
            free_slots: 1,
            speed: 0,
            attributes: Default::default(),
        };
        let too_large = FileView {
            size: 16 * 1024_u64.pow(4) + 1,
            ..empty.clone()
        };
        assert_eq!(
            append(db, "empty", [empty.clone(), too_large.clone()], 100)
                .unwrap()
                .0,
            1
        );
        assert_eq!(
            page(db, "empty", &SearchQuery::default()).unwrap().files[0].size,
            0
        );
        assert_eq!(
            crate::browse::save(db, "peer", [empty.clone(), too_large])
                .unwrap()
                .total,
            1
        );
        assert_eq!(
            crate::browse::page(
                db,
                &crate::browse::BrowseQuery {
                    username: "peer".into(),
                    ..Default::default()
                }
            )
            .unwrap()
            .files[0]
                .size,
            0
        );
        assert!(
            crate::model::Command::Download {
                username: empty.username,
                path: empty.path,
                size: 0
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn results_beyond_2500_are_durable_and_filtered_sorted_as_a_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let store = crate::store::Store::open(&path).unwrap();
        create(
            &store.connection,
            &SearchView {
                id: "one".into(),
                query: "music".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let files = (0..4000).map(|i| FileView {
            username: if i < 3000 { "alice" } else { "bob" }.into(),
            path: format!(
                "Music\\Album\\{:04}.{}",
                i,
                if i % 2 == 0 { "flac" } else { "mp3" }
            ),
            size: 100,
            free_slots: 1,
            speed: if i < 3000 { 10 } else { 100 },
            attributes: BTreeMap::new(),
        });
        assert_eq!(
            append(&store.connection, "one", files, 100_000).unwrap(),
            (4000, false)
        );
        let mut q = SearchQuery {
            offset: 3900,
            ..Default::default()
        };
        let p = page(&store.connection, "one", &q).unwrap();
        assert_eq!((p.total, p.files.len()), (4000, 100));
        assert_eq!(p.users["bob"].count, 1000);
        q.offset = 0;
        q.user_sort = "speed".into();
        assert_eq!(
            page(&store.connection, "one", &q).unwrap().files[0].username,
            "bob"
        );
        q.format = "lossless".into();
        q.exclude = "3900".into();
        assert_eq!(page(&store.connection, "one", &q).unwrap().total, 1999);
        drop(store);
        let store = crate::store::Store::open(&path).unwrap();
        assert_eq!(
            page(&store.connection, "one", &SearchQuery::default())
                .unwrap()
                .total,
            4000
        );
    }
}
