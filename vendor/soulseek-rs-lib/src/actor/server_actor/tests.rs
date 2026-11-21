use super::*;

#[test]
fn outbound_backpressure_keeps_the_exact_suffix_and_order_across_short_writes() {
    struct SlowWriter { output: Vec<u8>, calls: usize }
    impl Write for SlowWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.calls += 1;
            match self.calls % 4 {
                1 => Err(io::ErrorKind::Interrupted.into()),
                3 => Err(io::ErrorKind::WouldBlock.into()),
                _ => { let n = bytes.len().min(3); self.output.extend_from_slice(&bytes[..n]); Ok(n) },
            }
        }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    let first = MessageFactory::build_file_search_message(123, "first fixture").get_buffer();
    let next = MessageFactory::build_file_search_message(124, "second fixture").get_buffer();
    let mut pending: std::collections::VecDeque<_> = first.iter().copied().collect();
    let mut writer = SlowWriter { output: vec![], calls: 0 };
    assert_eq!(write_pending(&mut writer, &mut pending).unwrap(), 3);
    assert_eq!(pending.iter().copied().collect::<Vec<_>>(), first[3..]);
    pending.extend(next.iter().copied());
    for _ in 0..1000 { if pending.is_empty() { break; } write_pending(&mut writer, &mut pending).unwrap(); }
    assert!(pending.is_empty());
    assert_eq!(writer.output, [first, next].concat());
}

#[test]
fn outbound_work_is_bounded_and_real_write_errors_are_preserved() {
    let mut pending = std::collections::VecDeque::from(vec![1; SERVER_WRITE_BUDGET * 2]);
    let mut output = Vec::new();
    assert_eq!(write_pending(&mut output, &mut pending).unwrap(), SERVER_WRITE_BUDGET);
    assert_eq!(pending.len(), SERVER_WRITE_BUDGET);
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> { Err(io::ErrorKind::BrokenPipe.into()) }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    assert_eq!(write_pending(&mut Broken, &mut pending).unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(pending.len(), SERVER_WRITE_BUDGET);
}

#[test]
fn an_overflow_disconnect_drops_all_unsent_bytes_before_the_next_session() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (_remote, _) = listener.accept().unwrap();
    let mut actor = parked_actor(1);
    actor.stream = Some(stream);
    actor.connection_state = ConnectionState::Connected;
    actor.pending_write = vec![0; MAX_SERVER_PENDING_WRITE].into();
    actor.write_progress_at = Some(Instant::now());
    actor.send_message(MessageFactory::build_file_search_message(1, "overflow fixture"));
    assert!(actor.stream.is_none());
    assert!(actor.pending_write.is_empty());
    assert!(actor.write_progress_at.is_none());
    assert_eq!(actor.session.loss(), Some(SessionLoss::Disconnected));
}

