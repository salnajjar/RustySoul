//! Storage protection over real loopback P/F connections, with exact resume offsets.
use super::*;
#[test]
fn storage_floor_interrupts_and_resumes_a_real_file_without_network_retries() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let peer_port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        while let Ok((code, _)) = receive(&mut stream) {
            if code == 1 {
                let mut reply = vec![1];
                reply.extend(string("Storage fixture"));
                reply.extend([1, 0, 0, 127]);
                reply.extend(string("checksum"));
                reply.push(0);
                send(&mut stream, 1, &reply);
            }
        }
    });
    let temp = tempfile::tempdir().unwrap();
    let mut config = Config {
        data_dir: temp.path().into(),
        ..Default::default()
    };
    config.soulseek.server = "127.0.0.1".into();
    config.soulseek.port = port;
    config.soulseek.listen_port = peer_port;
    config.soulseek.auto_connect = false;
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
    handle.commands.send(Command::Connect).unwrap();
    wait_for(&handle, |s| s.connection == "connected");
    let mut peer = TcpStream::connect(("127.0.0.1", peer_port)).unwrap();
    init_peer(&mut peer, "P");
    send(&mut peer, 15, &[]);
    receive_code(&mut peer, 16);
    let filename = "Album\\storage.flac";
    let payload: Vec<u8> = (0..2 * 1024 * 1024).map(|n| (n % 251) as u8).collect();
    handle
        .commands
        .send(Command::Download {
            username: "fixture-peer".into(),
            path: filename.into(),
            size: payload.len() as u64,
        })
        .unwrap();
    assert_eq!(receive_code(&mut peer, 43), string(filename));
    fn offer(
        peer: &mut TcpStream,
        port: u16,
        filename: &str,
        size: u64,
        token: u32,
    ) -> (TcpStream, u64) {
        let mut body = 1_u32.to_le_bytes().to_vec();
        body.extend(token.to_le_bytes());
        body.extend(string(filename));
        body.extend(size.to_le_bytes());
        send(peer, 40, &body);
        let allowed = receive_code(peer, 41);
        assert_eq!(allowed[4], 1);
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        init_peer(&mut stream, "F");
        stream.write_all(&token.to_le_bytes()).unwrap();
        let mut offset = [0; 8];
        stream.read_exact(&mut offset).unwrap();
        (stream, u64::from_le_bytes(offset))
    }
    let (mut file, offset) = offer(&mut peer, peer_port, filename, payload.len() as u64, 1001);
    assert_eq!(offset, 0);
    let prefix = 1024 * 1024;
    file.write_all(&payload[..prefix]).unwrap();
    let root = handle.snapshot.borrow().locations.downloads.clone();
    let part = std::path::Path::new(&root)
        .join(".incomplete")
        .join(rustysoul::downloads::key("fixture-peer", filename))
        .join("storage.flac.part");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !std::fs::metadata(&part).is_ok_and(|m| m.len() == prefix as u64) {
        assert!(Instant::now() < deadline, "Prefix was not written");
        std::thread::sleep(Duration::from_millis(10));
    }
    handle
        .commands
        .send(Command::UpdateStorageReserve {
            minimum_free_space_mib: rustysoul::storage::MAX_MINIMUM_MIB,
        })
        .unwrap();
    wait_for(&handle, |s| {
        s.network.minimum_free_space_mib == rustysoul::storage::MAX_MINIMUM_MIB
    });
    file.write_all(&payload[prefix..prefix + 65536]).unwrap();
    wait_for(&handle, |s| {
        s.downloads.iter().any(|d| {
            d.path == filename && d.status == "waiting_for_space" && d.transferred == prefix as u64
        })
    });
    drop(file);
    let db = rusqlite::Connection::open(temp.path().join("rustysoul.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM download_retries", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(handle.snapshot.borrow().error.is_none());
    handle
        .commands
        .send(Command::UpdateStorageReserve {
            minimum_free_space_mib: 0,
        })
        .unwrap();
    assert_eq!(receive_code(&mut peer, 43), string(filename));
    let (mut resumed, offset) = offer(&mut peer, peer_port, filename, payload.len() as u64, 1002);
    assert_eq!(offset, prefix as u64);
    resumed.write_all(&payload[prefix..]).unwrap();
    drop(resumed);
    wait_for(&handle, |s| {
        s.downloads
            .iter()
            .any(|d| d.path == filename && d.status == "completed")
    });
    let relative = handle
        .snapshot
        .borrow()
        .downloads
        .iter()
        .find(|d| d.path == filename)
        .unwrap()
        .local_path
        .clone();
    assert_eq!(
        std::fs::read(std::path::Path::new(&root).join(relative)).unwrap(),
        payload
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM download_retries", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    handle.stop.send(()).unwrap();
    handle.thread.join().unwrap();
    server.join().unwrap();
}
