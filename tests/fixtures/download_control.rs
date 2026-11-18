//! Real wire transfers across pause, cancellation, premature EOF and a daemon restart.
use super::*;
use rustysoul::model::{RetryPolicy, TransferAction};

fn control(port: u16) -> TcpStream {
    let mut peer = TcpStream::connect(("127.0.0.1", port)).unwrap();
    init_peer(&mut peer, "P");
    send(&mut peer, 15, &[]);
    receive_code(&mut peer, 16);
    peer
}
fn offer(peer: &mut TcpStream, port: u16, path: &str, size: usize, token: u32) -> (TcpStream, u64) {
    let mut body = 1_u32.to_le_bytes().to_vec();
    body.extend(token.to_le_bytes());
    body.extend(string(path));
    body.extend((size as u64).to_le_bytes());
    send(peer, 40, &body);
    let accepted = receive_code(peer, 41);
    assert_eq!(&accepted[..4], token.to_le_bytes());
    assert_eq!(accepted[4], 1);
    let mut file = TcpStream::connect(("127.0.0.1", port)).unwrap();
    init_peer(&mut file, "F");
    file.write_all(&token.to_le_bytes()).unwrap();
    let mut offset = [0; 8];
    file.read_exact(&mut offset).unwrap();
    (file, u64::from_le_bytes(offset))
}
fn action(h: &engine::EngineHandle, path: &str, action: TransferAction) {
    h.commands
        .send(Command::Transfer {
            username: "fixture-peer".into(),
            path: path.into(),
            action,
        })
        .unwrap();
}
fn wait_size(path: &std::path::Path, size: u64) {
    let end = Instant::now() + Duration::from_secs(5);
    while !std::fs::metadata(path).is_ok_and(|m| m.len() == size) {
        assert!(Instant::now() < end, "partial did not reach {size} bytes");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn partial_downloads_pause_cancel_resume_and_restart_without_changing_bytes_or_totals() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server_port = listener.local_addr().unwrap().port();
    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let peer_port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let server = std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(30)))
                .unwrap();
            while let Ok((code, _)) = receive(&mut stream) {
                if code == 1 {
                    let mut reply = vec![1];
                    reply.extend(string("Transfer controls"));
                    reply.extend([1, 0, 0, 127]);
                    reply.extend(string("checksum"));
                    reply.push(0);
                    send(&mut stream, 1, &reply);
                }
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
    let secrets = || {
        Secrets::new(
            "s".repeat(32),
            "fixture-user".into(),
            "fixture-password".into(),
        )
        .unwrap()
    };
    let h = engine::start(config.clone(), secrets()).unwrap();
    h.commands.send(Command::Connect).unwrap();
    wait_for(&h, |s| s.connection == "connected");
    h.commands
        .send(Command::UpdateRetryPolicy {
            settings: RetryPolicy {
                enabled: false,
                ..Default::default()
            },
        })
        .unwrap();
    wait_for(&h, |s| !s.retry_policy.enabled);
    let mut peer = control(peer_port);
    let payload: Vec<u8> = (0..1024 * 1024).map(|n| (n % 251) as u8).collect();
    // Below the periodic SDK progress threshold: terminal history must still be exact.
    let prefix = 4093;
    let root = std::path::PathBuf::from(&h.snapshot.borrow().locations.downloads);
    let part = |path: &str| {
        root.join(".incomplete")
            .join(rustysoul::downloads::key("fixture-peer", path))
            .join(format!("{}.part", rustysoul::downloads::basename(path)))
    };
    let queue = |h: &engine::EngineHandle, peer: &mut TcpStream, path: &str| {
        h.commands
            .send(Command::Download {
                username: "fixture-peer".into(),
                path: path.into(),
                size: payload.len() as u64,
            })
            .unwrap();
        assert_eq!(receive_code(peer, 43), string(path));
    };
    let path = "Album\\pause.flac";
    queue(&h, &mut peer, path);
    let (mut file, offset) = offer(&mut peer, peer_port, path, payload.len(), 9001);
    assert_eq!(offset, 0);
    file.write_all(&payload[..prefix]).unwrap();
    wait_size(&part(path), prefix as u64);
    action(&h, path, TransferAction::Pause);
    wait_for(&h, |s| {
        s.downloads
            .iter()
            .any(|t| t.path == path && t.status == "paused")
    });
    // One read can already be in flight when pause is requested. After its poll
    // completes, queued peer bytes must stop reaching disk until resume.
    file.write_all(&payload[prefix..prefix + 65536]).unwrap();
    std::thread::sleep(Duration::from_millis(1300));
    let paused = std::fs::metadata(part(path)).unwrap().len();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(std::fs::metadata(part(path)).unwrap().len(), paused);
    action(&h, path, TransferAction::Resume);
    file.write_all(&payload[prefix + 65536..]).unwrap();
    drop(file);
    wait_for(&h, |s| {
        s.downloads
            .iter()
            .any(|t| t.path == path && t.status == "completed")
    });
    {
        let snapshot = h.snapshot.borrow();
        let timing = snapshot.downloads.iter().find(|t| t.path == path).unwrap();
        assert!(timing.started_at.is_some());
        assert!(timing.finished_at >= timing.started_at);
    }
    assert_eq!(
        std::fs::read(root.join("fixture-peer/Album/pause.flac")).unwrap(),
        payload
    );

    let cancelled = "Album\\cancel.flac";
    queue(&h, &mut peer, cancelled);
    let (mut file, _) = offer(&mut peer, peer_port, cancelled, payload.len(), 9002);
    file.write_all(&payload[..prefix]).unwrap();
    wait_size(&part(cancelled), prefix as u64);
    action(&h, cancelled, TransferAction::Pause);
    wait_for(&h, |s| {
        s.downloads
            .iter()
            .any(|t| t.path == cancelled && t.status == "paused")
    });
    action(&h, cancelled, TransferAction::Cancel);
    wait_for(&h, |s| {
        s.downloads
            .iter()
            .any(|t| t.path == cancelled && t.status == "cancelled")
    });
    drop(file);
    let cleanup_deadline = Instant::now() + Duration::from_secs(3);
    while part(cancelled).exists() {
        assert!(
            Instant::now() < cleanup_deadline,
            "Cancelled partial was not removed"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!part(cancelled).exists());
    assert!(!root.join("fixture-peer/Album/cancel.flac").exists());
    let mut late = 1_u32.to_le_bytes().to_vec();
    late.extend(9003_u32.to_le_bytes());
    late.extend(string(cancelled));
    late.extend((payload.len() as u64).to_le_bytes());
    send(&mut peer, 40, &late);
    assert_eq!(receive_code(&mut peer, 41)[4], 0);
    wait_for(&h, |s| {
        s.downloads
            .iter()
            .any(|t| t.path == cancelled && t.status == "cancelled")
    });

    let restart = "Album\\restart.flac";
    queue(&h, &mut peer, restart);
    let (mut file, _) = offer(&mut peer, peer_port, restart, payload.len(), 9004);
    file.write_all(&payload[..prefix]).unwrap();
    wait_size(&part(restart), prefix as u64);
    drop(file);
    wait_for(&h, |s| {
        s.downloads
            .iter()
            .any(|t| t.path == restart && t.status == "failed" && t.transferred == prefix as u64)
    });
    h.stop.send(()).unwrap();
    h.thread.join().unwrap();
    drop(peer);
    assert_eq!(std::fs::read(part(restart)).unwrap(), payload[..prefix]);

    let h = engine::start(config, secrets()).unwrap();
    assert!(!h.snapshot.borrow().retry_policy.enabled);
    assert!(
        h.snapshot
            .borrow()
            .downloads
            .iter()
            .any(|t| t.path == restart && t.status == "failed" && t.transferred == prefix as u64)
    );
    h.commands.send(Command::Connect).unwrap();
    wait_for(&h, |s| s.connection == "connected");
    let mut peer = control(peer_port);
    action(&h, restart, TransferAction::Resume);
    assert_eq!(receive_code(&mut peer, 43), string(restart));
    wait_for(&h, |s| {
        s.downloads
            .iter()
            .any(|t| t.path == restart && t.status == "queued" && t.transferred == prefix as u64)
    });
    let (mut file, offset) = offer(&mut peer, peer_port, restart, payload.len(), 9005);
    assert_eq!(offset, prefix as u64);
    file.write_all(&payload[prefix..]).unwrap();
    drop(file);
    wait_for(&h, |s| {
        s.downloads
            .iter()
            .any(|t| t.path == restart && t.status == "completed")
    });
    assert_eq!(
        std::fs::read(root.join("fixture-peer/Album/restart.flac")).unwrap(),
        payload
    );
    assert!(!part(restart).exists());
    let empty = "Album\\empty.txt";
    h.commands
        .send(Command::Download {
            username: "fixture-peer".into(),
            path: empty.into(),
            size: 0,
        })
        .unwrap();
    assert_eq!(receive_code(&mut peer, 43), string(empty));
    let (mut stream, offset) = offer(&mut peer, peer_port, empty, 0, 9006);
    assert_eq!(offset, 0);
    let mut byte = [0; 1];
    assert_eq!(stream.read(&mut byte).unwrap(), 0);
    wait_for(&h, |s| {
        s.downloads.iter().any(|t| {
            t.path == empty && t.status == "completed" && t.size == 0 && t.transferred == 0
        })
    });
    assert_eq!(
        std::fs::metadata(root.join("fixture-peer/Album/empty.txt"))
            .unwrap()
            .len(),
        0
    );
    assert!(!part(empty).exists());
    h.stop.send(()).unwrap();
    h.thread.join().unwrap();
    drop(peer);
    server.join().unwrap();
    let db = rusqlite::Connection::open(temp.path().join("rustysoul.sqlite3")).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT downloaded FROM traffic_totals WHERE id=1",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        (payload.len() * 2 + prefix) as i64
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM download_retries", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
}
