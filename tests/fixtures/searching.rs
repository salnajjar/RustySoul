use super::*;
use rustysoul::model::SearchScope;
fn take_string(bytes: &[u8], at: &mut usize) -> String {
    let len = u32::from_le_bytes(bytes[*at..*at + 4].try_into().unwrap()) as usize;
    *at += 4;
    let value = String::from_utf8(bytes[*at..*at + len].into()).unwrap();
    *at += len;
    value
}
#[test]
fn scoped_searches_have_independent_tokens_and_wishlist_clock_survives_restart() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let reserved = TcpListener::bind("127.0.0.1:0").unwrap();
    let peer_port = reserved.local_addr().unwrap().port();
    drop(reserved);
    let (tx, frames) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        while let Ok((code, payload)) = receive(&mut stream) {
            if code == 1 {
                let mut reply = vec![1];
                reply.extend(string("fixture"));
                reply.extend([127, 0, 0, 1]);
                reply.extend(string("fixture"));
                reply.extend(0_u32.to_le_bytes());
                send(&mut stream, 1, &reply);
                send(&mut stream, 104, &120_u32.to_le_bytes());
            }
            if matches!(code, 26 | 42 | 120 | 103) {
                tx.send((code, payload)).unwrap();
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
        config.clone(),
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
    for scope in [
        SearchScope::Global,
        SearchScope::User {
            username: "fixture-peer".into(),
        },
        SearchScope::Room {
            room: "Music".into(),
        },
    ] {
        handle
            .commands
            .send(Command::SearchScoped {
                query: "same query".into(),
                scope,
            })
            .unwrap();
    }
    let mut tokens = vec![];
    let mut user_token = 0;
    for _ in 0..3 {
        let (code, payload) = frames.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut at = 0;
        if code == 42 {
            assert_eq!(take_string(&payload, &mut at), "fixture-peer");
        }
        if code == 120 {
            assert_eq!(take_string(&payload, &mut at), "Music");
        }
        let token = u32::from_le_bytes(payload[at..at + 4].try_into().unwrap());
        at += 4;
        tokens.push(token);
        if code == 42 {
            user_token = token;
        }
        assert_eq!(take_string(&payload, &mut at), "same query");
    }
    tokens.sort();
    tokens.dedup();
    assert_eq!(tokens.len(), 3);
    assert!(user_token > 0);
    let mut peer = TcpStream::connect(("127.0.0.1", peer_port)).unwrap();
    init_peer(&mut peer, "P");
    let mut payload = string("fixture-peer");
    payload.extend(user_token.to_le_bytes());
    payload.extend(4000_u32.to_le_bytes());
    for i in 0..4000 {
        payload.push(1);
        payload.extend(string(&format!("Music\\Album\\{i:04}.flac")));
        payload.extend((if i == 0 { 0_u64 } else { 100_u64 }).to_le_bytes());
        payload.extend(string("flac"));
        payload.extend(0_u32.to_le_bytes());
    }
    payload.push(1);
    payload.extend(8000_u32.to_le_bytes());
    payload.extend([0; 12]);
    send(&mut peer, 9, &compressed(&payload));
    wait_for(&handle, |s| s.searches.iter().any(|s| s.total == 4000));
    assert_eq!(
        handle
            .snapshot
            .borrow()
            .searches
            .iter()
            .filter(|s| s.total == 4000)
            .count(),
        1
    );
    assert!(
        handle
            .snapshot
            .borrow()
            .searches
            .iter()
            .all(|s| s.results.len() <= 100)
    );
    let repeat_id = handle
        .snapshot
        .borrow()
        .searches
        .iter()
        .find(|s| matches!(s.scope, SearchScope::User { .. }))
        .unwrap()
        .id
        .clone();
    handle
        .commands
        .send(Command::RestartSearch {
            id: repeat_id.clone(),
            query: "edited album".into(),
        })
        .unwrap();
    wait_for(&handle, |s| {
        s.searches
            .iter()
            .any(|s| s.id == repeat_id && s.query == "edited album" && s.total == 0 && s.collecting)
    });
    let (code, repeat_payload) = frames.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(code, 42);
    let mut at = 0;
    assert_eq!(take_string(&repeat_payload, &mut at), "fixture-peer");
    let repeated_token = u32::from_le_bytes(repeat_payload[at..at + 4].try_into().unwrap());
    at += 4;
    assert!(!tokens.contains(&repeated_token));
    assert_eq!(take_string(&repeat_payload, &mut at), "edited album");
    // Old-token results cannot repopulate the repeated search.
    send(&mut peer, 9, &compressed(&payload));
    let token_at = string("fixture-peer").len();
    payload[token_at..token_at + 4].copy_from_slice(&repeated_token.to_le_bytes());
    send(&mut peer, 9, &compressed(&payload));
    wait_for(&handle, |s| {
        s.searches
            .iter()
            .any(|s| s.id == repeat_id && s.total == 4000)
    });
    handle
        .commands
        .send(Command::AddWish {
            query: "rare album".into(),
        })
        .unwrap();
    wait_for(&handle, |s| {
        s.wishlist.first().is_some_and(|w| w.last_run.is_some())
    });
    let wish = handle.snapshot.borrow().wishlist[0].clone();
    assert_eq!(frames.recv_timeout(Duration::from_secs(5)).unwrap().0, 103);
    handle
        .commands
        .send(Command::UpdateWish {
            id: wish.id.clone(),
            enabled: false,
        })
        .unwrap();
    wait_for(&handle, |s| {
        !s.wishlist[0].enabled
            && !s
                .searches
                .iter()
                .any(|s| s.wishlist_id.is_some() && s.collecting)
    });
    handle
        .commands
        .send(Command::RunWish {
            id: wish.id.clone(),
        })
        .unwrap();
    wait_for(&handle, |s| {
        s.error.as_ref().is_some_and(|e| e.contains("rate limit"))
    });
    assert!(frames.try_recv().is_err());
    drop(peer);
    handle.commands.send(Command::Disconnect).unwrap();
    wait_for(&handle, |s| s.connection == "disconnected");
    handle.stop.send(()).unwrap();
    handle.thread.join().unwrap();
    server.join().unwrap();
    let restored = engine::start(
        config,
        Secrets::new(
            "s".repeat(32),
            "fixture-user".into(),
            "fixture-password".into(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        restored.snapshot.borrow().wishlist[0].last_run,
        wish.last_run
    );
    assert!(!restored.snapshot.borrow().wishlist[0].enabled);
    assert_eq!(
        restored
            .snapshot
            .borrow()
            .searches
            .iter()
            .map(|s| s.total)
            .sum::<usize>(),
        4000
    );
    restored.stop.send(()).unwrap();
    restored.thread.join().unwrap();
}
