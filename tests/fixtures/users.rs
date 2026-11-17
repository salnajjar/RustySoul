use super::*;
use rustysoul::model::LibrarySettings;

fn read_frame(stream: &mut TcpStream) -> Vec<u8> {
    let mut length = [0; 4];
    stream.read_exact(&mut length).unwrap();
    let mut body = vec![0; u32::from_le_bytes(length) as usize];
    stream.read_exact(&mut body).unwrap();
    body
}
fn listing(stream: &mut TcpStream, code: u32) {
    if code == 4 {
        let mut payload = 1_u32.to_le_bytes().to_vec();
        payload.extend(string("Music\\Large album"));
        payload.extend(3001_u32.to_le_bytes());
        for i in 0..3001 {
            payload.push(1);
            payload.extend(string(&format!("{i:04}.flac")));
            payload.extend((if i == 0 { 0_u64 } else { 100_u64 }).to_le_bytes());
            payload.extend(string("flac"));
            payload.extend(0_u32.to_le_bytes());
        }
        payload.extend(0_u32.to_le_bytes());
        send(stream, 5, &compressed(&payload));
    } else if code == 15 {
        let mut payload = string("Independent fixture profile <script>inert</script>");
        payload.push(0);
        payload.extend(4_u32.to_le_bytes());
        payload.extend(2_u32.to_le_bytes());
        payload.push(1);
        payload.extend(0_u32.to_le_bytes());
        send(stream, 16, &payload);
    }
}
fn discovery_fixture(reverse: bool) {
    let server_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server_port = server_listener.local_addr().unwrap().port();
    let peer_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let target = peer_listener.local_addr().unwrap().port();
    let incoming = TcpListener::bind("127.0.0.1:0").unwrap();
    let incoming_port = incoming.local_addr().unwrap().port();
    drop(incoming);
    let (status_tx, status_rx) = mpsc::channel();
    let peer_thread = if !reverse {
        Some(std::thread::spawn(move || {
            let (mut peer, _) = peer_listener.accept().unwrap();
            peer.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
            assert_eq!(read_frame(&mut peer)[0], 1);
            while let Ok((code, _)) = receive(&mut peer) {
                listing(&mut peer, code);
            }
        }))
    } else {
        drop(peer_listener);
        None
    };
    let server = std::thread::spawn(move || {
        let (mut stream, _) = server_listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let mut reverse_peer = None;
        loop {
            if let Ok(status) = status_rx.try_recv() {
                let mut payload = string("fixture-peer");
                payload.extend(u32::to_le_bytes(status));
                payload.push(0);
                send(&mut stream, 7, &payload);
            }
            let (code, payload) = match receive(&mut stream) {
                Ok(v) => v,
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
            match code {
                1 => {
                    let mut reply = vec![1];
                    reply.extend(string("fixture login"));
                    reply.extend([127, 0, 0, 1]);
                    reply.extend(string("fixture"));
                    reply.extend(0_u32.to_le_bytes());
                    send(&mut stream, 1, &reply);
                }
                5 => {
                    let n = u32::from_le_bytes(payload[..4].try_into().unwrap()) as usize;
                    let name = std::str::from_utf8(&payload[4..4 + n]).unwrap();
                    let mut reply = string(name);
                    if name == "missing-user" {
                        reply.push(0);
                    } else {
                        reply.push(1);
                        reply.extend(2_u32.to_le_bytes());
                        reply.extend(4096_u32.to_le_bytes());
                        reply.extend(0_u64.to_le_bytes());
                        reply.extend(3001_u32.to_le_bytes());
                        reply.extend(1_u32.to_le_bytes());
                    }
                    send(&mut stream, 5, &reply);
                }
                3 => {
                    let mut reply = string("fixture-peer");
                    reply.extend([1, 0, 0, 127]);
                    reply.extend(u32::from(target).to_le_bytes());
                    reply.extend(0_u64.to_le_bytes());
                    send(&mut stream, 3, &reply);
                }
                18 if reverse => {
                    let token = u32::from_le_bytes(payload[..4].try_into().unwrap());
                    let mut peer = TcpStream::connect(("127.0.0.1", incoming_port)).unwrap();
                    peer.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
                    let mut body = vec![0];
                    body.extend(token.to_le_bytes());
                    peer.write_all(&(body.len() as u32).to_le_bytes()).unwrap();
                    peer.write_all(&body).unwrap();
                    reverse_peer = Some(std::thread::spawn(move || {
                        while let Ok((code, _)) = receive(&mut peer) {
                            listing(&mut peer, code);
                        }
                    }));
                }
                _ => {}
            }
        }
        if let Some(thread) = reverse_peer {
            thread.join().unwrap();
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config {
        data_dir: dir.path().into(),
        ..Default::default()
    };
    config.soulseek.server = "127.0.0.1".into();
    config.soulseek.port = server_port;
    config.soulseek.listen_port = incoming_port;
    let handle = engine::start(
        config,
        Secrets::new(
            "s".repeat(32),
            "fixture-self".into(),
            "fixture-password".into(),
        )
        .unwrap(),
    )
    .unwrap();
    wait_for(&handle, |s| s.connection == "connected");
    assert!(handle.snapshot.borrow().network.connect_on_startup);
    assert!(!handle.snapshot.borrow().library_settings.index_on_startup);
    handle
        .commands
        .send(Command::LookupUser {
            username: "fixture-peer".into(),
        })
        .unwrap();
    handle
        .commands
        .send(Command::Browse {
            username: "fixture-peer".into(),
        })
        .unwrap();
    wait_for(&handle, |s| {
        s.users.get("fixture-peer").is_some_and(|u| {
            u.status == "online"
                && u.description
                    .as_ref()
                    .is_some_and(|d| d.contains("Independent fixture"))
        })
    });
    wait_for(&handle, |s| {
        s.browse
            .get("fixture-peer")
            .is_some_and(|b| !b.pending && b.total == 3001)
    });
    assert_eq!(
        handle.snapshot.borrow().browse["fixture-peer"].files.len(),
        100
    );
    status_tx.send(1).unwrap();
    wait_for(&handle, |s| s.users["fixture-peer"].status == "away");
    status_tx.send(0).unwrap();
    wait_for(&handle, |s| s.users["fixture-peer"].status == "offline");
    handle
        .commands
        .send(Command::LookupUser {
            username: "missing-user".into(),
        })
        .unwrap();
    wait_for(&handle, |s| {
        s.users
            .get("missing-user")
            .is_some_and(|u| u.exists == Some(false) && !u.pending)
    });
    handle
        .commands
        .send(Command::SetBuddy {
            username: "fixture-peer".into(),
            buddy: true,
        })
        .unwrap();
    wait_for(&handle, |s| s.buddies == ["fixture-peer"]);
    handle.commands.send(Command::Disconnect).unwrap();
    wait_for(&handle, |s| {
        s.connection == "disconnected" && s.users["fixture-peer"].status == "unknown"
    });
    handle.stop.send(()).unwrap();
    handle.thread.join().unwrap();
    server.join().unwrap();
    if let Some(thread) = peer_thread {
        thread.join().unwrap();
    }
    let store = rustysoul::store::Store::open(&dir.path().join("rustysoul.sqlite3")).unwrap();
    assert_eq!(store.buddies().unwrap(), ["fixture-peer"]);
}
#[test]
fn lookup_presence_profile_and_full_browse_open_a_fresh_direct_connection() {
    discovery_fixture(false);
}
#[test]
fn lookup_and_browse_fall_back_to_a_reverse_connection() {
    discovery_fixture(true);
}

#[test]
fn own_username_browses_all_cached_shares_while_disconnected() {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let root = dir.path().join("library");
    std::fs::create_dir(&root).unwrap();
    let store = rustysoul::store::Store::open(&dir.path().join("rustysoul.sqlite3")).unwrap();
    drop(store);
    let mut db = rusqlite::Connection::open(dir.path().join("rustysoul.sqlite3")).unwrap();
    let tx = db.transaction().unwrap();
    for i in 0..3001 {
        let real = root.join(format!("{i}.flac"));
        let virtual_path = format!("library\\{i}.flac");
        tx.execute(
            "INSERT INTO library_cache VALUES(?1,?2,100,'0','0','[]')",
            rusqlite::params![real.to_string_lossy(), virtual_path],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    let mut config = Config {
        data_dir: dir.path().into(),
        ..Default::default()
    };
    config.soulseek.shares = vec![root];
    config.soulseek.auto_connect = false;
    let handle = engine::start(
        config,
        Secrets::new("s".repeat(32), "fixture-self".into(), "password".into()).unwrap(),
    )
    .unwrap();
    handle
        .commands
        .send(Command::Browse {
            username: "fixture-self".into(),
        })
        .unwrap();
    wait_for(&handle, |s| {
        s.browse
            .get("fixture-self")
            .is_some_and(|b| b.total == 3001)
    });
    assert_eq!(handle.snapshot.borrow().library.scan_id, 0);
    assert_eq!(handle.snapshot.borrow().shares.files, 3001);
    let page = rustysoul::browse::page(
        &db,
        &rustysoul::browse::BrowseQuery {
            username: "fixture-self".into(),
            offset: 3000,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!((page.total, page.files.len()), (3001, 1));
    handle
        .commands
        .send(Command::UpdateLibrarySettings {
            settings: LibrarySettings {
                index_on_startup: true,
            },
        })
        .unwrap();
    wait_for(&handle, |s| s.library_settings.index_on_startup);
    handle
        .commands
        .send(Command::CloseBrowse {
            username: "fixture-self".into(),
        })
        .unwrap();
    wait_for(&handle, |s| s.browse.is_empty());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM browse_files", [], |r| r
            .get::<_, u32>(0))
            .unwrap(),
        0
    );
    handle.stop.send(()).unwrap();
    handle.thread.join().unwrap();
    let store = rustysoul::store::Store::open(&dir.path().join("rustysoul.sqlite3")).unwrap();
    assert!(store.library_settings().unwrap().index_on_startup);
}
