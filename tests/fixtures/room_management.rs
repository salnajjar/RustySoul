//! Server frames are independently encoded from Nicotine+ codes 14/64/113-148.
use super::*;
use rustysoul::rooms::Action;
fn strings(values: &[&str]) -> Vec<u8> {
    let mut b = (values.len() as u32).to_le_bytes().to_vec();
    for value in values {
        b.extend(string(value));
    }
    b
}
fn room_list(stream: &mut TcpStream, created: bool, owned: bool, operator: bool) {
    let mut b = strings(&["Public"]);
    b.extend(1u32.to_le_bytes());
    b.extend(5u32.to_le_bytes());
    b.extend(strings(if created && owned { &["club"] } else { &[] }));
    b.extend(u32::from(created && owned).to_le_bytes());
    if created && owned {
        b.extend(1u32.to_le_bytes());
    }
    let members = if created && !owned {
        vec!["Other", "club"]
    } else {
        vec!["Other"]
    };
    b.extend(strings(&members));
    b.extend((members.len() as u32).to_le_bytes());
    for _ in &members {
        b.extend(2u32.to_le_bytes());
    }
    b.extend(strings(if operator { &["Other"] } else { &[] }));
    send(stream, 64, &b);
}
fn join(stream: &mut TcpStream, room: &str, owner: &str, operators: &[&str], private: bool) {
    let mut b = string(room);
    for _ in 0..5 {
        b.extend(0u32.to_le_bytes());
    }
    if private {
        b.extend(string(owner));
        b.extend(strings(operators));
    }
    send(stream, 14, &b);
    if private {
        let mut members = string(room);
        members.extend(strings(&["fixture-self", "alice", "bob"]));
        send(stream, 133, &members);
    }
    let mut board = string(room);
    board.extend(1u32.to_le_bytes());
    board.extend(string("alice"));
    board.extend(string("<script>literal board</script>"));
    send(stream, 113, &board);
}
#[test]
fn private_room_controls_board_public_fallback_and_membership_gated_restart_use_real_frames() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let reserve = TcpListener::bind("127.0.0.1:0").unwrap();
    let peer_port = reserve.local_addr().unwrap().port();
    drop(reserve);
    let (tx, frames) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let mut created = false;
        let mut owned = true;
        let mut operator = true;
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            while let Ok((code, payload)) = receive(&mut stream) {
                if matches!(
                    code,
                    14 | 15 | 116 | 134 | 135 | 136 | 137 | 141 | 143 | 144 | 147
                ) {
                    tx.send((code, payload.clone())).unwrap();
                }
                match code {
                    1 => {
                        let mut b = vec![1];
                        b.extend(string("fixture"));
                        b.extend([127, 0, 0, 1]);
                        b.extend(string("fixture"));
                        b.extend(0u32.to_le_bytes());
                        send(&mut stream, 1, &b);
                    }
                    64 => room_list(&mut stream, created, owned, operator),
                    141 => send(&mut stream, 141, &payload),
                    14 => {
                        let length = u32::from_le_bytes(payload[..4].try_into().unwrap()) as usize;
                        let room = std::str::from_utf8(&payload[4..4 + length]).unwrap();
                        let flag = u32::from_le_bytes(payload[4 + length..].try_into().unwrap());
                        if room == "taken" {
                            send(&mut stream, 1003, &string(room));
                            continue;
                        }
                        if room == "public-fallback" {
                            assert_eq!(flag, 1);
                            join(&mut stream, room, "", &[], false);
                            continue;
                        }
                        if room == "club" {
                            if !created {
                                assert_eq!(flag, 1);
                                created = true;
                            } else {
                                assert_eq!(flag, 0);
                            }
                            join(
                                &mut stream,
                                room,
                                if owned { "fixture-self" } else { "host" },
                                &[],
                                true,
                            );
                        } else {
                            assert_eq!(room, "Other");
                            assert_eq!(flag, 0);
                            join(
                                &mut stream,
                                room,
                                "host",
                                if operator {
                                    &["fixture-self", "alice"]
                                } else {
                                    &["alice"]
                                },
                                true,
                            );
                        }
                    }
                    116 => {
                        let n = u32::from_le_bytes(payload[..4].try_into().unwrap()) as usize;
                        let room = std::str::from_utf8(&payload[4..4 + n]).unwrap();
                        let mut b = string(room);
                        b.extend(string("fixture-self"));
                        b.extend(&payload[4 + n..]);
                        send(&mut stream, 114, &b);
                    }
                    134 | 135 | 143 | 144 => send(&mut stream, code, &payload),
                    147 => {
                        operator = false;
                        send(&mut stream, 146, &payload);
                    }
                    137 => owned = false,
                    136 => send(&mut stream, 140, &payload),
                    15 => send(&mut stream, 15, &payload),
                    _ => {}
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
    let secrets = || {
        Secrets::new(
            "s".repeat(32),
            "fixture-self".into(),
            "fixture-password".into(),
        )
        .unwrap()
    };
    let handle = engine::start(config.clone(), secrets()).unwrap();
    handle.commands.send(Command::Connect).unwrap();
    wait_for(&handle, |s| {
        s.connection == "connected" && s.room_management.invitations_confirmed == Some(false)
    });
    let expect = |code: u32| loop {
        let (got, payload) = frames.recv_timeout(Duration::from_secs(5)).unwrap();
        if code == got {
            break payload;
        }
    };
    assert_eq!(expect(141), vec![0]);
    handle
        .commands
        .send(Command::SetRoomInvitations { enabled: true })
        .unwrap();
    wait_for(&handle, |s| {
        s.room_management.invitations_confirmed == Some(true)
    });
    assert_eq!(expect(141), vec![1]);
    handle
        .commands
        .send(Command::CreatePrivateRoom {
            room: "club".into(),
        })
        .unwrap();
    wait_for(&handle, |s| {
        s.joined_rooms.contains(&"club".into()) && s.room_management.rooms["club"].members_known
    });
    let mut requested = string("club");
    requested.extend(1u32.to_le_bytes());
    assert_eq!(expect(14), requested);
    assert_eq!(
        handle.snapshot.borrow().room_management.rooms["club"]
            .owner
            .as_deref(),
        Some("fixture-self")
    );
    handle
        .commands
        .send(Command::SetRoomTicker {
            room: "club".into(),
            text: "Saved board message".into(),
        })
        .unwrap();
    wait_for(&handle, |s| {
        s.room_management.rooms["club"].own_ticker == "Saved board message"
            && s.room_management.rooms["club"].pending.is_none()
    });
    let mut ticker = string("club");
    ticker.extend(string("Saved board message"));
    assert_eq!(expect(116), ticker);
    for (action, code) in [
        (Action::AddMember, 134),
        (Action::AddOperator, 143),
        (Action::RemoveOperator, 144),
        (Action::RemoveMember, 135),
    ] {
        let revision = handle.snapshot.borrow().room_management.rooms["club"].revision;
        handle
            .commands
            .send(Command::PrivateRoomAction {
                room: "club".into(),
                action,
                username: Some("carol".into()),
            })
            .unwrap();
        let mut payload = string("club");
        payload.extend(string("carol"));
        assert_eq!(expect(code), payload);
        wait_for(&handle, |s| {
            s.room_management.rooms["club"].revision > revision
                && s.room_management.rooms["club"].pending.is_none()
        });
    }
    handle
        .commands
        .send(Command::JoinRoom {
            room: "Other".into(),
        })
        .unwrap();
    wait_for(&handle, |s| {
        s.joined_rooms.contains(&"Other".into()) && s.room_management.rooms["Other"].operators_known
    });
    let _ = expect(14);
    handle
        .commands
        .send(Command::PrivateRoomAction {
            room: "Other".into(),
            action: Action::RemoveMember,
            username: Some("alice".into()),
        })
        .unwrap();
    wait_for(&handle, |s| {
        s.error
            .as_deref()
            .is_some_and(|e| e.contains("cannot remove other operators"))
    });
    assert!(frames.recv_timeout(Duration::from_millis(100)).is_err());
    handle
        .commands
        .send(Command::PrivateRoomAction {
            room: "Other".into(),
            action: Action::ResignOperator,
            username: None,
        })
        .unwrap();
    assert_eq!(expect(147), string("Other"));
    wait_for(&handle, |s| {
        !s.room_management.rooms["Other"].operator
            && s.room_management.rooms["Other"].pending.is_none()
    });
    handle
        .commands
        .send(Command::PrivateRoomAction {
            room: "club".into(),
            action: Action::RelinquishOwnership,
            username: None,
        })
        .unwrap();
    assert_eq!(expect(137), string("club"));
    wait_for(&handle, |s| {
        s.room_management.rooms["club"].owner.is_none()
            && s.room_management.rooms["club"].pending.is_none()
    });
    handle
        .commands
        .send(Command::PrivateRoomAction {
            room: "club".into(),
            action: Action::LeaveMembership,
            username: None,
        })
        .unwrap();
    assert_eq!(expect(136), string("club"));
    wait_for(&handle, |s| !s.subscribed_rooms.contains(&"club".into()));
    handle
        .commands
        .send(Command::CreatePrivateRoom {
            room: "taken".into(),
        })
        .unwrap();
    let _ = expect(14);
    wait_for(&handle, |s| {
        s.room_management
            .rooms
            .get("taken")
            .is_some_and(|r| r.error.is_some())
            && !s.subscribed_rooms.contains(&"taken".into())
    });
    handle
        .commands
        .send(Command::CreatePrivateRoom {
            room: "public-fallback".into(),
        })
        .unwrap();
    let _ = expect(14);
    assert_eq!(expect(15), string("public-fallback"));
    wait_for(&handle, |s| {
        !s.subscribed_rooms.contains(&"public-fallback".into())
            && s.room_management
                .rooms
                .get("public-fallback")
                .is_some_and(|r| r.error.is_some())
    });
    handle
        .commands
        .send(Command::SetRoomTicker {
            room: "Other".into(),
            text: "Persistent board".into(),
        })
        .unwrap();
    let _ = expect(116);
    wait_for(&handle, |s| {
        s.room_management.rooms["Other"].pending.is_none()
    });
    handle.stop.send(()).unwrap();
    handle.thread.join().unwrap();
    let restored = engine::start(config, secrets()).unwrap();
    assert!(!restored.snapshot.borrow().room_management.rooms["Other"].confirmed);
    assert_eq!(
        restored.snapshot.borrow().room_management.rooms["Other"].own_ticker,
        "Persistent board"
    );
    restored.commands.send(Command::Connect).unwrap();
    wait_for(&restored, |s| {
        s.joined_rooms.contains(&"Other".into()) && s.room_management.rooms["Other"].confirmed
    });
    assert_eq!(expect(141), vec![1]);
    let mut requested = string("Other");
    requested.extend(0u32.to_le_bytes());
    assert_eq!(expect(14), requested);
    let mut published = string("Other");
    published.extend(string("Persistent board"));
    assert_eq!(expect(116), published);
    restored.stop.send(()).unwrap();
    restored.thread.join().unwrap();
    server.join().unwrap();
}
