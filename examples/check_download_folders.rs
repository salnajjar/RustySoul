use anyhow::Result;
fn main() -> Result<()> {
    let db = rusqlite::Connection::open_with_flags(
        "/data/rustysoul.sqlite3",
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    db.busy_timeout(std::time::Duration::from_secs(5))?;
    let mut stmt=db.prepare("SELECT username,path,size,status,directory,local_path FROM downloads WHERE status NOT IN ('completed','cancelled')")?;
    let rows = stmt.query_map([], |r| {
        Ok(rustysoul::store::SavedDownload {
            username: r.get(0)?,
            path: r.get(1)?,
            size: r.get::<_, i64>(2)? as u64,
            status: r.get(3)?,
            directory: r.get(4)?,
            local_path: r.get(5)?,
        })
    })?;
    let mut checked = 0;
    let mut errors = 0;
    for row in rows {
        let row = row?;
        checked += 1;
        if let Err(e) = rustysoul::downloads::work_directory(&row) {
            errors += 1;
            eprintln!("{}: {e:#}", row.status);
        }
    }
    println!("Checked {checked} pending download folders; {errors} permission/access failures");
    Ok(())
}
