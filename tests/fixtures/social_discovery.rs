//! Independent server frames from Nicotine+ codes 51-57 and 110-112.
use super::*;
use rustysoul::{discovery::Query, model::InterestDisposition};
fn entries(values: &[(&str, i32)]) -> Vec<u8> {
    let mut data = (values.len() as u32).to_le_bytes().to_vec();
    for (item, rating) in values {
        data.extend(string(item));
        data.extend(rating.to_le_bytes());
    }
    data
}
#[test]
fn recommendations_interests_and_similar_people_use_fresh_wire_replies_and_survive_restart() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let reserved = TcpListener::bind("127.0.0.1:0").unwrap();
    let peer_port = reserved.local_addr().unwrap().port();
    drop(reserved);
    let (sent, frames) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut personal = 0;
        while let Ok((code, payload)) = receive(&mut stream) {
            match code {
                1 => {
                    let mut reply = vec![1];
                    reply.extend(string("fixture"));
                    reply.extend([127, 0, 0, 1]);
                    reply.extend(string("fixture"));
                    reply.extend(0u32.to_le_bytes());
                    send(&mut stream, 1, &reply);
                }
                54 => {
                    personal += 1;
                    let mut reply = if personal == 1 {
                        entries(&[("blues", 8)])
                    } else {
                        entries(&[])
                    };
                    reply.extend(entries(&[("muzak", -2)]));
                    send(&mut stream, 54, &reply);
                }
                56 => {
                    let mut reply = entries(&[("rock", 10)]);
                    reply.extend(entries(&[]));
                    send(&mut stream, 56, &reply);
                }
                110 => {
                    let mut reply = 2u32.to_le_bytes().to_vec();
                    reply.extend(string("alice"));
                    reply.extend(2u32.to_le_bytes());
                    reply.extend(string("bob"));
                    reply.extend(9u32.to_le_bytes());
                    send(&mut stream, 110, &reply);
                }
                111 => {
                    assert_eq!(payload, string("jazz"));
                    let mut reply = string("jazz");
                    reply.extend(entries(&[("soul", 5)]));
                    reply.extend(entries(&[("pop", -4)]));
                    send(&mut stream, 111, &reply);
                }
                112 => {
                    assert_eq!(payload, string("jazz"));
                    let mut reply = string("jazz");
                    reply.extend(1u32.to_le_bytes());
                    reply.extend(string("alice"));
                    send(&mut stream, 112, &reply);
                }
                57 => {
                    assert_eq!(payload, string("alice"));
                    let mut reply = string("alice");
                    reply.extend(1u32.to_le_bytes());
                    reply.extend(string("jazz"));
                    reply.extend(1u32.to_le_bytes());
                    reply.extend(string("muzak"));
                    send(&mut stream, 57, &reply);
                }
                5 => {
                    let mut reply = payload;
                    reply.push(1);
                    reply.extend(0u32.to_le_bytes());
                    reply.extend(1000u32.to_le_bytes());
                    reply.extend(0u64.to_le_bytes());
                    reply.extend(10u32.to_le_bytes());
                    reply.extend(1u32.to_le_bytes());
                    send(&mut stream, 5, &reply);
                }
                51 | 52 | 117 | 118 => {
                    let _ = sent.send((code, payload));
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
    use base64::Engine as _;
    let mut image = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(2, 2)
        .write_to(&mut image, image::ImageFormat::Png)
        .unwrap();
    handle
        .commands
        .send(Command::UpdateProfile {
            settings: rustysoul::profile::Profile {
                description: "Keep this description".into(),
                away_message: "Keep this reply".into(),
                picture: Some(base64::engine::general_purpose::STANDARD.encode(image.into_inner())),
                ..Default::default()
            },
        })
        .unwrap();
    handle
        .commands
        .send(Command::SetInterest {
            item: " Jazz ".into(),
            disposition: InterestDisposition::Like,
        })
        .unwrap();
    wait_for(&handle, |s| s.profile.likes == ["jazz"]);
    let picture = rustysoul::profile::load(
        &rusqlite::Connection::open(dir.path().join("rustysoul.sqlite3")).unwrap(),
    )
    .unwrap()
    .picture;
    assert!(picture.is_some());
    assert_eq!(
        handle.snapshot.borrow().profile.description,
        "Keep this description"
    );
    handle.commands.send(Command::Connect).unwrap();
    wait_for(&handle, |s| s.connection == "connected");
    let (code, payload) = frames.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!((code, payload), (51, string("jazz")));
    let run = |query: Query| {
        handle
            .commands
            .send(Command::Discover {
                query,
                refresh: true,
            })
            .unwrap()
    };
    run(Query::Personal {});
    wait_for(&handle, |s| {
        s.discovery.recommendations_at.is_some() && !s.discovery.pending()
    });
    {
        let s = handle.snapshot.borrow();
        assert_eq!(s.discovery.recommendations[0].item, "blues");
        assert_eq!(s.discovery.against[0].rating, -2);
        assert_eq!(s.discovery.people[0].username, "bob");
    }
    run(Query::Popular {});
    wait_for(&handle, |s| {
        s.discovery.query == Query::Popular {}
            && s.discovery
                .recommendations
                .first()
                .is_some_and(|r| r.item == "rock")
    });
    run(Query::Personal {});
    wait_for(&handle, |s| {
        s.discovery.query == Query::Personal {}
            && s.discovery.recommendations_at.is_some()
            && !s.discovery.pending()
            && s.discovery.recommendations.is_empty()
    });
    run(Query::Topic {
        item: " Jazz ".into(),
    });
    wait_for(&handle, |s| {
        matches!(&s.discovery.query,Query::Topic{item} if item=="jazz")
            && s.discovery.recommendations_at.is_some()
            && !s.discovery.pending()
    });
    {
        let s = handle.snapshot.borrow();
        assert_eq!(s.discovery.recommendations[0].item, "soul");
        assert_eq!(s.discovery.against[0].item, "pop");
        assert_eq!(s.discovery.people[0].weight, None);
    }
    handle
        .commands
        .send(Command::LookupUser {
            username: "alice".into(),
        })
        .unwrap();
    wait_for(&handle, |s| {
        s.users
            .get("alice")
            .is_some_and(|u| u.interests.updated_at > 0 && !u.interests_pending)
    });
    assert_eq!(
        handle.snapshot.borrow().users["alice"].interests.likes,
        ["jazz"]
    );
    handle
        .commands
        .send(Command::SetInterest {
            item: "jazz".into(),
            disposition: InterestDisposition::Dislike,
        })
        .unwrap();
    wait_for(&handle, |s| {
        s.profile.likes.is_empty() && s.profile.dislikes == ["jazz"]
    });
    assert_eq!(
        handle.snapshot.borrow().profile.away_message,
        "Keep this reply"
    );
    assert_eq!(
        rustysoul::profile::load(
            &rusqlite::Connection::open(dir.path().join("rustysoul.sqlite3")).unwrap()
        )
        .unwrap()
        .picture,
        picture
    );
    assert_eq!(
        frames.recv_timeout(Duration::from_secs(5)).unwrap(),
        (52, string("jazz"))
    );
    assert_eq!(
        frames.recv_timeout(Duration::from_secs(5)).unwrap(),
        (117, string("jazz"))
    );
    handle.stop.send(()).unwrap();
    handle.thread.join().unwrap();
    server.join().unwrap();
    let restored = engine::start(config, secrets()).unwrap();
    assert_eq!(
        restored.snapshot.borrow().discovery.recommendations[0].item,
        "soul"
    );
    assert!(!restored.snapshot.borrow().discovery.pending());
    assert_eq!(restored.snapshot.borrow().profile.dislikes, ["jazz"]);
    let db = rusqlite::Connection::open(dir.path().join("rustysoul.sqlite3")).unwrap();
    assert_eq!(
        rustysoul::discovery::load_interests(&db, "alice")
            .unwrap()
            .dislikes,
        ["muzak"]
    );
    restored.stop.send(()).unwrap();
    restored.thread.join().unwrap();
}
