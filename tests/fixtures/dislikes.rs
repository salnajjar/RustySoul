//! Shared dislikes are confirmed by public code-57 lists, never inferred from code 112 alone.
use super::*;
#[test]
fn discovery_verifies_public_dislikes_and_keeps_like_only_users_out_of_the_cohort() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let reserve = TcpListener::bind("127.0.0.1:0").unwrap();
    let peer_port = reserve.local_addr().unwrap().port();
    drop(reserve);
    let (sent, frames) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        while let Ok((code, payload)) = receive(&mut stream) {
            match code {
                1 => {
                    let mut p = vec![1];
                    p.extend(string("fixture"));
                    p.extend([127, 0, 0, 1]);
                    p.extend(string("fixture"));
                    p.extend(0u32.to_le_bytes());
                    send(&mut stream, 1, &p);
                }
                110 => {
                    let mut p = 2u32.to_le_bytes().to_vec();
                    for user in ["alice", "bob"] {
                        p.extend(string(user));
                        p.extend(1u32.to_le_bytes());
                    }
                    send(&mut stream, 110, &p);
                }
                112 => {
                    assert_eq!(payload, string("noise"));
                    let mut p = string("noise");
                    p.extend(3u32.to_le_bytes());
                    for user in ["alice", "bob", "liker"] {
                        p.extend(string(user));
                    }
                    send(&mut stream, 112, &p);
                }
                57 => {
                    let user = if payload == string("alice") {
                        "alice"
                    } else if payload == string("bob") {
                        "bob"
                    } else {
                        "liker"
                    };
                    let mut p = string(user);
                    p.extend(1u32.to_le_bytes());
                    p.extend(string(if user == "liker" { "noise" } else { "jazz" }));
                    p.extend((if user == "liker" { 0u32 } else { 2u32 }).to_le_bytes());
                    if user != "liker" {
                        p.extend(string("noise"));
                        p.extend(string("spam"));
                    }
                    send(&mut stream, 57, &p);
                }
                51 | 117 => {
                    sent.send((code, payload)).unwrap();
                }
                _ => {}
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
    handle
        .commands
        .send(Command::SetInterest {
            item: "noise".into(),
            disposition: rustysoul::model::InterestDisposition::Dislike,
        })
        .unwrap();
    wait_for(&handle, |s| s.profile.dislikes == ["noise"]);
    handle.commands.send(Command::Connect).unwrap();
    wait_for(&handle, |s| s.connection == "connected");
    assert_eq!(
        frames.recv_timeout(Duration::from_secs(3)).unwrap(),
        (117, string("noise"))
    );
    handle
        .commands
        .send(Command::Discover {
            query: rustysoul::discovery::Query::Dislikes {
                item: "NOISE".into(),
            },
            refresh: true,
        })
        .unwrap();
    wait_for(&handle, |s| {
        !s.discovery.pending()
            && s.discovery
                .dislike_evidence
                .as_ref()
                .is_some_and(|e| e.checked == 3)
    });
    {
        let s = handle.snapshot.borrow();
        assert_eq!(
            s.discovery
                .people
                .iter()
                .map(|p| p.username.as_str())
                .collect::<Vec<_>>(),
            ["alice", "bob"]
        );
        assert_eq!(s.discovery.against[0].item, "spam");
        assert_eq!(s.discovery.against[0].rating, -2);
        assert_eq!(s.discovery.recommendations[0].item, "jazz");
        assert_eq!(s.discovery.recommendations[0].rating, 2);
        assert_eq!(s.profile.dislikes, ["noise"]);
        assert!(s.profile.likes.is_empty());
    }
    assert!(
        frames.try_recv().is_err(),
        "discovery must not temporarily advertise a disliked item as a like"
    );
    handle.stop.send(()).unwrap();
    handle.thread.join().unwrap();
    server.join().unwrap();
    let restored = engine::start(config, secrets()).unwrap();
    assert_eq!(restored.snapshot.borrow().discovery.people.len(), 2);
    restored
        .commands
        .send(Command::Discover {
            query: rustysoul::discovery::Query::Dislikes {
                item: "spam".into(),
            },
            refresh: true,
        })
        .unwrap();
    wait_for(&restored, |s| {
        s.discovery.query
            == rustysoul::discovery::Query::Dislikes {
                item: "spam".into(),
            }
            && s.discovery.people.len() == 2
    });
    restored.stop.send(()).unwrap();
    restored.thread.join().unwrap();
}
