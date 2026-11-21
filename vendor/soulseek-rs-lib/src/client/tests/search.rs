//! Answering a search against our shares, and what the client keeps of
//! the answers it gets back.

use super::*;
#[test]
fn draining_responses_preserves_cumulative_limits_and_tokens_are_independent_of_queries() {
    let client = Client::new("me", "pwd");
    client.context.write().unwrap().searches.insert(
        "id".into(),
        Search {
            token: 1,
            results: Vec::new(),
            received_files: 0,
            received_responses: 0,
            truncated: false,
        },
    );
    client
        .context
        .write()
        .unwrap()
        .searches
        .get_mut("id")
        .unwrap()
        .accept(peer_files(3));
    assert_eq!(client.take_search_results("id").0[0].files.len(), 3);
    {
        let mut ctx = client.context.write().unwrap();
        let s = ctx.searches.get_mut("id").unwrap();
        assert!(s.results.is_empty());
        assert_eq!(s.received_files, 3);
        s.received_files = crate::types::MAX_SEARCH_FILES;
        s.accept(peer_files(1));
        assert!(s.truncated);
        assert!(s.results.is_empty());
    }
}

#[test]
fn build_search_response_matches_shares_and_echoes_token() {
    let dir = std::env::temp_dir().join(format!("soulseek-searchresp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("probe_xyzzy.bin"), b"data").unwrap();
    let shares = Shares::scan(&dir).unwrap();

    let response = build_search_response(&shares, "me", 99, "xyzzy", true, 0, 0, &[])
        .expect("a matching share yields a response");
    let mut decoded = crate::message::Message::new_with_data(response.get_buffer());
    decoded.set_pointer(8);
    let result = SearchResult::new_from_message(&mut decoded).unwrap();
    assert_eq!(result.username, "me");
    assert_eq!(result.token, 99);
    assert!(result.files.iter().any(|f| f.name.contains("probe_xyzzy")));

    assert!(build_search_response(&shares, "me", 1, "nomatch", true, 0, 0, &[]).is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn search_file_counts_cover_every_search_without_the_results() {
    // The daemon polls these counts many times a second; they must come from
    // a walk of the cache, never a copy of it.
    let client = Client::new("u", "p");
    {
        let mut context = client.context.write().unwrap();
        context.searches.insert(
            "aphex twin".to_string(),
            Search {
                token: 1,
                results: vec![peer_files(2), peer_files(1)],
                received_files: 3,
                received_responses: 2,
                truncated: false,
            },
        );
        context.searches.insert(
            "nothing yet".to_string(),
            Search {
                token: 2,
                results: Vec::new(),
                received_files: 0,
                received_responses: 0,
                truncated: false,
            },
        );
    }

    let mut counts = client.search_file_counts();
    counts.sort();
    assert_eq!(
        counts,
        [
            ("aphex twin".to_string(), 3),
            ("nothing yet".to_string(), 0)
        ]
    );
}

#[test]
fn a_search_stops_collecting_once_it_has_enough_responses() {
    // A popular query on the live network draws answers for minutes —
    // nearly a million files and gigabytes of memory for one search.
    // Surplus responders are dropped, not archived.
    let mut search = Search {
        token: 1,
        results: Vec::new(),
        received_files: 0,
        received_responses: 0,
        truncated: false,
    };
    for _ in 0..(crate::types::MAX_SEARCH_RESPONSES + 50) {
        search.accept(peer_files(1));
    }
    assert_eq!(search.results.len(), crate::types::MAX_SEARCH_RESPONSES);
}

#[test]
fn a_flood_of_files_fills_a_search_before_the_response_cap() {
    // A handful of whales with huge matching collections must not add up
    // to an unbounded set just because the responses are few.
    let mut search = Search {
        token: 1,
        results: Vec::new(),
        received_files: 0,
        received_responses: 0,
        truncated: false,
    };
    for _ in 0..10 {
        search.accept(peer_files(crate::types::MAX_SEARCH_FILES / 2));
    }
    assert_eq!(
        search.results.len(),
        2,
        "two half-cap responses fill the search; the rest are dropped"
    );
}

#[test]
fn a_file_the_server_excludes_is_left_out_of_a_reply() {
    // The server's excluded phrases (code 160) police what travels the search
    // network: a matching file whose path carries one must not be offered,
    // and a reply with nothing left is not sent at all.
    let dir = std::env::temp_dir().join(format!("soulseek-excluded-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Spam_Xyzzy.bin"), b"data").unwrap();
    let shares = Shares::scan(&dir).unwrap();

    assert!(
        build_search_response(&shares, "me", 1, "xyzzy", true, 0, 0, &[]).is_some(),
        "with no exclusions the file is offered"
    );
    assert!(
        build_search_response(&shares, "me", 1, "xyzzy", true, 0, 0, &["spam".to_string()],)
            .is_none(),
        "an excluded phrase in the path, matched case-insensitively, \
         withholds the file — and with no files there is no reply"
    );
    assert!(
        build_search_response(
            &shares,
            "me",
            1,
            "xyzzy",
            true,
            0,
            0,
            &["unrelated".to_string()],
        )
        .is_some(),
        "an exclusion the path does not carry changes nothing"
    );

    // The phrases are lowercased where they arrive, so a server that sends
    // one capitalised still matches a path.
    let mut ctx = ClientContext::new();
    ctx.set_excluded_search_phrases(vec!["SPAM".to_string()]);
    assert!(
        build_search_response(
            &shares,
            "me",
            1,
            "xyzzy",
            true,
            0,
            0,
            &ctx.excluded_search_phrases(),
        )
        .is_none(),
        "a capitalised phrase from the server still withholds the file"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn search_reply_bursts_bypass_a_full_protected_peer_pool() {
    use crate::message::MessageReader;
    use crate::actor::{ActorSystem, peer_registry::PeerRegistry};
    use crate::message::peer::{FileEntry, build_file_search_response};
    use crate::peer::listen::{ConnectionContext, handle_peer_connection};
    use crate::types::Search;
    use std::io::Write;
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::AtomicUsize;
    let (sender, _operations) = std::sync::mpsc::channel();
    let registry = PeerRegistry::with_capacity(
        Arc::new(ActorSystem::new()),
        sender.clone(),
        "me".into(),
        Arc::new(AtomicUsize::new(1)),
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let held = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    held.set_nonblocking(true).unwrap();
    let (_held_remote, _) = listener.accept().unwrap();
    registry
        .register_peer(
            Peer::new(
                "busy".into(),
                ConnectionType::P,
                "127.0.0.1".into(),
                1,
                None,
                0,
                0,
                0,
            ),
            Some(held),
            None,
        )
        .unwrap();
    let mut ctx = ClientContext::new();
    ctx.peer_registry = Some(registry.clone());
    ctx.pending_serves.insert("busy".into(), vec![1]);
    ctx.searches.insert(
        "supertramp logical flac".into(),
        Search {
            token: 77,
            results: vec![],
            received_files: 0,
            received_responses: 0,
            truncated: false,
        },
    );
    let context = ConnectionContext {
        client_sender: sender,
        client_context: Arc::new(RwLock::new(ctx)),
        own_username: "me".into(),
    };
    let began = Instant::now();
    for batch in 0..25 {
        let count = if batch == 24 { 39 } else { 100 };
        let names: Vec<_> = (0..count)
            .map(|i| format!("Music\\Supertramp\\Logical Song {batch}-{i}.flac"))
            .collect();
        let entries: Vec<_> = names
            .iter()
            .map(|name| FileEntry {
                name,
                size: 1234,
                attribs: &[],
            })
            .collect();
        let name = format!("result-peer-{batch}");
        let mut remote = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        remote
            .write_all(&build_file_search_response(&name, 77, &entries, 1, 8000, 0).get_buffer())
            .unwrap();
        let peer = Peer::new(
            name.clone(),
            ConnectionType::P,
            "127.0.0.1".into(),
            1,
            None,
            0,
            0,
            0,
        );
        assert!(
            !handle_peer_connection(peer, stream, MessageReader::new(), &context),
            "standalone replies must not allocate a persistent actor"
        );
        assert!(!registry.contains(&name));
    }
    let elapsed = began.elapsed();
    let ctx = context.client_context.read().unwrap();
    assert_eq!(ctx.searches["supertramp logical flac"].received_files, 2439);
    assert_eq!(
        ctx.searches["supertramp logical flac"].received_responses,
        25
    );
    assert!(
        registry.contains("busy"),
        "the protected transfer connection was evicted"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "reply burst took {elapsed:?}"
    );
    println!("Admitted 2439 files from 25 peers with a full protected pool in {elapsed:?}");
    registry.stop_all();
}
