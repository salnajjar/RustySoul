//! Empty shares complete the normal offset handshake without payload or waiting for EOF.
use super::*;
#[test]
fn an_empty_local_share_is_indexed_browsed_and_uploaded_without_payload() {
    let server_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server_port = server_listener.local_addr().unwrap().port();
    let file_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let file_port = file_listener.local_addr().unwrap().port();
    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let peer_port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let server = std::thread::spawn(move || {
        let (mut s, _) = server_listener.accept().unwrap();
        while let Ok((code, body)) = receive(&mut s) {
            if code == 1 {
                let mut reply = vec![1];
                reply.extend(string("Empty fixture"));
                reply.extend([1, 0, 0, 127]);
                reply.extend(string("checksum"));
                reply.push(0);
                send(&mut s, 1, &reply);
            }
            if code == 3 {
                let mut reply = body.clone();
                reply.extend([1, 0, 0, 127]);
                reply.extend(u32::from(file_port).to_le_bytes());
                send(&mut s, 3, &reply);
            }
            if code == 5 {
                let mut reply = body.clone();
                reply.push(1);
                reply.extend(2_u32.to_le_bytes());
                for n in [1024_u32, 0, 0, 10, 2] {
                    reply.extend(n.to_le_bytes());
                }
                send(&mut s, 5, &reply);
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
    std::fs::write(public.join("empty.txt"), b"").unwrap();
    let mut config = Config {
        data_dir: temp.path().into(),
        ..Default::default()
    };
    config.soulseek.server = "127.0.0.1".into();
    config.soulseek.port = server_port;
    config.soulseek.listen_port = peer_port;
    config.soulseek.auto_connect = false;
    config.soulseek.shares = vec![public.clone()];
    let h = engine::start(
        config,
        Secrets::new("s".repeat(32), "fixture-user".into(), "password".into()).unwrap(),
    )
    .unwrap();
    h.commands.send(Command::RescanShares).unwrap();
    wait_for(&h, |s| s.library.indexed == 1 && !s.library.scanning);
    h.commands
        .send(Command::Browse {
            username: "fixture-user".into(),
        })
        .unwrap();
    wait_for(&h, |s| {
        s.browse
            .get("fixture-user")
            .is_some_and(|v| v.total == 1 && v.files[0].size == 0)
    });
    h.commands.send(Command::Connect).unwrap();
    wait_for(&h, |s| s.connection == "connected");
    let mut p = TcpStream::connect(("127.0.0.1", peer_port)).unwrap();
    init_peer(&mut p, "P");
    send(&mut p, 15, &[]);
    receive_code(&mut p, 16);
    send(&mut p, 43, &string("public\\empty.txt"));
    let offer = receive_code(&mut p, 40);
    assert_eq!(
        &offer[8..8 + string("public\\empty.txt").len()],
        string("public\\empty.txt")
    );
    assert_eq!(
        u64::from_le_bytes(offer[offer.len() - 8..].try_into().unwrap()),
        0
    );
    let token = u32::from_le_bytes(offer[4..8].try_into().unwrap());
    let mut accept = token.to_le_bytes().to_vec();
    accept.push(1);
    send(&mut p, 41, &accept);
    file_listener.set_nonblocking(true).unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    let mut f = loop {
        match file_listener.accept() {
            Ok((f, _)) => break f,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < end);
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("{e}"),
        }
    };
    f.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let mut len = [0; 4];
    f.read_exact(&mut len).unwrap();
    let mut init = vec![0; u32::from_le_bytes(len) as usize];
    f.read_exact(&mut init).unwrap();
    f.read_exact(&mut len).unwrap();
    assert_eq!(u32::from_le_bytes(len), token);
    f.write_all(&0_u64.to_le_bytes()).unwrap();
    let mut payload = vec![];
    f.read_to_end(&mut payload).unwrap();
    assert!(payload.is_empty());
    wait_for(&h, |s| {
        s.uploads.iter().any(|t| {
            t.path == "public\\empty.txt"
                && t.status == "completed"
                && t.size == 0
                && t.transferred == 0
        })
    });
    assert_eq!(
        std::fs::metadata(public.join("empty.txt")).unwrap().len(),
        0
    );
    assert_eq!(
        (
            h.snapshot.borrow().traffic.uploaded,
            h.snapshot.borrow().traffic.downloaded
        ),
        (0, 0)
    );
    h.stop.send(()).unwrap();
    h.thread.join().unwrap();
    drop(p);
    server.join().unwrap();
}
