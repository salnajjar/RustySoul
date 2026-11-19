//! Offline wire fixtures. Never connects to the public Soulseek network.
#[path = "fixtures/dislikes.rs"]
mod dislikes;
#[path = "fixtures/download_control.rs"]
mod download_control;
#[path = "fixtures/download_slots.rs"]
mod download_slots;
#[path = "fixtures/empty_files.rs"]
mod empty_files;
#[path = "fixtures/privileges.rs"]
mod privileges;
#[path = "fixtures/room_management.rs"]
mod room_management;
#[path = "fixtures/searching.rs"]
mod searching;
#[path = "fixtures/social_discovery.rs"]
mod social_discovery;
#[path = "fixtures/storage.rs"]
mod storage;
#[path = "fixtures/users.rs"]
mod users;
use rustysoul::{
    config::{Config, Secrets},
    engine,
    model::Command,
};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    time::{Duration, Instant},
};

fn string(value: &str) -> Vec<u8> {
    let mut out = (value.len() as u32).to_le_bytes().to_vec();
    out.extend(value.as_bytes());
    out
}
fn send(stream: &mut TcpStream, code: u32, payload: &[u8]) {
    stream
        .write_all(&((payload.len() + 4) as u32).to_le_bytes())
        .unwrap();
    stream.write_all(&code.to_le_bytes()).unwrap();
    stream.write_all(payload).unwrap();
}
fn receive(stream: &mut TcpStream) -> std::io::Result<(u32, Vec<u8>)> {
    let mut length = [0; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    assert!((4..=65536).contains(&length));
    let mut body = vec![0; length];
    stream.read_exact(&mut body)?;
    Ok((
        u32::from_le_bytes(body[..4].try_into().unwrap()),
        body[4..].into(),
    ))
}

fn init_peer(stream: &mut TcpStream, kind: &str) {
    let mut body = vec![1];
    body.extend(string("fixture-peer"));
    body.extend(string(kind));
    body.extend(0_u32.to_le_bytes());
    stream
        .write_all(&(body.len() as u32).to_le_bytes())
        .unwrap();
    stream.write_all(&body).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
}
fn compressed(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}
fn receive_code(stream: &mut TcpStream, expected: u32) -> Vec<u8> {
    loop {
        let (code, payload) = receive(stream).unwrap();
        if code == expected {
            return payload;
        }
    }
}
#[track_caller]
fn wait_for(
    handle: &engine::EngineHandle,
    predicate: impl Fn(&rustysoul::model::Snapshot) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if predicate(&handle.snapshot.borrow()) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Timed out waiting for daemon state: {} {:?}; transfers: {:?}",
            handle.snapshot.borrow().connection,
            handle.snapshot.borrow().error,
            handle
                .snapshot
                .borrow()
                .downloads
                .iter()
                .map(|t| (&t.username, &t.path, &t.status))
                .collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn login_search_messages_and_room_list_use_reference_wire_format() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer_reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let peer_port = peer_reservation.local_addr().unwrap().port();
    drop(peer_reservation);
    let upload_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let upload_port = upload_listener.local_addr().unwrap().port();
    let (seen, frames) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        while let Ok((code, payload)) = receive(&mut stream) {
            if code == 1 {
                assert!(payload.starts_with(&string("fixture-user")));
                let offset = string("fixture-user").len() + string("fixture-password").len();
                assert_eq!(
                    u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap()),
                    177
                );
                let mut response = vec![1];
                response.extend(string("Fixture welcome"));
                response.extend([1, 0, 0, 127]);
                response.extend(string("checksum"));
                response.push(0);
                send(&mut stream, 1, &response);
                let mut message = 42_u32.to_le_bytes().to_vec();
                message.extend(1700000000_u32.to_le_bytes());
                message.extend(string("fixture-peer"));
                message.extend(string("Hello 👋 🌍 <script>not code</script>"));
                message.push(1);
                send(&mut stream, 22, &message);
            }
            if code == 64 {
                let mut rooms = 1_u32.to_le_bytes().to_vec();
                rooms.extend(string("Fixture room"));
                rooms.extend(1_u32.to_le_bytes());
                rooms.extend(42_u32.to_le_bytes());
                for _ in 0..5 {
                    rooms.extend(0_u32.to_le_bytes());
                }
                send(&mut stream, 64, &rooms);
            }
            if code == 5 {
                let mut reply = payload.clone();
                reply.push(1);
                for value in [
                    2_u32,
                    8192,
                    0,
                    0,
                    if payload == string("fixture-leech") {
                        0
                    } else {
                        10
                    },
                    if payload == string("fixture-leech") {
                        0
                    } else {
                        2
                    },
                ] {
                    reply.extend(value.to_le_bytes());
                }
                send(&mut stream, 5, &reply);
            }
            if code == 3 {
                let mut address = string("fixture-peer");
                address.extend([1, 0, 0, 127]);
                address.extend(u32::from(upload_port).to_le_bytes());
                send(&mut stream, 3, &address);
            }
            if code == 14 {
                let length = u32::from_le_bytes(payload[..4].try_into().unwrap()) as usize;
                let mut joined = payload[..4 + length].to_vec();
                for _ in 0..5 {
                    joined.extend(0_u32.to_le_bytes());
                }
                send(&mut stream, 14, &joined);
                let length = u32::from_le_bytes(payload[..4].try_into().unwrap()) as usize;
                let mut room_message = payload[..4 + length].to_vec();
                room_message.extend(string("room-peer"));
                room_message.extend(string("New room activity"));
                send(&mut stream, 13, &room_message);
            }
            if code == 15 {
                send(&mut stream, 15, &payload);
            }
            if code == 13 {
                let mut echo = string("Fixture room");
                echo.extend(string("fixture-user"));
                echo.extend(string("Fixture room message 🎵 👩🏽‍🎤 ❤️ 🇬🇧"));
                send(&mut stream, 13, &echo);
            }
            let _ = seen.send((code, payload));
        }
    });
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut config = Config {
        data_dir: dir.path().into(),
        ..Default::default()
    };
    config.soulseek.server = "127.0.0.1".into();
    config.soulseek.port = port;
    config.soulseek.listen_port = peer_port;
    config.soulseek.auto_connect = false;
    let public = dir.path().join("public");
    std::fs::create_dir(&public).unwrap();
    let shared_content = vec![42_u8; 8192];
    std::fs::write(public.join("upload.txt"), &shared_content).unwrap();
    std::fs::write(public.join("second.txt"), &shared_content).unwrap();
    config.soulseek.shares.push(public);
    rustysoul::store::Store::open(&dir.path().join("rustysoul.sqlite3"))
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
        .send(Command::UpdateProfile {
            settings: rustysoul::profile::Profile {
                description: "Fixture public profile <script>literal</script>".into(),
                likes: vec!["jazz".into()],
                ..Default::default()
            },
        })
        .unwrap();
    wait_for(&handle, |s| s.profile.revision == 1);
    handle.commands.send(Command::Connect).unwrap();
    wait_for(&handle, |s| s.connection == "connected");
    let settings = rustysoul::model::NetworkSettings {
        port: peer_port,
        upload_limit_kib: 8,
        download_limit_kib: 8,
        upload_per_user_limit_kib: 16,
        download_per_user_limit_kib: 16,
        ..Default::default()
    };
    handle
        .commands
        .send(Command::UpdateNetwork {
            settings: settings.clone(),
        })
        .unwrap();
    wait_for(&handle, |s| s.network == settings);
    wait_for(&handle, |s| {
        s.messages.iter().any(|m| m.username == "fixture-peer")
            && s.rooms
                .iter()
                .any(|r| r.name == "Fixture room" && r.users == 42)
    });
    handle
        .commands
        .send(Command::JoinRoom {
            room: "Fixture room".into(),
        })
        .unwrap();
    wait_for(&handle, |s| {
        s.joined_rooms.contains(&"Fixture room".to_string())
    });
    handle
        .commands
        .send(Command::JoinRoom {
            room: "Second room".into(),
        })
        .unwrap();
    wait_for(&handle, |s| {
        s.joined_rooms.len() == 2 && s.room_unread.get("Second room") == Some(&1)
    });
    let through = handle
        .snapshot
        .borrow()
        .messages
        .iter()
        .filter(|m| m.target == "Fixture room" && !m.private)
        .map(|m| m.id)
        .max()
        .unwrap();
    handle
        .commands
        .send(Command::ReadRoom {
            room: "Fixture room".into(),
            through_id: through,
        })
        .unwrap();
    wait_for(&handle, |s| {
        s.room_unread.get("Fixture room").copied().unwrap_or(0) == 0
            && s.room_unread.get("Second room") == Some(&1)
    });
    handle
        .commands
        .send(Command::LeaveRoom {
            room: "Second room".into(),
        })
        .unwrap();
    wait_for(&handle, |s| {
        !s.subscribed_rooms.contains(&"Second room".to_string())
    });
    handle
        .commands
        .send(Command::SendMessage {
            target: "Fixture room".into(),
            text: "Fixture room message 🎵 👩🏽‍🎤 ❤️ 🇬🇧".into(),
            private: false,
        })
        .unwrap();
    wait_for(&handle, |s| {
        s.messages
            .iter()
            .any(|m| !m.private && m.text == "Fixture room message 🎵 👩🏽‍🎤 ❤️ 🇬🇧")
    });
    handle
        .commands
        .send(Command::LeaveRoom {
            room: "Fixture room".into(),
        })
        .unwrap();
    wait_for(&handle, |s| s.joined_rooms.is_empty());
    handle
        .commands
        .send(Command::SendMessage {
            target: "fixture-peer".into(),
            text: "Fixture private message 👍🏾 🎶".into(),
            private: true,
        })
        .unwrap();
    handle
        .commands
        .send(Command::Search {
            query: "reference fixture".into(),
        })
        .unwrap();
    wait_for(&handle, |s| {
        s.searches.iter().any(|s| s.query == "reference fixture")
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut search_token = None;
    let mut sent_private = false;
    let mut sent_interest = false;
    while let Ok((code, payload)) =
        frames.recv_timeout(deadline.saturating_duration_since(Instant::now()))
    {
        if code == 51 && payload == string("jazz") {
            sent_interest = true;
        }
        if code == 22 {
            let mut expected = string("fixture-peer");
            expected.extend(string("Fixture private message 👍🏾 🎶"));
            assert_eq!(payload, expected);
            sent_private = true;
        }
        if code == 26 {
            assert_eq!(&payload[4..], string("reference fixture"));
            search_token = Some(u32::from_le_bytes(payload[..4].try_into().unwrap()));
            break;
        }
    }
    assert!(sent_interest, "Saved likes must publish on network login");
    let token = search_token.expect("Search must reach the server as code 26");
    assert!(
        sent_private,
        "Private message must reach the server with code 22"
    );
    let filename = "Fixture\\track.flac";
    let contents = vec![23_u8; 8192];
    let mut peer = TcpStream::connect(("127.0.0.1", peer_port)).unwrap();
    init_peer(&mut peer, "P");
    send(&mut peer, 15, &[]);
    let info = receive_code(&mut peer, 16);
    assert!(info.starts_with(&string("Fixture public profile <script>literal</script>")));
    let mut result = string("fixture-peer");
    result.extend(token.to_le_bytes());
    result.extend(1_u32.to_le_bytes());
    result.push(1);
    result.extend(string(filename));
    result.extend((contents.len() as u64).to_le_bytes());
    result.extend(string(""));
    result.extend(0_u32.to_le_bytes());
    result.push(1);
    result.extend(200000_u32.to_le_bytes());
    for _ in 0..3 {
        result.extend(0_u32.to_le_bytes());
    }
    send(&mut peer, 9, &compressed(&result));
    wait_for(&handle, |s| {
        s.searches.iter().any(|s| {
            s.results
                .iter()
                .any(|f| f.path == filename && f.size == contents.len() as u64)
        })
    });

    handle
        .commands
        .send(Command::Browse {
            username: "fixture-peer".into(),
        })
        .unwrap();
    receive_code(&mut peer, 4);
    let mut listing = 1_u32.to_le_bytes().to_vec();
    listing.extend(string("Fixture"));
    listing.extend(1_u32.to_le_bytes());
    listing.push(1);
    listing.extend(string("track.flac"));
    listing.extend((contents.len() as u64).to_le_bytes());
    listing.extend(string(""));
    for _ in 0..3 {
        listing.extend(0_u32.to_le_bytes());
    }
    send(&mut peer, 5, &compressed(&listing));
    wait_for(&handle, |s| {
        s.browse
            .get("fixture-peer")
            .is_some_and(|b| !b.pending && b.files.iter().any(|f| f.path == filename))
    });

    handle
        .commands
        .send(Command::UpdateRetryPolicy {
            settings: rustysoul::model::RetryPolicy {
                enabled: true,
                max_attempts: 2,
                base_delay_seconds: 5,
                max_delay_seconds: 10,
            },
        })
        .unwrap();
    wait_for(&handle, |s| s.retry_policy.enabled);
    handle
        .commands
        .send(Command::DownloadFolder {
            username: "fixture-peer".into(),
            folder: "Fixture".into(),
            recursive: true,
        })
        .unwrap();
    assert_eq!(receive_code(&mut peer, 43), string(filename));
    let mut denial = string(filename);
    denial.extend(string("Temporary fixture failure"));
    send(&mut peer, 50, &denial);
    wait_for(&handle, |s| {
        s.downloads
            .iter()
            .any(|d| d.path == filename && d.status == "retry_waiting" && d.retry_at.is_some())
    });
    peer.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    assert_eq!(
        receive_code(&mut peer, 43),
        string(filename),
        "Scheduled retry must request the same exact file again"
    );

    let transfer_token = 123456_u32;
    let mut offer = 1_u32.to_le_bytes().to_vec();
    offer.extend(transfer_token.to_le_bytes());
    offer.extend(string(filename));
    offer.extend((contents.len() as u64).to_le_bytes());
    send(&mut peer, 40, &offer);
    let allowed = receive_code(&mut peer, 41);
    assert_eq!(&allowed[..4], transfer_token.to_le_bytes());
    assert_eq!(allowed[4], 1);
    let mut file_peer = TcpStream::connect(("127.0.0.1", peer_port)).unwrap();
    init_peer(&mut file_peer, "F");
    file_peer.write_all(&transfer_token.to_le_bytes()).unwrap();
    let mut offset = [0; 8];
    file_peer.read_exact(&mut offset).unwrap();
    assert_eq!(u64::from_le_bytes(offset), 0);
    let download_started = Instant::now();
    file_peer.write_all(&contents).unwrap();
    drop(file_peer);
    wait_for(&handle, |s| {
        s.downloads.iter().any(|d| {
            d.path == filename && d.status == "completed" && d.transferred == contents.len() as u64
        })
    });
    assert!(
        download_started.elapsed() >= Duration::from_millis(800),
        "Download payload ignored its configured speed cap"
    );
    let destination = dir.path().join("downloads/fixture-peer/Fixture");
    assert_eq!(
        std::fs::read(destination.join("track.flac")).unwrap(),
        contents
    );
    // A late repeat offer for completed/unrequested work must be declined.
    send(&mut peer, 40, &offer);
    let refused = receive_code(&mut peer, 41);
    assert_eq!(&refused[..4], transfer_token.to_le_bytes());
    assert_eq!(refused[4], 0);
    let mut leech = TcpStream::connect(("127.0.0.1", peer_port)).unwrap();
    leech
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut init = vec![1];
    init.extend(string("fixture-leech"));
    init.extend(string("P"));
    init.extend(0_u32.to_le_bytes());
    leech.write_all(&(init.len() as u32).to_le_bytes()).unwrap();
    leech.write_all(&init).unwrap();
    send(&mut leech, 43, &string("public\\upload.txt"));
    loop {
        let (code, payload) = receive(&mut leech).unwrap();
        assert_ne!(code, 40, "A zero-share peer must never get an upload offer");
        if code == 50 {
            assert!(String::from_utf8_lossy(&payload).contains("Please share at least"));
            break;
        }
    }
    wait_for(&handle, |s| {
        s.uploads
            .iter()
            .any(|u| u.username == "fixture-leech" && u.status == "blocked" && !u.detail.is_empty())
    });
    handle
        .commands
        .send(Command::ClearTransfers {
            direction: rustysoul::model::HistoryDirection::Uploads,
            status: rustysoul::model::HistoryStatus::Blocked,
        })
        .unwrap();
    wait_for(&handle, |s| {
        !s.uploads.iter().any(|u| u.status == "blocked")
    });
    drop(leech);
    // An independent peer requests a shared file and resumes it from a nonzero offset.
    send(&mut peer, 43, &string("public\\upload.txt"));
    let offer = receive_code(&mut peer, 40);
    assert_eq!(u32::from_le_bytes(offer[..4].try_into().unwrap()), 1);
    let upload_token = u32::from_le_bytes(offer[4..8].try_into().unwrap());
    let mut accepted = upload_token.to_le_bytes().to_vec();
    accepted.push(1);
    send(&mut peer, 41, &accepted);
    upload_listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut upload_socket = loop {
        match upload_listener.accept() {
            Ok((socket, _)) => break socket,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "Upload connection did not arrive"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("Upload listener failed: {e}"),
        }
    };
    upload_socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut length = [0; 4];
    upload_socket.read_exact(&mut length).unwrap();
    let mut init = vec![0; u32::from_le_bytes(length) as usize];
    upload_socket.read_exact(&mut init).unwrap();
    assert_eq!(init[0], 1);
    assert!(init[1..].starts_with(&string("fixture-user")));
    let mut wire_token = [0; 4];
    upload_socket.read_exact(&mut wire_token).unwrap();
    assert_eq!(u32::from_le_bytes(wire_token), upload_token);
    let upload_started = Instant::now();
    upload_socket.write_all(&5_u64.to_le_bytes()).unwrap();
    let mut delivered = vec![];
    upload_socket.read_to_end(&mut delivered).unwrap();
    assert!(
        upload_started.elapsed() >= Duration::from_millis(800),
        "Upload payload ignored its configured speed cap"
    );
    assert_eq!(delivered, shared_content[5..]);
    wait_for(&handle, |s| {
        s.uploads
            .iter()
            .any(|u| u.status == "completed" && u.transferred == shared_content.len() as u64)
    });
    wait_for(&handle, |s| {
        s.traffic.downloaded == contents.len() as u64
            && s.traffic.uploaded == (shared_content.len() - 5) as u64
    });
    handle
        .commands
        .send(Command::ClearTransfers {
            direction: rustysoul::model::HistoryDirection::Downloads,
            status: rustysoul::model::HistoryStatus::Completed,
        })
        .unwrap();
    wait_for(&handle, |s| s.downloads.is_empty());
    assert!(
        destination.join("track.flac").exists(),
        "History clearing must keep the downloaded file"
    );
    handle
        .commands
        .send(Command::ClearTransfers {
            direction: rustysoul::model::HistoryDirection::Uploads,
            status: rustysoul::model::HistoryStatus::Completed,
        })
        .unwrap();
    wait_for(&handle, |s| s.uploads.is_empty());
    let mut paused_uploads = handle.snapshot.borrow().network.clone();
    paused_uploads.upload_slots = 0;
    handle
        .commands
        .send(Command::UpdateNetwork {
            settings: paused_uploads.clone(),
        })
        .unwrap();
    wait_for(&handle, |s| s.network == paused_uploads);
    send(&mut peer, 43, &string("public\\upload.txt"));
    send(&mut peer, 43, &string("public\\second.txt"));
    wait_for(&handle, |s| {
        s.uploads
            .iter()
            .filter(|u| u.username == "fixture-peer" && u.status == "queued")
            .count()
            == 2
    });
    handle
        .commands
        .send(Command::CancelUploads {
            username: "fixture-peer".into(),
            paths: vec!["public\\second.txt".into()],
        })
        .unwrap();
    wait_for(&handle, |s| {
        s.uploads
            .iter()
            .any(|u| u.path == "public\\second.txt" && u.status == "cancelled")
            && s.uploads
                .iter()
                .any(|u| u.path == "public\\upload.txt" && u.status == "queued")
    });
    handle
        .commands
        .send(Command::CancelUploads {
            username: "fixture-peer".into(),
            paths: vec![],
        })
        .unwrap();
    wait_for(&handle, |s| {
        s.uploads
            .iter()
            .filter(|u| u.username == "fixture-peer")
            .all(|u| u.status == "cancelled")
    });
    let mut offered_settings = paused_uploads.clone();
    offered_settings.upload_slots = 1;
    handle
        .commands
        .send(Command::UpdateNetwork {
            settings: offered_settings.clone(),
        })
        .unwrap();
    wait_for(&handle, |s| s.network == offered_settings);
    send(&mut peer, 43, &string("public\\upload.txt"));
    send(&mut peer, 43, &string("public\\second.txt"));
    wait_for(&handle, |s| {
        s.uploads
            .iter()
            .filter(|u| u.username == "fixture-peer" && u.status == "queued")
            .count()
            == 2
    });
    handle
        .commands
        .send(Command::CancelUploads {
            username: "fixture-peer".into(),
            paths: vec![],
        })
        .unwrap();
    wait_for(&handle, |s| {
        s.uploads
            .iter()
            .filter(|u| u.username == "fixture-peer")
            .all(|u| u.status == "cancelled")
    });
    // Each explicit cancellation informs the waiting peer, including an offer
    // it has received but not accepted. Drain these before the later ban check.
    for _ in 0..4 {
        let cancelled = receive_code(&mut peer, 50);
        assert!(cancelled.ends_with(&string("Cancelled")));
    }
    handle
        .commands
        .send(Command::SetUserRule {
            username: "fixture-peer".into(),
            ignored: true,
            banned: true,
        })
        .unwrap();
    wait_for(&handle, |s| {
        s.user_rules
            .iter()
            .any(|r| r.username == "fixture-peer" && r.banned && r.ignored)
    });
    send(&mut peer, 43, &string("public\\upload.txt"));
    let denied = receive_code(&mut peer, 50);
    let mut expected = string("public\\upload.txt");
    expected.extend(string("Banned"));
    assert_eq!(denied, expected);
    send(&mut peer, 4, &[]);
    peer.set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    assert!(
        receive(&mut peer).is_err(),
        "Banned peers must not receive shares"
    );
    drop(peer);
    handle.commands.send(Command::Disconnect).unwrap();
    wait_for(&handle, |s| s.connection == "disconnected");
    handle.stop.send(()).unwrap();
    handle.thread.join().unwrap();
    server.join().unwrap();
    // Wire-received, locally sent, and echoed room messages remain after the
    // daemon stops; the live preview is not the permanent history.
    let history_db = rusqlite::Connection::open(dir.path().join("rustysoul.sqlite3")).unwrap();
    let private_history = rustysoul::messages::page(
        &history_db,
        &rustysoul::messages::Query {
            target: "fixture-peer".into(),
            private: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        !private_history
            .messages
            .iter()
            .any(|m| m.username == "fixture-peer"),
        "Ignored users must be hidden from saved history as well as live snapshots"
    );
    assert!(
        history_db
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE username='fixture-peer'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap()
            > 0,
        "Ignoring must not delete saved history"
    );
    history_db
        .execute(
            "UPDATE user_rules SET ignored=0 WHERE username='fixture-peer'",
            [],
        )
        .unwrap();
    let private_history = rustysoul::messages::page(
        &history_db,
        &rustysoul::messages::Query {
            target: "fixture-peer".into(),
            private: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        private_history
            .messages
            .iter()
            .any(|m| m.username == "fixture-peer")
    );
    assert!(
        private_history
            .messages
            .iter()
            .any(|m| m.text == "Fixture private message 👍🏾 🎶")
    );
    let room_history = rustysoul::messages::page(
        &history_db,
        &rustysoul::messages::Query {
            target: "Fixture room".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        room_history
            .messages
            .iter()
            .any(|m| m.text == "New room activity")
    );
    assert!(
        room_history
            .messages
            .iter()
            .any(|m| m.text == "Fixture room message 🎵 👩🏽‍🎤 ❤️ 🇬🇧")
    );
    let store = rustysoul::store::Store::open(&dir.path().join("rustysoul.sqlite3")).unwrap();
    let stats = rustysoul::traffic::totals(
        &rusqlite::Connection::open(dir.path().join("rustysoul.sqlite3")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        (stats.downloaded, stats.uploaded),
        (contents.len() as u64, (shared_content.len() - 5) as u64)
    );
    assert!(
        store.downloads().unwrap().is_empty(),
        "Cleared download records must remain cleared after restart"
    );
    let users = rusqlite::Connection::open(dir.path().join("rustysoul.sqlite3")).unwrap();
    let peer_totals: (i64, i64) = users
        .query_row(
            "SELECT downloaded,uploaded FROM traffic_users WHERE username='fixture-peer'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        peer_totals,
        (contents.len() as i64, (shared_content.len() - 5) as i64)
    );
    assert_eq!(
        rustysoul::profile::load(&users).unwrap().description,
        "Fixture public profile <script>literal</script>"
    );
    let mut restored = rustysoul::model::Snapshot::default();
    store.restore(&mut restored, 8).unwrap();
    assert_eq!(
        restored.messages[0].text,
        "Hello 👋 🌍 <script>not code</script>"
    );
    assert_eq!(restored.searches[0].query, "reference fixture");
    let database = rusqlite::Connection::open(dir.path().join("rustysoul.sqlite3")).unwrap();
    assert_eq!(
        database
            .query_row(
                "SELECT path FROM local_files WHERE path LIKE '%upload.txt'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "public\\upload.txt"
    );
}

#[test]
fn a_second_daemon_cannot_take_over_the_same_data_directory() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        data_dir: dir.path().into(),
        ..Default::default()
    };
    let secrets = || Secrets::new("s".repeat(32), "".into(), "".into()).unwrap();
    let first = engine::start(config.clone(), secrets()).unwrap();
    assert!(engine::start(config, secrets()).is_err());
    first.stop.send(()).unwrap();
    first.thread.join().unwrap();
}
