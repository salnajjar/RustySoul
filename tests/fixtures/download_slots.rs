//! Remote queues are not active slots. All accepted offers still obey hard limits.
use super::*;
use rustysoul::model::{DownloadFile, NetworkSettings};

fn init_named(s: &mut TcpStream, user: &str, kind: &str) {
    let mut body = vec![1];
    body.extend(string(user));
    body.extend(string(kind));
    body.extend(0_u32.to_le_bytes());
    s.write_all(&(body.len() as u32).to_le_bytes()).unwrap();
    s.write_all(&body).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
}
fn control(port: u16, user: &str) -> TcpStream {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    init_named(&mut s, user, "P");
    send(&mut s, 15, &[]);
    receive_code(&mut s, 16);
    s
}
fn offer(
    s: &mut TcpStream,
    port: u16,
    user: &str,
    path: &str,
    token: u32,
    allowed: bool,
) -> Option<TcpStream> {
    let mut body = 1_u32.to_le_bytes().to_vec();
    body.extend(token.to_le_bytes());
    body.extend(string(path));
    body.extend(4096_u64.to_le_bytes());
    send(s, 40, &body);
    let reply = receive_code(s, 41);
    assert_eq!(&reply[..4], token.to_le_bytes());
    assert_eq!(reply[4], u8::from(allowed));
    if !allowed {
        return None;
    }
    let mut f = TcpStream::connect(("127.0.0.1", port)).unwrap();
    init_named(&mut f, user, "F");
    f.write_all(&token.to_le_bytes()).unwrap();
    let mut offset = [0; 8];
    f.read_exact(&mut offset).unwrap();
    assert_eq!(u64::from_le_bytes(offset), 0);
    Some(f)
}