#[test]
fn slow_server_reads_resume_without_disconnect_truncation_or_reordering() {
    use std::io::Read;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    stream.set_nonblocking(true).unwrap();
    let (mut remote, _) = listener.accept().unwrap();
    remote.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let (mut actor, operations) = parked_actor_with_client(1);
    actor.stream = Some(stream);
    actor.connection_state = ConnectionState::Connected;
    actor.initialize_dispatcher();
    let mut expected = Vec::new();
    for i in 0..96 {
        let mut message = Message::new();
        message.write_int32(26).write_string(&format!("{i:03}:{}", "x".repeat(65536)));
        expected.extend(message.get_buffer());
        actor.send_message(message);
    }
    assert!(actor.stream.is_some());
    assert!(actor.pending_write.len() <= MAX_SERVER_PENDING_WRITE);
    assert_eq!(actor.session.loss(), None);
    // Read-side protocol work still runs while output is queued.
    let mut reply = Message::new(); reply.write_int32(92).write_int32(321);
    remote.write_all(&reply.get_buffer()).unwrap();
    let mut received_reply = false;
    for _ in 0..20 {
        actor.tick();
        if operations.try_iter().any(|op| matches!(op, ClientOperation::OwnPrivileges(321))) { received_reply = true; break; }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(received_reply, "outgoing backpressure starved an incoming server reply");
    let count = expected.len();
    let receiver = std::thread::spawn(move || {
        let mut actual = vec![0; count]; remote.read_exact(&mut actual).unwrap(); actual
    });
    let end = Instant::now() + Duration::from_secs(10);
    while !actor.pending_write.is_empty() {
        assert!(Instant::now() < end);
        actor.flush_pending_write();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(receiver.join().unwrap(), expected);
    assert!(actor.stream.is_some());
    assert_eq!(actor.session.loss(), None);
    actor.disconnect();
    assert!(actor.pending_write.is_empty());
    assert!(actor.write_progress_at.is_none());
}

#[test]
fn a_large_room_roster_does_not_delay_a_private_room_confirmation() {
    use std::io::Write;
    use std::net::{TcpListener, TcpStream};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    stream.set_nonblocking(true).unwrap();
    let (mut remote, _) = listener.accept().unwrap();
    let (mut actor, received) = parked_actor_with_client(1);
    actor.stream = Some(stream);
    actor.connection_state = ConnectionState::Connected;
    actor.initialize_dispatcher();
    let writer = std::thread::spawn(move || {
        let mut roster = Message::new();
        roster
            .write_int32(14)
            .write_string("Large public room")
            .write_int32(5000);
        for i in 0..5000 {
            roster.write_string(&format!("fixture-member-{i:05}"));
        }
        for _ in 0..4 {
            roster.write_int32(0);
        }
        remote.write_all(&roster.get_buffer()).unwrap();
        let mut joined = Message::new();
        joined.write_int32(14).write_string("Owned room");
        for _ in 0..5 {
            joined.write_int32(0);
        }
        joined.write_string("fixture-owner").write_int32(0);
        remote.write_all(&joined.get_buffer()).unwrap();
    });
    // Eight bounded reads/ticks should consume this roster, not require
    // hundreds of 100 ms ticks before the next room's acknowledgement.
    let mut confirmed = false;
    for _ in 0..8 {
        actor.process_read();
        for op in received.try_iter() {
            if let ClientOperation::RoomEvent(RoomEvent::PrivateJoined { room, owner, .. }) = op {
                assert_eq!(room, "Owned room");
                assert_eq!(owner, "fixture-owner");
                confirmed = true;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(confirmed, "a large roster starved the room confirmation");
    writer.join().unwrap();
}

fn code_of(message: &Message) -> u32 {
    u32::from_le_bytes(message.get_data()[0..4].try_into().unwrap())
}

#[test]
fn a_timed_out_connect_parks_the_actor_in_disconnected() {
    let mut actor = parked_actor(1);
    actor.connection_state = ConnectionState::Connecting {
        since: Instant::now().checked_sub(Duration::from_secs(21)).unwrap(),
    };

    actor.tick();

    assert!(
        matches!(actor.connection_state, ConnectionState::Disconnected),
        "a timed-out connect must leave Connecting"
    );
}

#[cfg(unix)]
#[test]
fn the_server_socket_keeps_itself_alive() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut actor = parked_actor(listener.local_addr().unwrap().port());

    assert!(actor.initiate_connection());

    assert!(
        crate::utils::keepalive::keepalive_enabled(actor.stream.as_ref().unwrap()),
        "an idle NAT mapping must be kept alive and a dead path noticed"
    );
}

fn parked_actor(port: u16) -> ServerActor {
    parked_actor_with_client(port).0
}

fn parked_actor_with_client(
    port: u16,
) -> (ServerActor, std::sync::mpsc::Receiver<ClientOperation>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let actor = ServerActor::new(
        PeerAddress::new("127.0.0.1".to_string(), port),
        tx,
        0,
        false,
        0,
        0,
    );
    (actor, rx)
}

#[test]
fn dropping_the_connection_forgets_the_dispatcher_and_the_verdict() {
    let mut actor = parked_actor(1);
    actor.initialize_dispatcher();
    actor.connection_state = ConnectionState::Connected;
    actor.handle_login_status(true);

    actor.disconnect_with_error();

    assert_eq!(actor.session.loss(), Some(SessionLoss::Disconnected));
    assert!(
        actor.dispatcher_sender.is_none(),
        "a login queued now must wait in the buffer, not in a channel \
         the next connection replaces"
    );
    assert_eq!(
        actor.context.read().unwrap().logged_in,
        None,
        "the old verdict must not answer the next login"
    );
}

#[test]
fn disconnect_clears_a_partial_frame_so_the_next_session_reframes_clean() {
    use std::io::Write;

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let client = TcpStream::connect(addr).unwrap();
    let (mut server_side, _) = listener.accept().unwrap();
    let partial_frame = [10u8, 0, 0, 0, 1, 2, 3];
    server_side.write_all(&partial_frame).unwrap();
    server_side.flush().unwrap();
    client.set_nonblocking(true).unwrap();

    let mut actor = parked_actor(addr.port());
    actor.stream = Some(client);
    actor.connection_state = ConnectionState::Connected;
    for _ in 0..50 {
        actor.process_read();
        if actor.reader.buffer_len() > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        actor.reader.buffer_len() > 0,
        "an incomplete frame should be waiting for its rest"
    );

    actor.disconnect();

    assert_eq!(
        actor.reader.buffer_len(),
        0,
        "a stale partial frame carried across a reconnect reads the next \
         session's bytes as a length prefix and wedges recovery"
    );
}

#[test]
fn a_login_on_a_parked_actor_dials_again_and_queues_the_login() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut actor = parked_actor(listener.local_addr().unwrap().port());
    actor.session.record(SessionLoss::Disconnected);
    let (response, _verdict) = std::sync::mpsc::channel();

    actor.handle_login("u".into(), "p".into(), ClientVersion::default(), response);

    assert!(
        matches!(actor.connection_state, ConnectionState::Connecting { .. }),
        "a parked actor must dial again on login"
    );
    assert_eq!(
        actor.queued_messages.len(),
        1,
        "the login waits for the connection to come up"
    );
}

#[test]
fn a_parked_actor_handles_login_rather_than_queueing_it() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut actor = parked_actor(listener.local_addr().unwrap().port());
    actor.session.record(SessionLoss::Disconnected);
    let (response, _verdict) = std::sync::mpsc::channel();

    actor.handle_message(ServerMessage::Login {
        username: "u".into(),
        password: "p".into(),
        version: ClientVersion::default(),
        response,
    });

    assert!(
        matches!(actor.connection_state, ConnectionState::Connecting { .. }),
        "a parked actor must act on a login, not park it in the queue"
    );
    assert!(
        actor
            .queued_messages
            .iter()
            .all(|m| !matches!(m, ServerMessage::Login { .. })),
        "the login must not be sitting unhandled in the queue"
    );
}

#[test]
fn a_login_with_nobody_listening_is_refused_at_once() {
    let mut actor = parked_actor(1);
    let (response, verdict) = std::sync::mpsc::channel();

    actor.handle_login("u".into(), "p".into(), ClientVersion::default(), response);

    assert!(
        matches!(
            verdict.recv_timeout(Duration::from_secs(5)),
            Ok(Err(SoulseekRs::NotConnected))
        ),
        "a refused dial must not make the caller wait out the verdict \
         timeout"
    );
}

#[test]
fn a_cancelled_actor_never_writes_the_login_it_queued() {
    use std::io::Read;

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let client = TcpStream::connect(addr).unwrap();
    let (mut server_side, _) = listener.accept().unwrap();
    client.set_nonblocking(true).unwrap();

    let cancel = CancelHandle::new();
    let mut actor = parked_actor(addr.port());
    actor.set_cancel(cancel.clone());
    actor.stream = Some(client);
    actor.connection_state = ConnectionState::Connecting {
        since: Instant::now(),
    };
    let (response, _verdict) = std::sync::mpsc::channel();
    actor.handle_login("u".into(), "p".into(), ClientVersion::default(), response);
    assert_eq!(actor.queued_messages.len(), 1);

    cancel.cancel();
    actor.tick();
    assert!(matches!(actor.connection_state, ConnectionState::Connected));
    actor.on_stop();

    server_side
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut written = Vec::new();
    assert_eq!(server_side.read_to_end(&mut written).unwrap(), 0);
}

#[test]
fn a_successful_login_marks_the_session_live_again() {
    let mut actor = parked_actor(1);
    actor.session.record(SessionLoss::Disconnected);

    actor.handle_login_status(true);

    assert_eq!(actor.session.loss(), None);
}

// A new session starts without a parent: the leaf drops whatever tree it
// hung from and announces its parentless stance from there.
#[test]
fn a_successful_login_resets_the_distributed_leaf() {
    let (mut actor, client) = parked_actor_with_client(1);

    actor.handle_login_status(true);

    assert!(matches!(
        client.try_recv(),
        Ok(ClientOperation::ResetDistributed)
    ));
    // A new session also has to be told what only the session held: our
    // interests, which the server forgets when the old one ended.
    assert!(matches!(
        client.try_recv(),
        Ok(ClientOperation::SessionEstablished)
    ));
    actor.handle_login_status(false);
    assert!(client.try_recv().is_err(), "a failed login resets nothing");
}

#[test]
fn post_login_messages_carry_counts_and_conditional_wait_port() {
    let messages = post_login_messages(true, 4321, 3, 7);
    let codes: Vec<u32> = messages.iter().map(code_of).collect();
    // SharedFolders; SetStatus; SetWaitPort.
    assert_eq!(codes, vec![35, 28, 2]);

    // The SharedFolders message (code 35) carries the real counts.
    let shared = messages[0].get_data();
    assert_eq!(u32::from_le_bytes(shared[4..8].try_into().unwrap()), 3);
    assert_eq!(u32::from_le_bytes(shared[8..12].try_into().unwrap()), 7);

    // Not listening omits SetWaitPort (code 2).
    let no_listen = post_login_messages(false, 4321, 3, 7);
    let codes: Vec<u32> = no_listen.iter().map(code_of).collect();
    assert_eq!(codes, vec![35, 28]);
}
