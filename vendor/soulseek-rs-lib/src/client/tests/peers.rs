//! Uploads, broker connects, and the peers a disconnect may not drop.

use super::*;

#[test]
fn upload_speed_uses_recent_payload_and_reaches_zero_after_a_stall() {
    let start = Instant::now();
    let mut rate = super::super::UploadRate::default();
    assert_eq!(rate.sample(start, 0), 0.);
    assert_eq!(rate.sample(start + Duration::from_secs(1), 1024), 1024.);
    for n in 2..=7 {
        rate.sample(start + Duration::from_secs(n), 1024);
    }
    assert_eq!(rate.sample(start + Duration::from_secs(8), 1024), 0.);
    for n in 0..1000 {
        rate.sample(
            start + Duration::from_secs(9) + Duration::from_millis(n),
            2048,
        );
    }
    assert!(rate.samples.len() <= 8);
}

// A clean disconnect — the idle reaper, or a remote client tidying an idle
// socket while it waits in our queue — must keep the peer's queued uploads;
// only an error disconnect is evidence the peer is gone.

#[test]
fn a_clean_disconnect_keeps_queued_uploads_an_error_drops_them() {
    let client = Client::new("test-user", "test-password");
    let (ops, ops_rx) = mpsc::channel();
    Client::listen_to_client_operations(ops_rx, client.context.clone(), "me".to_string());

    client.context.write().unwrap().enqueue_upload(
        "amy",
        "@@share\\f.mp3",
        std::path::PathBuf::from("/tmp/f.mp3"),
        4096,
    );

    ops.send(ClientOperation::PeerDisconnected(
        1,
        "amy".to_string(),
        None,
    ))
    .unwrap();
    // The loop is serial, so once this fence op is visible the disconnect
    // before it has been handled.
    ops.send(ClientOperation::OwnPrivileges(7)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.own_privilege_seconds() != Some(7) {
        assert!(Instant::now() < deadline, "ops loop never caught up");
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        client
            .context
            .read()
            .unwrap()
            .place_in_queue("amy", "@@share\\f.mp3"),
        Some(1),
        "a clean disconnect must keep the queue"
    );

    ops.send(ClientOperation::PeerDisconnected(
        1,
        "amy".to_string(),
        Some(crate::error::SoulseekRs::NotConnected),
    ))
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let place = client
            .context
            .read()
            .unwrap()
            .place_in_queue("amy", "@@share\\f.mp3");
        if place.is_none() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "an error disconnect must drop the queue"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_pending_connect_expires_after_the_broker_timeout() {
    let mut context = ClientContext::new();
    context.add_pending_connect(7, "ghost".to_string());

    assert!(context.take_expired_connects(Instant::now()).is_empty());

    let after_deadline = Instant::now() + BROKER_CONNECT_TIMEOUT + Duration::from_secs(1);
    assert_eq!(
        context.take_expired_connects(after_deadline),
        vec!["ghost".to_string()]
    );
    assert!(context.take_pending_connect(7).is_none());
}

#[test]
fn upload_reverse_connect_allows_legacy_clients_45_seconds_without_extending_control_dials() {
    let mut ctx = ClientContext::new();
    ctx.add_brokered_upload(11, 22, "f.flac".into());
    assert!(ctx.take_expired_brokered_uploads(Instant::now() + Duration::from_secs(30)).is_empty());
    assert_eq!(ctx.take_expired_brokered_uploads(Instant::now() + Duration::from_secs(46)).len(), 1);
    assert_eq!(BROKER_CONNECT_TIMEOUT, Duration::from_secs(20));
}

#[test]
fn a_closed_control_recovers_a_reserved_offer_and_reconnects_instead_of_timing_out() {
    for error in [None, Some(crate::error::SoulseekRs::NotConnected)] {
        let client = Client::new("me", "password");
        let (ops, rx) = mpsc::channel();
        let (server, messages) = mpsc::channel();
        {
            let mut ctx = client.context.write().unwrap();
            ctx.enqueue_upload("alice", "album\\track.flac", "track.flac".into(), 100);
            ctx.pump_uploads(|| 10);
            ctx.uploads.get_mut(&10).unwrap().delivery_actor = Some(7);
            ctx.peer_registry = Some(PeerRegistry::new(ctx.actor_system.clone(), ops.clone(), "me".into()));
            ctx.server_sender = Some(server);
        }
        Client::listen_to_client_operations(rx, client.context.clone(), "me".into());
        ops.send(ClientOperation::PeerDisconnected(7, "alice".into(), error)).unwrap();
        ops.send(ClientOperation::OwnPrivileges(7)).unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        while client.own_privilege_seconds() != Some(7) {
            assert!(Instant::now() < end);
            thread::sleep(Duration::from_millis(5));
        }
        let mut ctx = client.context.write().unwrap();
        assert!(ctx.uploads.is_empty(), "the dead actor must not retain its reserved offer");
        assert_eq!(ctx.place_in_queue("alice", "album\\track.flac"), Some(1));
        assert!(matches!(messages.try_recv(), Ok(ServerMessage::GetPeerAddress(user)) if user == "alice"));
        assert!(!ctx.expire_stale_offers(Instant::now() + Duration::from_secs(61)));
    }
}

#[test]
fn a_replaced_control_error_cannot_release_its_successors_uploads() {
    use std::net::{TcpListener, TcpStream};
    let client = Client::new("me", "password");
    let (ops, rx) = mpsc::channel();
    let registry = PeerRegistry::new(Arc::new(crate::actor::ActorSystem::new()), ops.clone(), "me".into());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let remote = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (stream, _) = listener.accept().unwrap();
    stream.set_nonblocking(true).unwrap();
    let peer = Peer::new("alice".into(), ConnectionType::P, "127.0.0.1".into(), 0, None, 0, 0, 0);
    registry.register_peer(peer, Some(stream), None).unwrap();
    let (current_id, _) = registry.get_peer_with_id("alice").unwrap();
    {
        let mut ctx = client.context.write().unwrap();
        ctx.enqueue_upload("alice", "new.flac", "new.flac".into(), 100);
        ctx.pump_uploads(|| 10);
        ctx.uploads.get_mut(&10).unwrap().delivery_actor = Some(current_id);
        ctx.peer_registry = Some(registry.clone());
    }
    Client::listen_to_client_operations(rx, client.context.clone(), "me".into());
    ops.send(ClientOperation::PeerDisconnected(current_id + 1000, "alice".into(), Some(crate::error::SoulseekRs::NotConnected))).unwrap();
    ops.send(ClientOperation::OwnPrivileges(7)).unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while client.own_privilege_seconds() != Some(7) {
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(registry.contains("alice"));
    assert!(client.context.read().unwrap().uploads.contains_key(&10));
    registry.stop_all();
    drop(remote);
}

#[test]
fn protected_peers_covers_downloads_uploads_and_pending_serves() {
    let mut context = ClientContext::new();
    context.add_download(download(
        "downloader",
        "song.mp3",
        1,
        DownloadStatus::Queued,
        mpsc::channel().0,
    ));
    context.add_download(download(
        "done",
        "old.mp3",
        2,
        DownloadStatus::Completed,
        mpsc::channel().0,
    ));
    context
        .pending_serves
        .insert("waiting".to_string(), vec![9]);
    context.mark_browse_pending("browsed");

    let protected = context.protected_peers();
    assert!(protected.contains("downloader"));
    assert!(protected.contains("waiting"));
    assert!(protected.contains("browsed"));
    assert!(!protected.contains("done"));

    context.store_browse_result("browsed".to_string(), Vec::new());
    assert!(!context.protected_peers().contains("browsed"));
}

#[test]
fn a_peer_message_nobody_could_deliver_expires() {
    let mut ctx = ClientContext::for_user("me");
    ctx.queue_peer_message("ghost", crate::message::Message::new());
    ctx.expire_pending_peer_messages(Instant::now());
    assert_eq!(ctx.take_peer_messages("ghost").len(), 1, "still fresh");

    ctx.queue_peer_message("ghost", crate::message::Message::new());
    ctx.expire_pending_peer_messages(Instant::now() + PENDING_PEER_TTL);
    assert!(ctx.take_peer_messages("ghost").is_empty(), "expired");
}

#[test]
fn the_server_is_told_our_child_capacity_only_when_it_changes() {
    // AcceptChildren is a standing state on the server, not a heartbeat.
    let mut ctx = ClientContext::new();
    assert_eq!(
        ctx.accept_children_change(),
        Some(false),
        "the first answer is always worth sending"
    );
    assert_eq!(ctx.accept_children_change(), None, "unchanged, so silent");
}

#[test]
fn repeated_privilege_balances_have_distinct_observation_revisions() {
    let client = Client::new("test-user", "test-password");
    let (ops, receiver) = mpsc::channel();
    Client::listen_to_client_operations(receiver, client.context.clone(), "me".into());
    assert_eq!(client.own_privilege_observation(), None);
    for revision in 1..=2 {
        ops.send(ClientOperation::OwnPrivileges(0)).unwrap();
        let end = Instant::now() + Duration::from_secs(2);
        while client.own_privilege_observation() != Some((0, revision)) {
            assert!(Instant::now() < end);
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(client.own_privilege_seconds(), Some(0));
    }
}
