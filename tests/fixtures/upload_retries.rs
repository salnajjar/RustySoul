//! Re-offers over real P/F sockets, using only a local fake server and recipient.
use super::*;
#[test]
fn failed_upload_waits_offline_then_reoffers_once_and_resumes_exact_payload() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server_port = listener.local_addr().unwrap().port();
    let reserve = TcpListener::bind("127.0.0.1:0").unwrap();
    let peer_port = reserve.local_addr().unwrap().port();
    drop(reserve);
    let file_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    file_listener.set_nonblocking(true).unwrap();
    let file_port = file_listener.local_addr().unwrap().port();
    let (ready, receiver) = mpsc::channel();
    let online = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let status = online.clone();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(25)))
            .unwrap();
        let writer = std::sync::Arc::new(std::sync::Mutex::new(stream.try_clone().unwrap()));
        ready.send(writer.clone()).unwrap();
        while let Ok((code, body)) = receive(&mut stream) {
            let mut output = writer.lock().unwrap();
            match code {
                1 => {
                    let mut reply = vec![1];
                    reply.extend(string("Retry fixture"));
                    reply.extend([1, 0, 0, 127]);
                    reply.extend(string("checksum"));
                    reply.push(0);
                    send(&mut output, 1, &reply);
                }
                3 => {
                    let mut reply = body;
                    reply.extend([1, 0, 0, 127]);
                    reply.extend(u32::from(file_port).to_le_bytes());
                    send(&mut output, 3, &reply);
                }
                5 => {
                    let mut reply = body;
                    reply.push(1);
                    for value in [
                        if status.load(std::sync::atomic::Ordering::Acquire) {
                            2_u32
                        } else {
                            0
                        },
                        8192,
                        0,
                        0,
                        100,
                        10,
                    ] {
                        reply.extend(value.to_le_bytes());
                    }
                    send(&mut output, 5, &reply);
                }
                _ => (),
            }
        }
    });
    let temp = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let public = temp.path().join("public");
    std::fs::create_dir(&public).unwrap();
    let payload: Vec<u8> = (0..8192).map(|i| (i % 251) as u8).collect();
    std::fs::write(public.join("resume.flac"), &payload).unwrap();
    let mut config = Config {
        data_dir: temp.path().into(),
        ..Default::default()
    };
    config.soulseek.server = "127.0.0.1".into();
    config.soulseek.port = server_port;
    config.soulseek.listen_port = peer_port;
    config.soulseek.auto_connect = false;
    config.soulseek.shares.push(public);
    rustysoul::store::Store::open(&temp.path().join("rustysoul.sqlite3"))
        .unwrap()
        .save_library_settings(&rustysoul::model::LibrarySettings {
            index_on_startup: true,
        })
        .unwrap();
    let handle = engine::start(
        config,
        Secrets::new(
            "s".repeat(32),
            "fixture-user".into(),
            "fixture-password".into(),
        )
        .unwrap(),
    )
    .unwrap();
    handle
        .commands
        .send(Command::UpdateRetryPolicy {
            settings: rustysoul::model::RetryPolicy {
                base_delay_seconds: 5,
                max_attempts: 2,
                ..Default::default()
            },
        })
        .unwrap();
    handle.commands.send(Command::Connect).unwrap();
    wait_for(&handle, |s| {
        s.connection == "connected" && s.library.indexed == 1 && !s.library.scanning
    });
    let writer = receiver.recv_timeout(Duration::from_secs(3)).unwrap();
    let mut peer = TcpStream::connect(("127.0.0.1", peer_port)).unwrap();
    init_peer(&mut peer, "P");
    send(&mut peer, 15, &[]);
    receive_code(&mut peer, 16);
    let path = "public\\resume.flac";
    send(&mut peer, 43, &string(path));
    let offer = receive_code(&mut peer, 40);
    let mut decline = offer[4..8].to_vec();
    decline.push(0);
    decline.extend(string("Busy"));
    send(&mut peer, 41, &decline);
    wait_for(&handle, |s| {
        s.uploads
            .iter()
            .any(|u| u.path == path && u.status == "retry_waiting")
    });
    online.store(false, std::sync::atomic::Ordering::Release);
    send(
        &mut writer.lock().unwrap(),
        7,
        &[
            string("fixture-peer"),
            0_u32.to_le_bytes().to_vec(),
            0_u32.to_le_bytes().to_vec(),
        ]
        .concat(),
    );
    wait_for(&handle, |s| {
        s.uploads.iter().any(|u| {
            u.path == path && u.status == "retry_waiting" && u.detail.contains("no upload slot")
        })
    });
    assert!(
        file_listener.accept().is_err(),
        "No payload connection is allowed before the recipient accepts"
    );
    online.store(true, std::sync::atomic::Ordering::Release);
    send(
        &mut writer.lock().unwrap(),
        7,
        &[
            string("fixture-peer"),
            2_u32.to_le_bytes().to_vec(),
            0_u32.to_le_bytes().to_vec(),
        ]
        .concat(),
    );
    // Bring the fixture's durable timer forward instead of sleeping for its 30s offline poll.
    wait_for(&handle, |s| {
        s.users
            .get("fixture-peer")
            .is_some_and(|u| u.status == "online")
    });
    let db = rusqlite::Connection::open(temp.path().join("rustysoul.sqlite3")).unwrap();
    db.busy_timeout(Duration::from_secs(5)).unwrap();
    db.execute("UPDATE upload_retries SET next_at=strftime('%s','now')", [])
        .unwrap();
    let retry = receive_code(&mut peer, 40);
    assert_ne!(&retry[4..8], &offer[4..8]);
    assert_eq!(&retry[8..8 + string(path).len()], string(path));
    let mut accepted = retry[4..8].to_vec();
    accepted.push(1);
    send(&mut peer, 41, &accepted);
    let end = Instant::now() + Duration::from_secs(5);
    let mut file = loop {
        match file_listener.accept() {
            Ok((file, _)) => break file,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < end);
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("{e}"),
        }
    };
    file.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let mut count = [0; 4];
    file.read_exact(&mut count).unwrap();
    let mut init = vec![0; u32::from_le_bytes(count) as usize];
    file.read_exact(&mut init).unwrap();
    assert_eq!(init[0], 1);
    file.read_exact(&mut count).unwrap();
    assert_eq!(count, retry[4..8]);
    file.write_all(&137_u64.to_le_bytes()).unwrap();
    let mut actual = Vec::new();
    file.read_to_end(&mut actual).unwrap();
    assert_eq!(actual, payload[137..]);
    wait_for(&handle, |s| {
        s.uploads
            .iter()
            .any(|u| u.path == path && u.status == "completed")
    });
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM upload_retries", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    wait_for(&handle, |s| {
        s.traffic.uploaded == (payload.len() - 137) as u64
    });
    assert_eq!(
        handle.snapshot.borrow().traffic.uploaded,
        (payload.len() - 137) as u64
    );
    assert!(handle.snapshot.borrow().error.is_none());
    drop(peer);
    drop(file);
    drop(writer);
    handle.stop.send(()).unwrap();
    handle.thread.join().unwrap();
    server.join().unwrap();
}