#[test]
fn another_users_transfer_fills_slots_when_a_peer_can_only_send_one_file() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server_port = listener.local_addr().unwrap().port();
    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let peer_port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let server = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        while let Ok((code, _)) = receive(&mut s) {
            if code == 1 {
                let mut r = vec![1];
                r.extend(string("Slot fixture"));
                r.extend([1, 0, 0, 127]);
                r.extend(string("checksum"));
                r.push(0);
                send(&mut s, 1, &r);
            }
        }
    });
    let temp = tempfile::tempdir().unwrap();
    let mut config = Config {
        data_dir: temp.path().into(),
        ..Default::default()
    };
    config.soulseek.server = "127.0.0.1".into();
    config.soulseek.port = server_port;
    config.soulseek.listen_port = peer_port;
    config.soulseek.auto_connect = false;
    let h = engine::start(
        config,
        Secrets::new(
            "s".repeat(32),
            "fixture-user".into(),
            "fixture-password".into(),
        )
        .unwrap(),
    )
    .unwrap();
    h.commands
        .send(Command::UpdateNetwork {
            settings: NetworkSettings {
                // Storage admission has its own fixture; scheduler assertions must
                // not depend on the CI runner's free space or probe timing.
                minimum_free_space_mib: 0,
                port: peer_port,
                download_slots: 2,
                download_slots_per_user: 2,
                ..Default::default()
            },
        })
        .unwrap();
    h.commands.send(Command::Connect).unwrap();
    wait_for(&h, |s| s.connection == "connected");
    let mut alice = control(peer_port, "alice");
    let mut bob = control(peer_port, "bob");
    let mut carol = control(peer_port, "carol");
    h.commands
        .send(Command::DownloadBatch {
            files: [
                ("alice", "a1.flac"),
                ("alice", "a2.flac"),
                ("alice", "a3.flac"),
                ("bob", "b.flac"),
                ("carol", "c.flac"),
            ]
            .map(|(u, p)| DownloadFile {
                username: u.into(),
                path: p.into(),
                size: 4096,
            })
            .to_vec(),
        })
        .unwrap();
    assert_eq!(receive_code(&mut alice, 43), string("a1.flac"));
    assert_eq!(receive_code(&mut bob, 43), string("b.flac"));
    assert_eq!(receive_code(&mut carol, 43), string("c.flac"));
    let mut a = offer(&mut alice, peer_port, "alice", "a1.flac", 7101, true).unwrap();
    // Alice has only one remote upload slot: a2 stays queued there. That must
    // leave our second actual slot available to Bob.
    assert_eq!(receive_code(&mut alice, 43), string("a2.flac"));
    let mut b = offer(&mut bob, peer_port, "bob", "b.flac", 7102, true).unwrap();
    wait_for(&h, |s| {
        s.downloads
            .iter()
            .filter(|d| d.status == "downloading")
            .count()
            == 2
    });
    offer(&mut carol, peer_port, "carol", "c.flac", 7103, false);
    wait_for(&h, |s| {
        s.downloads
            .iter()
            .any(|d| d.username == "carol" && d.status == "waiting")
    });
    assert!(h.snapshot.borrow().error.is_none());
    let db = rusqlite::Connection::open(temp.path().join("rustysoul.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM download_retries", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0,
        "local slot waiting must not spend a retry attempt"
    );
    b.write_all(&vec![2; 4096]).unwrap();
    drop(b);
    assert_eq!(receive_code(&mut carol, 43), string("c.flac"));
    let mut c = offer(&mut carol, peer_port, "carol", "c.flac", 7104, true).unwrap();
    wait_for(&h, |s| {
        s.downloads
            .iter()
            .filter(|d| d.status == "downloading")
            .count()
            == 2
            && s.downloads
                .iter()
                .any(|d| d.username == "carol" && d.status == "downloading")
    });
    assert!(
        h.snapshot
            .borrow()
            .downloads
            .iter()
            .any(|d| d.path == "a2.flac" && d.status == "queued")
    );
    a.write_all(&vec![1; 4096]).unwrap();
    drop(a);
    c.write_all(&vec![3; 4096]).unwrap();
    drop(c);
    wait_for(&h, |s| {
        s.downloads
            .iter()
            .filter(|d| d.status == "completed")
            .count()
            == 3
    });
    h.commands
        .send(Command::CancelDownloads {
            username: "alice".into(),
            paths: vec![],
        })
        .unwrap();
    wait_for(&h, |s| {
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
            .filter(|d| d.status == "completed")
            .count(),
        3
    );
    h.commands
        .send(Command::UpdateNetwork {
            settings: NetworkSettings {
                // Storage admission has its own fixture; scheduler assertions must
                // not depend on the CI runner's free space or probe timing.
                minimum_free_space_mib: 0,
                port: peer_port,
                download_slots: 0,
                download_slots_per_user: 2,
                ..Default::default()
            },
        })
        .unwrap();
    wait_for(&h, |s| s.network.download_slots == 0);
    h.commands
        .send(Command::DownloadBatch {
            files: ["p1.flac", "p2.flac", "p3.flac"]
                .map(|p| DownloadFile {
                    username: "alice".into(),
                    path: p.into(),
                    size: 4096,
                })
                .to_vec(),
        })
        .unwrap();
    wait_for(&h, |s| {
        s.downloads
            .iter()
            .filter(|d| d.path.starts_with('p') && d.status == "waiting")
            .count()
            == 3
    });
    h.commands
        .send(Command::UpdateTransferQueue {
            direction: rustysoul::model::HistoryDirection::Downloads,
            username: "alice".into(),
            path: "p3.flac".into(),
            priority: Some(1),
            movement: None,
        })
        .unwrap();
    h.commands
        .send(Command::UpdateTransferQueue {
            direction: rustysoul::model::HistoryDirection::Downloads,
            username: "alice".into(),
            path: "p2.flac".into(),
            priority: None,
            movement: Some(rustysoul::model::QueueMove::Top),
        })
        .unwrap();
    wait_for(&h, |s| {
        s.downloads
            .iter()
            .any(|d| d.path == "p3.flac" && d.priority == 1)
    });
    h.commands
        .send(Command::UpdateNetwork {
            settings: NetworkSettings {
                // Storage admission has its own fixture; scheduler assertions must
                // not depend on the CI runner's free space or probe timing.
                minimum_free_space_mib: 0,
                port: peer_port,
                download_slots: 1,
                download_slots_per_user: 2,
                ..Default::default()
            },
        })
        .unwrap();
    for (n, path) in ["p3.flac", "p2.flac", "p1.flac"].into_iter().enumerate() {
        assert_eq!(
            receive_code(&mut alice, 43),
            string(path),
            "Local priority and manual order must choose actual requests"
        );
        let mut f = offer(&mut alice, peer_port, "alice", path, 7200 + n as u32, true).unwrap();
        f.write_all(&vec![4; 4096]).unwrap();
        drop(f);
        wait_for(&h, |s| {
            s.downloads
                .iter()
                .any(|d| d.path == path && d.status == "completed")
        });
    }
    h.stop.send(()).unwrap();
    h.thread.join().unwrap();
    drop(alice);
    drop(bob);
    drop(carol);
    server.join().unwrap();
}
