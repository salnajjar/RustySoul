//! Independent Soulseek CheckPrivileges request/reply (server code 92).
use super::*;
#[test]
fn own_privileges_are_checked_on_login_updated_and_reset_across_accounts() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let reserved = TcpListener::bind("127.0.0.1:0").unwrap();
    let peer_port = reserved.local_addr().unwrap().port();
    drop(reserved);
    let (sent, frames) = mpsc::channel();
    let (updates, changes) = mpsc::channel::<u32>();
    let server = std::thread::spawn(move || {
        for connection in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_millis(200)))
                .unwrap();
            loop {
                if let Ok(seconds) = changes.try_recv() {
                    send(&mut stream, 92, &seconds.to_le_bytes());
                }
                match receive(&mut stream) {
                    Ok((1, _)) => {
                        let mut reply = vec![1];
                        reply.extend(string("fixture"));
                        reply.extend([127, 0, 0, 1]);
                        reply.extend(string("fixture"));
                        reply.extend(0u32.to_le_bytes());
                        send(&mut stream, 1, &reply);
                    }
                    Ok((92, payload)) => {
                        assert!(payload.is_empty());
                        sent.send(connection).unwrap();
                        let seconds = if connection == 0 { 0u32 } else { 7200u32 };
                        send(&mut stream, 92, &seconds.to_le_bytes());
                    }
                    Ok(_) => {}
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) => {}
                    Err(_) => break,
                }
            }
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config {
        data_dir: dir.path().into(),
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
            "fixture-self".into(),
            "fixture-password".into(),
        )
        .unwrap(),
    )
    .unwrap();
    handle.commands.send(Command::Connect).unwrap();
    wait_for(&handle, |s| s.privileges.remaining_seconds == Some(0));
    assert_eq!(frames.recv_timeout(Duration::from_secs(3)).unwrap(), 0);
    assert!(!handle.snapshot.borrow().privileges.official_server);
    handle
        .commands
        .send(Command::CheckPrivileges {
            watch_for_activation: true,
        })
        .unwrap();
    wait_for(&handle, |s| s.privileges.watching);
    updates.send(86400).unwrap();
    wait_for(&handle, |s| {
        s.privileges.remaining_seconds.is_some_and(|v| v > 86000)
    });
    {
        let s = handle.snapshot.borrow();
        assert!(!s.privileges.watching);
        assert!(!s.privileges.pending);
        assert!(s.privileges.expires_at > s.privileges.checked_at);
    }
    handle.commands.send(Command::Disconnect).unwrap();
    wait_for(&handle, |s| {
        s.connection == "disconnected" && s.privileges.remaining_seconds.is_none()
    });
    handle
        .commands
        .send(Command::UpdateCredentials {
            username: "other-account".into(),
            password: Some("other-password".into()),
        })
        .unwrap();
    wait_for(&handle, |s| {
        s.username == "other-account"
            && s.privileges
                .remaining_seconds
                .is_some_and(|v| v <= 7200 && v > 7100)
    });
    assert_eq!(frames.recv_timeout(Duration::from_secs(3)).unwrap(), 1);
    handle.stop.send(()).unwrap();
    handle.thread.join().unwrap();
    server.join().unwrap();
}
