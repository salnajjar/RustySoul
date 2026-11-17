use rustysoul::{
    config::{Config, Secrets},
    engine,
    model::Command,
};
use std::time::{Duration, Instant};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
struct Server {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    port: u16,
    peer_port: u16,
}
impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let peer = TcpListener::bind("127.0.0.1:0").unwrap();
        let peer_port = peer.local_addr().unwrap().port();
        drop(peer);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                stream
                    .set_read_timeout(Some(Duration::from_millis(100)))
                    .unwrap();
                loop {
                    if stopping.load(Ordering::Relaxed) {
                        break;
                    }
                    let mut len = [0; 4];
                    match stream.read_exact(&mut len) {
                        Ok(()) => {}
                        Err(e)
                            if matches!(
                                e.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) =>
                        {
                            continue;
                        }
                        Err(_) => break,
                    };
                    let size = u32::from_le_bytes(len) as usize;
                    if !(4..=8192).contains(&size) {
                        break;
                    }
                    let mut data = vec![0; size];
                    if stream.read_exact(&mut data).is_err() {
                        break;
                    }
                    if data[..4] == 1_u32.to_le_bytes() {
                        let mut body = vec![];
                        if data.windows(14).any(|w| w == b"wrong-password") {
                            body.push(0);
                            put_string(&mut body, "INVALIDPASS");
                        } else {
                            body.push(1);
                            put_string(&mut body, "Offline test");
                            body.extend([1, 0, 0, 127]);
                            put_string(&mut body, "checksum");
                            body.push(0);
                        }
                        let mut frame = ((body.len() + 4) as u32).to_le_bytes().to_vec();
                        frame.extend(1_u32.to_le_bytes());
                        frame.extend(body);
                        if stream.write_all(&frame).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Self {
            stop,
            thread: Some(thread),
            port,
            peer_port,
        }
    }
}
fn put_string(out: &mut Vec<u8>, s: &str) {
    out.extend((s.len() as u32).to_le_bytes());
    out.extend(s.as_bytes());
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
fn disconnect(handle: &engine::EngineHandle) {
    handle.commands.send(Command::Disconnect).unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while handle.snapshot.borrow().connection != "disconnected" {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn wait(handle: &engine::EngineHandle, revision: u64) {
    let end = Instant::now() + Duration::from_secs(5);
    while handle.snapshot.borrow().credentials_revision != revision {
        assert!(Instant::now() < end, "Credential update timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
}
#[test]
fn credentials_survive_restart_keep_password_and_can_be_removed() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start();
    let mut config = Config {
        data_dir: dir.path().into(),
        ..Default::default()
    };
    config.soulseek.server = "127.0.0.1".into();
    config.soulseek.port = server.port;
    config.soulseek.listen_port = server.peer_port;
    config.soulseek.auto_connect = false;
    let secrets = || Secrets::new("s".repeat(32), "".into(), "".into()).unwrap();
    let first = engine::start(config.clone(), secrets()).unwrap();
    first
        .commands
        .send(Command::UpdateCredentials {
            username: "fixture".into(),
            password: Some("never-return-this-secret".into()),
        })
        .unwrap();
    wait(&first, 1);
    assert_eq!(first.snapshot.borrow().connection, "connected");
    let json = serde_json::to_string(&*first.snapshot.borrow()).unwrap();
    assert!(!json.contains("never-return-this-secret"));
    disconnect(&first);
    first
        .commands
        .send(Command::UpdateCredentials {
            username: "wrong-user".into(),
            password: Some("wrong-password".into()),
        })
        .unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while first.snapshot.borrow().error.is_none() {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(first.snapshot.borrow().credentials_revision, 1);
    assert_eq!(first.snapshot.borrow().username, "fixture");
    first.stop.send(()).unwrap();
    first.thread.join().unwrap();
    let second = engine::start(config.clone(), secrets()).unwrap();
    assert!(second.snapshot.borrow().credentials_configured);
    assert_eq!(second.snapshot.borrow().username, "fixture");
    second
        .commands
        .send(Command::UpdateCredentials {
            username: "fixture".into(),
            password: None,
        })
        .unwrap();
    wait(&second, 1);
    disconnect(&second);
    second.commands.send(Command::ClearCredentials).unwrap();
    wait(&second, 2);
    second.stop.send(()).unwrap();
    second.thread.join().unwrap();
    let third = engine::start(
        config,
        Secrets::new(
            "s".repeat(32),
            "env-bootstrap".into(),
            "env-password".into(),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(!third.snapshot.borrow().credentials_configured);
    assert!(third.snapshot.borrow().username.is_empty());
    third.stop.send(()).unwrap();
    third.thread.join().unwrap();
}

#[test]
fn network_settings_port_conflicts_and_batch_queue_are_transactional() {
    use rustysoul::model::{DownloadFile, NetworkSettings};
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start();
    let mut config = Config {
        data_dir: dir.path().into(),
        ..Default::default()
    };
    config.soulseek.server = "127.0.0.1".into();
    config.soulseek.port = server.port;
    config.soulseek.listen_port = server.peer_port;
    config.soulseek.auto_connect = false;
    config.limits.transfers = 2;
    let secrets = || Secrets::new("s".repeat(32), "".into(), "".into()).unwrap();
    let h = engine::start(config.clone(), secrets()).unwrap();
    fn until(h: &engine::EngineHandle, p: impl Fn(&rustysoul::model::Snapshot) -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !p(&h.snapshot.borrow()) {
            assert!(
                Instant::now() < end,
                "Command timed out: {:?}",
                h.snapshot.borrow().error
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
    let settings = NetworkSettings {
        port: occupied.local_addr().unwrap().port(),
        ..Default::default()
    };
    h.commands
        .send(Command::UpdateNetwork {
            settings: settings.clone(),
        })
        .unwrap();
    until(&h, |s| {
        s.error.as_ref().is_some_and(|e| e.contains("already used"))
    });
    assert_eq!(h.snapshot.borrow().network.port, server.peer_port);
    drop(occupied);
    let settings = NetworkSettings {
        port: server.peer_port,
        upload_slots: 3,
        download_limit_kib: 100,
        ..settings
    };
    h.commands
        .send(Command::UpdateNetwork {
            settings: settings.clone(),
        })
        .unwrap();
    until(&h, |s| s.network == settings);
    h.commands
        .send(Command::UpdateCredentials {
            username: "fixture".into(),
            password: Some("fixture-password".into()),
        })
        .unwrap();
    wait(&h, 1);
    let f = |name: &str| DownloadFile {
        username: "peer".into(),
        path: format!("Music/{name}.flac"),
        size: 100,
    };
    h.commands
        .send(Command::DownloadBatch {
            files: vec![
                f("a"),
                DownloadFile {
                    path: "../secret".into(),
                    ..f("invalid")
                },
            ],
        })
        .unwrap();
    until(&h, |s| s.error.is_some());
    assert!(h.snapshot.borrow().downloads.is_empty());
    h.commands
        .send(Command::DownloadBatch {
            files: vec![f("a"), f("b"), f("c")],
        })
        .unwrap();
    until(&h, |s| {
        s.error.as_ref().is_some_and(|e| e.contains("queue space"))
    });
    assert!(h.snapshot.borrow().downloads.is_empty());
    h.commands
        .send(Command::DownloadBatch {
            files: vec![f("a"), f("b"), f("a")],
        })
        .unwrap();
    until(&h, |s| s.downloads.len() == 2);
    assert_eq!(h.snapshot.borrow().transfer_capacity, 2);
    h.commands
        .send(Command::UpdateNetwork {
            settings: NetworkSettings {
                port: settings.port.wrapping_add(1),
                ..settings.clone()
            },
        })
        .unwrap();
    until(&h, |s| {
        s.error.as_ref().is_some_and(|e| e.contains("Disconnect"))
    });
    disconnect(&h);
    h.stop.send(()).unwrap();
    h.thread.join().unwrap();
    let h = engine::start(config, secrets()).unwrap();
    assert_eq!(h.snapshot.borrow().network, settings);
    assert_eq!(h.snapshot.borrow().downloads.len(), 2);
    h.stop.send(()).unwrap();
    h.thread.join().unwrap();
}

#[test]
fn download_scheduler_enforces_total_and_per_user_slots_and_zero_pauses_new_work() {
    use rustysoul::model::{
        DownloadFile, HistoryDirection, HistoryStatus, NetworkSettings, TransferAction,
    };
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start();
    let mut config = Config {
        data_dir: dir.path().into(),
        ..Default::default()
    };
    config.soulseek.server = "127.0.0.1".into();
    config.soulseek.port = server.port;
    config.soulseek.listen_port = server.peer_port;
    config.soulseek.auto_connect = false;
    let h = engine::start(
        config,
        Secrets::new("s".repeat(32), "".into(), "".into()).unwrap(),
    )
    .unwrap();
    fn until(h: &engine::EngineHandle, p: impl Fn(&rustysoul::model::Snapshot) -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !p(&h.snapshot.borrow()) {
            assert!(Instant::now() < end, "{:?}", h.snapshot.borrow().error);
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    let mut settings = NetworkSettings {
        port: server.peer_port,
        download_slots: 2,
        download_slots_per_user: 1,
        ..Default::default()
    };
    h.commands
        .send(Command::UpdateNetwork {
            settings: settings.clone(),
        })
        .unwrap();
    until(&h, |s| s.network == settings);
    h.commands
        .send(Command::UpdateCredentials {
            username: "fixture".into(),
            password: Some("fixture-password".into()),
        })
        .unwrap();
    wait(&h, 1);
    h.commands
        .send(Command::DownloadBatch {
            files: [
                ("alice", "a"),
                ("alice", "b"),
                ("bob", "c"),
                ("bob", "d"),
                ("charlie", "e"),
            ]
            .into_iter()
            .map(|(user, path)| DownloadFile {
                username: user.into(),
                path: format!("Music/{path}.flac"),
                size: 100,
            })
            .collect(),
        })
        .unwrap();
    until(&h, |s| {
        s.downloads.len() == 5 && s.downloads.iter().filter(|d| d.status == "queued").count() == 3
    });
    {
        let s = h.snapshot.borrow();
        let active: Vec<_> = s
            .downloads
            .iter()
            .filter(|d| d.status == "queued")
            .map(|d| d.username.as_str())
            .collect();
        assert_eq!(active, ["alice", "bob", "charlie"]);
    }
    // Clearing all history never removes active reservations or waiting files.
    h.commands
        .send(Command::ClearTransfers {
            direction: HistoryDirection::Downloads,
            status: HistoryStatus::All,
        })
        .unwrap();
    until(&h, |s| {
        s.notice.as_ref().is_some_and(|n| n.starts_with("Cleared"))
    });
    assert_eq!(h.snapshot.borrow().downloads.len(), 5);
    settings.download_slots = 0;
    h.commands
        .send(Command::UpdateNetwork {
            settings: settings.clone(),
        })
        .unwrap();
    until(&h, |s| s.network == settings);
    for (user, path) in [("alice", "a"), ("bob", "c")] {
        h.commands
            .send(Command::Transfer {
                username: user.into(),
                path: format!("Music/{path}.flac"),
                action: TransferAction::Cancel,
            })
            .unwrap();
    }
    until(&h, |s| {
        s.downloads
            .iter()
            .filter(|d| d.status == "cancelled")
            .count()
            == 2
    });
    assert_eq!(
        h.snapshot
            .borrow()
            .downloads
            .iter()
            .filter(|d| d.status == "waiting")
            .count(),
        2
    );
    h.commands
        .send(Command::ClearTransfers {
            direction: HistoryDirection::Downloads,
            status: HistoryStatus::Cancelled,
        })
        .unwrap();
    until(&h, |s| s.downloads.len() == 3);
    settings.download_slots = 1;
    h.commands
        .send(Command::UpdateNetwork {
            settings: settings.clone(),
        })
        .unwrap();
    until(&h, |s| {
        s.downloads.iter().filter(|d| d.status == "queued").count() == 3
    });
    h.stop.send(()).unwrap();
    h.thread.join().unwrap();
}

#[test]
fn bridge_deployment_port_is_authoritative_and_cannot_be_changed_via_api() {
    use rustysoul::model::NetworkSettings;
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let reserved = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reserved.local_addr().unwrap().port();
    drop(reserved);
    let store = rustysoul::store::Store::open(&dir.path().join("rustysoul.sqlite3")).unwrap();
    store
        .save_network(&NetworkSettings {
            port: 2234,
            upnp: true,
            ..Default::default()
        })
        .unwrap();
    drop(store);
    let h = engine::start(
        Config {
            data_dir: dir.path().into(),
            published_transfer_port: Some(port),
            ..Default::default()
        },
        Secrets::new("s".repeat(32), "".into(), "".into()).unwrap(),
    )
    .unwrap();
    assert_eq!(h.snapshot.borrow().port_binding.fixed_port, Some(port));
    assert_eq!(h.snapshot.borrow().network.port, port);
    assert!(!h.snapshot.borrow().network.upnp);
    h.commands
        .send(Command::UpdateNetwork {
            settings: NetworkSettings {
                port: if port == 2234 { 2235 } else { 2234 },
                ..Default::default()
            },
        })
        .unwrap();
    let end = Instant::now() + Duration::from_secs(3);
    while h.snapshot.borrow().error.is_none() {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        h.snapshot
            .borrow()
            .error
            .as_ref()
            .unwrap()
            .contains("fixed published port")
    );
    h.stop.send(()).unwrap();
    h.thread.join().unwrap();
}

#[test]
fn saved_startup_preference_connects_with_the_verified_account_after_restart() {
    use rustysoul::model::NetworkSettings;
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start();
    let mut config = Config {
        data_dir: dir.path().into(),
        ..Default::default()
    };
    config.soulseek.server = "127.0.0.1".into();
    config.soulseek.port = server.port;
    config.soulseek.listen_port = server.peer_port;
    config.soulseek.auto_connect = false;
    let secrets = || Secrets::new("s".repeat(32), "".into(), "".into()).unwrap();
    let first = engine::start(config.clone(), secrets()).unwrap();
    first
        .commands
        .send(Command::UpdateCredentials {
            username: "fixture".into(),
            password: Some("fixture-password".into()),
        })
        .unwrap();
    wait(&first, 1);
    let settings = NetworkSettings {
        port: server.peer_port,
        connect_on_startup: true,
        ..Default::default()
    };
    first
        .commands
        .send(Command::UpdateNetwork {
            settings: settings.clone(),
        })
        .unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while first.snapshot.borrow().network != settings {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(20));
    }
    first.stop.send(()).unwrap();
    first.thread.join().unwrap();
    let second = engine::start(config, secrets()).unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while second.snapshot.borrow().connection != "connected" {
        assert!(Instant::now() < end, "{:?}", second.snapshot.borrow().error);
        std::thread::sleep(Duration::from_millis(20));
    }
    second.stop.send(()).unwrap();
    second.thread.join().unwrap();
}

#[test]
fn manual_retry_scopes_failed_downloads_and_preserves_completed_and_other_users() {
    use rustysoul::model::NetworkSettings;
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let server = Server::start();
    let mut config = Config {
        data_dir: dir.path().into(),
        ..Default::default()
    };
    config.soulseek.server = "127.0.0.1".into();
    config.soulseek.port = server.port;
    config.soulseek.listen_port = server.peer_port;
    config.soulseek.auto_connect = false;
    let mut store = rustysoul::store::Store::open(&dir.path().join("rustysoul.sqlite3")).unwrap();
    store
        .save_downloads(
            &[
                ("alice", "a", "failed"),
                ("alice", "b", "timed_out"),
                ("bob", "c", "failed"),
                ("alice", "done", "completed"),
            ]
            .map(|(u, p, status)| rustysoul::store::SavedDownload {
                username: u.into(),
                path: p.into(),
                status: status.into(),
                size: 10,
                directory: dir.path().join("downloads").to_string_lossy().into_owned(),
                local_path: String::new(),
            }),
        )
        .unwrap();
    drop(store);
    let h = engine::start(
        config,
        Secrets::new("s".repeat(32), "".into(), "".into()).unwrap(),
    )
    .unwrap();
    h.commands
        .send(Command::UpdateNetwork {
            settings: NetworkSettings {
                port: server.peer_port,
                download_slots: 0,
                ..Default::default()
            },
        })
        .unwrap();
    h.commands
        .send(Command::UpdateCredentials {
            username: "fixture".into(),
            password: Some("fixture-password".into()),
        })
        .unwrap();
    wait(&h, 1);
    h.commands
        .send(Command::RetryDownloads {
            username: Some("alice".into()),
        })
        .unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while h
        .snapshot
        .borrow()
        .downloads
        .iter()
        .filter(|d| d.username == "alice" && d.status == "waiting")
        .count()
        != 2
    {
        assert!(Instant::now() < end, "{:?}", h.snapshot.borrow().error);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        h.snapshot
            .borrow()
            .downloads
            .iter()
            .any(|d| d.username == "bob" && d.status == "failed")
    );
    assert!(
        h.snapshot
            .borrow()
            .downloads
            .iter()
            .any(|d| d.path == "done" && d.status == "completed")
    );
    h.stop.send(()).unwrap();
    h.thread.join().unwrap();
}

#[test]
fn bulk_download_cancellation_is_scoped_durable_and_available_offline_without_touching_completed_files()
 {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    let downloads = dir.path().join("downloads");
    std::fs::create_dir_all(downloads.join("alice/Music")).unwrap();
    let completed = downloads.join("alice/Music/done.flac");
    std::fs::write(&completed, b"safe completed bytes").unwrap();
    let mut store = rustysoul::store::Store::open(&dir.path().join("rustysoul.sqlite3")).unwrap();
    store
        .save_downloads(
            &[
                ("alice", "one.flac", "waiting"),
                ("alice", "two.flac", "paused_waiting"),
                ("alice", "retry.flac", "retry_waiting"),
                ("alice", "done.flac", "completed"),
                ("bob", "other.flac", "waiting"),
            ]
            .map(|(u, p, status)| rustysoul::store::SavedDownload {
                username: u.into(),
                path: format!("Music/{p}"),
                size: 20,
                status: status.into(),
                directory: downloads.to_string_lossy().into_owned(),
                local_path: if status == "completed" {
                    "alice/Music/done.flac".into()
                } else {
                    String::new()
                },
            }),
        )
        .unwrap();
    drop(store);
    let seed = rusqlite::Connection::open(dir.path().join("rustysoul.sqlite3")).unwrap();
    seed.execute("INSERT INTO download_retries VALUES('alice','Music/retry.flac',2,strftime('%s','now')+300)",[]).unwrap();
    drop(seed);
    let config = Config {
        data_dir: dir.path().into(),
        ..Default::default()
    };
    let secrets = || Secrets::new("s".repeat(32), "".into(), "".into()).unwrap();
    let h = engine::start(config.clone(), secrets()).unwrap();
    h.commands
        .send(Command::CancelDownloads {
            username: "alice".into(),
            paths: vec![
                "Music/one.flac".into(),
                "Music/done.flac".into(),
                "Music/missing.flac".into(),
            ],
        })
        .unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while !h
        .snapshot
        .borrow()
        .downloads
        .iter()
        .any(|d| d.path == "Music/one.flac" && d.status == "cancelled")
    {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        h.snapshot
            .borrow()
            .downloads
            .iter()
            .any(|d| d.path == "Music/two.flac" && d.status == "paused_waiting")
    );
    h.commands
        .send(Command::CancelDownloads {
            username: "ALICE".into(),
            paths: vec![],
        })
        .unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while h
        .snapshot
        .borrow()
        .downloads
        .iter()
        .filter(|d| d.status == "cancelled")
        .count()
        != 3
    {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        h.snapshot
            .borrow()
            .downloads
            .iter()
            .any(|d| d.username == "bob" && d.status == "waiting")
    );
    assert!(
        h.snapshot
            .borrow()
            .downloads
            .iter()
            .any(|d| d.path == "Music/done.flac" && d.status == "completed")
    );
    h.stop.send(()).unwrap();
    h.thread.join().unwrap();
    let db = rusqlite::Connection::open(dir.path().join("rustysoul.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM download_retries", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(db);
    assert_eq!(std::fs::read(&completed).unwrap(), b"safe completed bytes");
    let h = engine::start(config, secrets()).unwrap();
    assert_eq!(
        h.snapshot
            .borrow()
            .downloads
            .iter()
            .filter(|d| d.status == "cancelled")
            .count(),
        3
    );
    h.stop.send(()).unwrap();
    h.thread.join().unwrap();
    assert!(
        Command::CancelDownloads {
            username: "alice".into(),
            paths: vec!["../unsafe".into()]
        }
        .validate()
        .is_err()
    );
    assert!(
        Command::CancelDownloads {
            username: "alice".into(),
            paths: vec!["Music/a.flac".into(); 257]
        }
        .validate()
        .is_err()
    );
}

#[test]
fn storage_holds_release_slots_do_not_retry_and_respect_offline_pause_cancel_and_restart() {
    use rustysoul::model::{DownloadFile, TransferAction};
    fn until(h: &engine::EngineHandle, p: impl Fn(&rustysoul::model::Snapshot) -> bool) {
        let end = Instant::now() + Duration::from_secs(10);
        while !p(&h.snapshot.borrow()) {
            assert!(Instant::now() < end, "{:?}", h.snapshot.borrow().error);
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let server = Server::start();
    let mut config = Config {
        data_dir: temp.path().into(),
        ..Default::default()
    };
    config.soulseek.server = "127.0.0.1".into();
    config.soulseek.port = server.port;
    config.soulseek.listen_port = server.peer_port;
    config.soulseek.auto_connect = false;
    let secrets = || Secrets::new("s".repeat(32), "".into(), "".into()).unwrap();
    let h = engine::start(config.clone(), secrets()).unwrap();
    h.commands
        .send(Command::UpdateStorageReserve {
            minimum_free_space_mib: rustysoul::storage::MAX_MINIMUM_MIB,
        })
        .unwrap();
    until(&h, |s| {
        s.network.minimum_free_space_mib == rustysoul::storage::MAX_MINIMUM_MIB
    });
    let original = h.snapshot.borrow().network.clone();
    h.commands
        .send(Command::UpdateCredentials {
            username: "fixture".into(),
            password: Some("fixture-password".into()),
        })
        .unwrap();
    wait(&h, 1);
    h.commands
        .send(Command::DownloadBatch {
            files: ["pause", "cancel", "resume"]
                .into_iter()
                .map(|name| DownloadFile {
                    username: "alice".into(),
                    path: format!("Music/{name}.flac"),
                    size: 100,
                })
                .collect(),
        })
        .unwrap();
    until(&h, |s| {
        s.downloads.len() == 3 && s.downloads.iter().all(|d| d.status == "waiting_for_space")
    });
    assert!(h.snapshot.borrow().error.is_none());
    disconnect(&h);
    for (name, action) in [
        ("pause", TransferAction::Pause),
        ("cancel", TransferAction::Cancel),
    ] {
        h.commands
            .send(Command::Transfer {
                username: "alice".into(),
                path: format!("Music/{name}.flac"),
                action,
            })
            .unwrap();
    }
    until(&h, |s| {
        s.downloads.iter().any(|d| d.status == "paused_waiting")
            && s.downloads.iter().any(|d| d.status == "cancelled")
    });
    h.stop.send(()).unwrap();
    h.thread.join().unwrap();
    let db = rusqlite::Connection::open(temp.path().join("rustysoul.sqlite3")).unwrap();
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM download_retries", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
    drop(db);
    let h = engine::start(config, secrets()).unwrap();
    assert_eq!(
        h.snapshot.borrow().network.minimum_free_space_mib,
        rustysoul::storage::MAX_MINIMUM_MIB
    );
    h.commands
        .send(Command::UpdateStorageReserve {
            minimum_free_space_mib: 0,
        })
        .unwrap();
    until(&h, |s| s.network.minimum_free_space_mib == 0);
    let mut expected = original;
    expected.minimum_free_space_mib = 0;
    assert_eq!(h.snapshot.borrow().network, expected);
    h.commands.send(Command::Connect).unwrap();
    until(&h, |s| s.connection == "connected");
    until(&h, |s| {
        s.downloads
            .iter()
            .any(|d| d.path == "Music/resume.flac" && d.status == "queued")
    });
    let snapshot = h.snapshot.borrow();
    assert_eq!(
        snapshot
            .downloads
            .iter()
            .find(|d| d.path == "Music/pause.flac")
            .unwrap()
            .status,
        "paused_waiting"
    );
    assert_eq!(
        snapshot
            .downloads
            .iter()
            .find(|d| d.path == "Music/cancel.flac")
            .unwrap()
            .status,
        "cancelled"
    );
    drop(snapshot);
    h.stop.send(()).unwrap();
    h.thread.join().unwrap();
}
