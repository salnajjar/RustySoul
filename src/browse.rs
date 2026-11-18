//! SQLite-backed peer listings. SSE carries a small preview; HTTP pages the full index.
use crate::model::{BrowseView, FileView};
use anyhow::{Result, ensure};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub fn save(
    db: &Connection,
    username: &str,
    files: impl IntoIterator<Item = FileView>,
) -> Result<BrowseView> {
    save_with_limit(db, username, files, 1_000_000)
}
pub fn save_with_limit(
    db: &Connection,
    username: &str,
    files: impl IntoIterator<Item = FileView>,
    limit: usize,
) -> Result<BrowseView> {
    let tx = crate::store::write_transaction(db)?;
    tx.execute("DELETE FROM browse_files WHERE username=?1", [username])?;
    let mut view = BrowseView::default();
    let mut folders = BTreeMap::<String, usize>::new();
    {
        let mut insert =
            tx.prepare_cached("INSERT OR IGNORE INTO browse_files(username,path,folder,size,attributes,search_key) VALUES(?1,?2,?3,?4,?5,?6)")?;
        for file in files {
            if crate::validation::remote_path(&file.path).is_err()
                || file.size > 16 * 1024_u64.pow(4)
            {
                continue;
            }
            if view.total == limit {
                view.truncated = true;
                break;
            }
            let folder = crate::search::folder(&file.path);
            if insert.execute(params![
                username,
                file.path,
                folder,
                i64::try_from(file.size)?,
                serde_json::to_string(&file.attributes)?,
                file.path.to_lowercase()
            ])? > 0
            {
                *folders.entry(folder).or_default() += 1;
                view.total += 1;
                if view.files.len() < 100 {
                    view.files.push(file);
                }
            }
        }
    }
    populate_tree(&tx, username, folders)?;
    tx.commit()?;
    view.revision = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    Ok(view)
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BrowseQuery {
    pub username: String,
    pub folder: Option<String>,
    pub filter: String,
    pub offset: usize,
    pub folders: bool,
    pub recursive: bool,
    pub order: String,
}
impl Default for BrowseQuery {
    fn default() -> Self {
        Self {
            username: String::new(),
            folder: None,
            filter: String::new(),
            offset: 0,
            folders: false,
            recursive: false,
            order: "name".into(),
        }
    }
}
#[derive(Serialize)]
pub struct Folder {
    pub path: String,
    pub files: usize,
}
#[derive(Serialize)]
pub struct Page {
    pub files: Vec<FileView>,
    pub folders: Vec<Folder>,
    pub total: usize,
    pub offset: usize,
}
pub fn migrate_keys(db: &Connection) -> Result<()> {
    let mut stmt = db.prepare("SELECT username,path FROM browse_files WHERE search_key=''")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    if rows.is_empty() {
        return Ok(());
    }
    let tx = crate::store::write_transaction(db)?;
    for (u, p) in rows {
        tx.execute(
            "UPDATE browse_files SET search_key=?1 WHERE username=?2 AND path=?3",
            params![p.to_lowercase(), u, p],
        )?;
    }
    tx.commit()?;
    Ok(())
}
fn populate_tree(db: &Connection, username: &str, counts: BTreeMap<String, usize>) -> Result<()> {
    let mut tree = BTreeMap::<String, (usize, usize)>::new();
    for (folder, count) in counts {
        let mut path = String::new();
        for name in folder.split('\\').filter(|p| !p.is_empty()) {
            if !path.is_empty() {
                path.push('\\');
            }
            path.push_str(name);
            tree.entry(path.clone()).or_default().0 += count;
        }
        if let Some(node) = tree.get_mut(&folder) {
            node.1 = count;
        }
    }
    db.execute("DELETE FROM browse_folders WHERE username=?1", [username])?;
    let mut insert =
        db.prepare_cached("INSERT INTO browse_folders(username,path,parent,name,files,direct_files,search_key,subfolders,direct_folders) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)")?;
    let mut child_counts = BTreeMap::<String, (usize, usize)>::new();
    for path in tree.keys() {
        let mut child = path.as_str();
        let mut immediate = true;
        while let Some((parent, _)) = child.rsplit_once('\\') {
            let count = child_counts.entry(parent.into()).or_default();
            count.0 += 1;
            if immediate {
                count.1 += 1;
            }
            immediate = false;
            child = parent;
        }
    }
    for (path, (files, direct)) in tree {
        let (parent, name) = path.rsplit_once('\\').unwrap_or(("", &path));
        insert.execute(params![
            username,
            path,
            parent,
            name,
            files as i64,
            direct as i64,
            path.to_lowercase(),
            child_counts.get(&path).map_or(0, |c| c.0) as i64,
            child_counts.get(&path).map_or(0, |c| c.1) as i64
        ])?;
    }
    Ok(())
}
pub fn ensure_tree(db: &Connection, username: &str) -> Result<()> {
    if db.query_row(
        "SELECT EXISTS(SELECT 1 FROM browse_folders WHERE username=?1)",
        [username],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(());
    }
    let mut stmt =
        db.prepare("SELECT folder,COUNT(*) FROM browse_files WHERE username=?1 GROUP BY folder")?;
    let rows = stmt
        .query_map([username], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize))
        })?
        .collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
    drop(stmt);
    let tx = crate::store::write_transaction(db)?;
    populate_tree(&tx, username, rows)?;
    tx.commit()?;
    Ok(())
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TreeQuery {
    pub username: String,
    pub parent: String,
    pub filter: String,
    pub offset: usize,
    pub all: bool,
}
#[derive(Serialize)]
pub struct TreeFolder {
    pub path: String,
    pub parent: String,
    pub name: String,
    pub files: usize,
    pub direct_files: usize,
    pub has_children: bool,
    pub subfolders: usize,
    pub direct_folders: usize,
}
#[derive(Serialize)]
pub struct TreePage {
    pub folders: Vec<TreeFolder>,
    pub total: usize,
    pub offset: usize,
}
pub fn tree(db: &Connection, q: &TreeQuery) -> Result<TreePage> {
    crate::validation::username(&q.username).map_err(anyhow::Error::msg)?;
    ensure!(
        q.parent.len() <= 4096 && q.filter.len() <= 256 && q.offset <= 2_000_000,
        "Invalid folder tree query"
    );
    if q.all {
        ensure!(
            q.parent.is_empty() && q.filter.is_empty() && q.offset == 0,
            "A complete folder tree cannot be paged or filtered"
        );
        let mut stmt = db.prepare("SELECT path,name,files,direct_files,direct_folders>0,subfolders,direct_folders,parent FROM browse_folders WHERE username=?1 ORDER BY name COLLATE NOCASE,name,path")?;
        let folders = stmt
            .query_map([&q.username], read_tree_folder)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        return Ok(TreePage {
            total: folders.len(),
            folders,
            offset: 0,
        });
    }
    let condition = "f.username=?1 AND f.parent=?2 AND (?3='' OR instr(f.search_key,?3)>0 OR EXISTS(SELECT 1 FROM browse_folders d WHERE d.username=f.username AND substr(d.path,1,length(f.path)+1)=f.path||char(92) AND instr(d.search_key,?3)>0))";
    let filter = q.filter.to_lowercase();
    let total = db.query_row(
        &format!("SELECT COUNT(*) FROM browse_folders f WHERE {condition}"),
        params![q.username, q.parent, filter],
        |r| r.get::<_, i64>(0),
    )? as usize;
    let mut stmt=db.prepare(&format!("SELECT f.path,f.name,f.files,f.direct_files,f.direct_folders>0,f.subfolders,f.direct_folders,f.parent FROM browse_folders f WHERE {condition} ORDER BY f.name COLLATE NOCASE,f.name LIMIT 100 OFFSET ?4"))?;
    let folders = stmt
        .query_map(
            params![q.username, q.parent, filter, q.offset as i64],
            read_tree_folder,
        )?
        .collect::<rusqlite::Result<_>>()?;
    Ok(TreePage {
        folders,
        total,
        offset: q.offset,
    })
}
fn read_tree_folder(r: &rusqlite::Row<'_>) -> rusqlite::Result<TreeFolder> {
    Ok(TreeFolder {
        path: r.get(0)?,
        parent: r.get(7)?,
        name: r.get(1)?,
        files: r.get::<_, i64>(2)? as usize,
        direct_files: r.get::<_, i64>(3)? as usize,
        has_children: r.get(4)?,
        subfolders: r.get::<_, i64>(5)? as usize,
        direct_folders: r.get::<_, i64>(6)? as usize,
    })
}
pub fn page(db: &Connection, query: &BrowseQuery) -> Result<Page> {
    crate::validation::username(&query.username).map_err(anyhow::Error::msg)?;
    ensure!(
        query.filter.len() <= 256
            && query.offset <= 2_000_000
            && query.folder.as_ref().is_none_or(|p| p.len() <= 4096),
        "Invalid browse query"
    );
    let mut page = Page {
        files: vec![],
        folders: vec![],
        total: 0,
        offset: query.offset,
    };
    if query.folders {
        page.total = db.query_row("SELECT COUNT(DISTINCT folder) FROM browse_files WHERE username=?1 AND instr(lower(folder),lower(?2))>0", params![query.username,query.filter], |r|r.get::<_,u32>(0).map(|v|v as usize))?;
        let mut stmt = db.prepare("SELECT folder,COUNT(*) FROM browse_files WHERE username=?1 AND instr(lower(folder),lower(?2))>0 GROUP BY folder ORDER BY folder COLLATE NOCASE,folder LIMIT 100 OFFSET ?3")?;
        page.folders = stmt
            .query_map(
                params![query.username, query.filter, query.offset as i64],
                |r| {
                    Ok(Folder {
                        path: r.get(0)?,
                        files: r.get::<_, u32>(1)? as usize,
                    })
                },
            )?
            .collect::<rusqlite::Result<_>>()?;
    } else {
        let order = match query.order.as_str() {
            "name" => "path COLLATE NOCASE,path",
            "name_desc" => "path COLLATE NOCASE DESC,path DESC",
            "smallest" => "size,path",
            "largest" => "size DESC,path",
            "bitrate" => "COALESCE(json_extract(attributes,'$.0'),0) DESC,path",
            "duration" => "COALESCE(json_extract(attributes,'$.1'),9223372036854),path",
            _ => anyhow::bail!("Invalid file order"),
        };
        let condition = "username=?1 AND (?2 IS NULL OR folder=?2 OR (?4 AND substr(folder,1,length(?2)+1)=?2||char(92))) AND instr(search_key,?3)>0";
        page.total = db.query_row(
            &format!("SELECT COUNT(*) FROM browse_files WHERE {condition}"),
            params![
                query.username,
                query.folder,
                query.filter.to_lowercase(),
                query.recursive
            ],
            |r| r.get::<_, i64>(0),
        )? as usize;
        let mut stmt=db.prepare(&format!("SELECT path,size,attributes FROM browse_files WHERE {condition} ORDER BY {order} LIMIT 100 OFFSET ?5"))?;
        let rows = stmt.query_map(
            params![
                query.username,
                query.folder,
                query.filter.to_lowercase(),
                query.recursive,
                query.offset as i64
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)? as u64,
                    r.get::<_, String>(2)?,
                ))
            },
        )?;
        for row in rows {
            let (path, size, attributes) = row?;
            page.files.push(FileView {
                username: query.username.clone(),
                path,
                size,
                attributes: serde_json::from_str(&attributes)?,
                free_slots: 0,
                speed: 0,
            });
        }
    }
    Ok(page)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tree_exposes_virtual_parents_and_paged_siblings_and_recursive_file_selection() {
        let store = crate::store::Store::open(std::path::Path::new(":memory:")).unwrap();
        let files = (0..2801).map(|i| FileView {
            username: "alice".into(),
            path: format!("Music\\Artist {i}\\Album\\song.flac"),
            size: 100,
            free_slots: 0,
            speed: 0,
            attributes: Default::default(),
        });
        save(&store.connection, "alice", files).unwrap();
        let root = tree(
            &store.connection,
            &TreeQuery {
                username: "alice".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(root.folders.len(), 1);
        assert_eq!(root.folders[0].path, "Music");
        assert_eq!(root.folders[0].files, 2801);
        assert!(root.folders[0].has_children);
        assert_eq!(root.folders[0].subfolders, 5602);
        assert_eq!(root.folders[0].direct_folders, 2801);
        let complete = tree(
            &store.connection,
            &TreeQuery {
                username: "alice".into(),
                all: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            (complete.total, complete.folders.len(), complete.offset),
            (5603, 5603, 0)
        );
        assert!(
            complete
                .folders
                .iter()
                .any(|f| f.path == "Music" && f.parent.is_empty())
        );
        assert!(
            complete
                .folders
                .iter()
                .any(|f| f.path == "Music\\Artist 2800\\Album" && f.parent == "Music\\Artist 2800")
        );
        assert_eq!(
            tree(
                &store.connection,
                &TreeQuery {
                    username: "bob".into(),
                    all: true,
                    ..Default::default()
                }
            )
            .unwrap()
            .total,
            0
        );
        for query in [
            TreeQuery {
                parent: "Music".into(),
                ..Default::default()
            },
            TreeQuery {
                filter: "Artist".into(),
                ..Default::default()
            },
            TreeQuery {
                offset: 1,
                ..Default::default()
            },
        ] {
            assert!(
                tree(
                    &store.connection,
                    &TreeQuery {
                        username: "alice".into(),
                        all: true,
                        ..query
                    }
                )
                .is_err()
            );
        }
        let siblings = tree(
            &store.connection,
            &TreeQuery {
                username: "alice".into(),
                parent: "Music".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!((siblings.total, siblings.folders.len()), (2801, 100));
        let tail = tree(
            &store.connection,
            &TreeQuery {
                username: "alice".into(),
                parent: "Music".into(),
                offset: 2800,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(tail.total, 2801);
        assert_eq!(tail.folders.len(), 1);
        let filtered = tree(
            &store.connection,
            &TreeQuery {
                username: "alice".into(),
                filter: "Artist 2800".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(filtered.folders[0].path, "Music");
        let mut query = BrowseQuery {
            username: "alice".into(),
            folder: Some("Music".into()),
            recursive: true,
            ..Default::default()
        };
        assert_eq!(page(&store.connection, &query).unwrap().total, 2801);
        query.recursive = false;
        assert_eq!(page(&store.connection, &query).unwrap().total, 0);
        query.folder = None;
        query.order = "bitrate".into();
        assert!(page(&store.connection, &query).is_ok());
    }
    #[test]
    fn full_listing_is_paged_filtered_and_isolated_from_other_users() {
        let store = crate::store::Store::open(std::path::Path::new(":memory:")).unwrap();
        let files = (0..2601).map(|i| FileView {
            username: "alice".into(),
            path: format!("Music\\Album {}\\{:04}.flac", i / 100, i),
            size: 20,
            free_slots: 0,
            speed: 0,
            attributes: Default::default(),
        });
        let view = save(&store.connection, "alice", files).unwrap();
        assert_eq!(
            (view.total, view.files.len(), view.truncated),
            (2601, 100, false)
        );
        let mut query = BrowseQuery {
            username: "alice".into(),
            offset: 2500,
            ..Default::default()
        };
        let result = page(&store.connection, &query).unwrap();
        assert_eq!((result.total, result.files.len()), (2601, 100));
        query.offset = 0;
        query.folder = Some("Music\\Album 26".into());
        let result = page(&store.connection, &query).unwrap();
        assert_eq!(result.total, 1);
        query.folder = None;
        query.folders = true;
        assert_eq!(page(&store.connection, &query).unwrap().total, 27);
        query.filter = "Album 26".into();
        assert_eq!(page(&store.connection, &query).unwrap().total, 1);
        query.username = "bob".into();
        assert_eq!(page(&store.connection, &query).unwrap().total, 0);
        query.username = "alice".into();
        query.filter = "' OR 1=1 --".into();
        assert_eq!(page(&store.connection, &query).unwrap().total, 0);
    }
}
