//! Serving a shared file to a peer over an F (file transfer) connection.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::message::server::MessageFactory;
use crate::peer::ConnectionType;
use crate::trace;
use crate::utils::bandwidth::UserBandwidth;

/// Connect to the downloader's file listener and stream `path`'s bytes.
///
/// We announce ourselves with a `PeerInit(F)` immediately followed by the raw
/// transfer token (so it lands in the downloader's read buffer, where the
/// download is matched by token), then the downloader sends an 8-byte
/// START_DOWNLOAD offset before we stream the file.
///
/// `bytes_sent` is updated as the transfer progresses, and setting `cancel`
/// aborts the stream with an [`io::ErrorKind::Interrupted`] error. Returns the
/// bytes streamed by this call, which a resumed transfer's prefix is not.
///
/// # Errors
/// Returns any I/O error opening the file or talking to the peer.
#[cfg(test)]
pub fn serve_file(
    host: &str,
    port: u32,
    own_username: &str,
    token: u32,
    path: &Path,
    bytes_sent: &AtomicU64,
    cancel: &AtomicBool,
) -> io::Result<u64> {
    let stream = dial(host, port)?;
    serve_direct(stream, own_username, token, path, bytes_sent, cancel)
}

/// Connect to `host:port` with the transfer deadlines applied.
///
/// # Errors
/// Returns the connect error, or `InvalidInput` for an unresolvable address.
pub fn dial(host: &str, port: u32) -> io::Result<TcpStream> {
    let socket = format!("{host}:{port}")
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no address"))?;
    with_deadlines(TcpStream::connect_timeout(
        &socket,
        Duration::from_secs(20),
    )?)
}

const IO_POLL: Duration = Duration::from_millis(500);
const STALL_DEADLINE: Duration = Duration::from_secs(30);
const PAYLOAD_STALL_DEADLINE: Duration = Duration::from_secs(60);

/// Linux may keep application writes asleep until enough send-buffer space is
/// free, even while the peer acknowledges smaller amounts of buffered data.
/// Count actual queue drain as progress, never zero-window probes alone.
#[derive(Default)]
struct WireProgress(Option<u64>);
impl WireProgress {
    fn observe(&mut self, queued: Option<u64>, progress: &mut Instant, now: Instant) {
        if let (Some(previous), Some(current)) = (self.0, queued)
            && current < previous { *progress = now; }
        self.0 = queued;
    }
}
#[cfg(target_os = "linux")]
fn pending_output(stream: &TcpStream) -> Option<u64> {
    use std::os::fd::AsRawFd;
    let mut queued: libc::c_int = 0;
    // SAFETY: the live TCP descriptor is queried with TIOCOUTQ, which writes
    // exactly one c_int into the valid, exclusively borrowed stack value.
    let result = unsafe { libc::ioctl(stream.as_raw_fd(), libc::TIOCOUTQ as _, &raw mut queued) };
    (result == 0 && queued >= 0).then_some(queued as u64)
}
#[cfg(not(target_os = "linux"))]
fn pending_output(_stream: &TcpStream) -> Option<u64> { None }

fn abort_failed_stream(stream: &TcpStream) {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let linger = libc::linger { l_onoff: 1, l_linger: 0 };
        // SAFETY: SO_LINGER reads this initialized, correctly sized value on
        // a live TCP descriptor. Dropping the stream then discards unsent data
        // and resets the failed transfer instead of continuing it in FIN-WAIT.
        let result = unsafe { libc::setsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_LINGER,
            (&raw const linger).cast(), std::mem::size_of_val(&linger) as _) };
        if result == 0 { return; }
        crate::warn!("[upload] could not discard failed stream's pending data: {}", io::Error::last_os_error());
    }
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

fn with_deadlines(stream: TcpStream) -> io::Result<TcpStream> {
    // Poll connected socket waits so cancellation does not wait for the entire
    // peer deadline. An idle peer still gets the full 30-second stall budget.
    stream.set_read_timeout(Some(IO_POLL))?;
    stream.set_write_timeout(Some(IO_POLL))?;
    stream.set_nodelay(true).ok();
    Ok(stream)
}

fn check_wait(
    cancel: &AtomicBool,
    since: Instant,
    timeout: Duration,
    reason: &'static str,
) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(io::ErrorKind::Interrupted.into());
    }
    if since.elapsed() >= timeout {
        return Err(io::Error::new(io::ErrorKind::TimedOut, reason));
    }
    Ok(())
}
fn transient(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
    )
}
fn write_handshake(
    stream: &mut TcpStream,
    mut bytes: &[u8],
    cancel: &AtomicBool,
) -> io::Result<()> {
    let since = Instant::now();
    while !bytes.is_empty() {
        check_wait(
            cancel,
            since,
            STALL_DEADLINE,
            "The receiving peer did not accept the transfer handshake within 30 seconds.",
        )?;
        match stream.write(bytes) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => bytes = &bytes[n..],
            Err(e) if transient(&e) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
fn read_resume_offset(
    stream: &mut TcpStream,
    cancel: &AtomicBool,
    timeout: Duration,
    allow_empty_eof: bool,
) -> io::Result<u64> {
    let since = Instant::now();
    let mut bytes = [0; 8];
    let mut received = 0;
    while received < bytes.len() {
        // This tiny handshake has one total deadline, even if bytes trickle in.
        check_wait(
            cancel,
            since,
            timeout,
            "The receiving peer did not send its resume offset within 30 seconds.",
        )?;
        match stream.read(&mut bytes[received..]) {
            // Nicotine+ closes after matching the transfer token when the
            // destination already contains all zero advertised bytes. No
            // payload or resume offset is needed for this empty-file case.
            Ok(0) if received == 0 && allow_empty_eof => {
                check_wait(
                    cancel,
                    since,
                    timeout,
                    "The empty-file handshake timed out.",
                )?;
                return Ok(0);
            }
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => received += n,
            Err(e) if transient(&e) => {}
            Err(e) => return Err(e),
        }
    }
    check_wait(
        cancel,
        since,
        timeout,
        "The receiving peer did not send its resume offset within 30 seconds.",
    )?;
    Ok(u64::from_le_bytes(bytes))
}

/// Stream `path` over a connection we dialled: announce with `PeerInit(F)`
/// and the token, then hand over to [`stream_file`].
///
/// # Errors
/// Returns any I/O error opening the file or talking to the peer.
#[cfg(test)]
pub fn serve_direct(
    stream: TcpStream,
    own_username: &str,
    token: u32,
    path: &Path,
    bytes_sent: &AtomicU64,
    cancel: &AtomicBool,
) -> io::Result<u64> {
    serve_direct_limited(stream, own_username, token, path, bytes_sent, cancel, None)
}
pub(crate) fn serve_direct_limited(
    stream: TcpStream,
    own_username: &str,
    token: u32,
    path: &Path,
    bytes_sent: &AtomicU64,
    cancel: &AtomicBool,
    bandwidth: Option<&UserBandwidth>,
) -> io::Result<u64> {
    let file = File::open(path)?;
    let mut stream = with_deadlines(stream)?;
    // PeerInit(F) + the 4-byte token in a single write so they coalesce.
    let mut init = MessageFactory::build_peer_init_message(own_username, ConnectionType::F, token)
        .get_buffer();
    init.extend_from_slice(&token.to_le_bytes());
    write_handshake(&mut stream, &init, cancel)?;
    stream.flush()?;
    stream_file(stream, file, path, bytes_sent, cancel, bandwidth)
}

/// Stream `path` over a connection the downloader opened to us after the
/// server brokered it: it has already sent `PierceFirewall`, so only the
/// 4-byte transfer token precedes the offset exchange.
///
/// # Errors
/// Returns any I/O error opening the file or talking to the peer.
#[cfg(test)]
pub fn serve_pierced(
    stream: TcpStream,
    token: u32,
    path: &Path,
    bytes_sent: &AtomicU64,
    cancel: &AtomicBool,
) -> io::Result<u64> {
    serve_pierced_limited(stream, token, path, bytes_sent, cancel, None)
}
pub(crate) fn serve_pierced_limited(
    stream: TcpStream,
    token: u32,
    path: &Path,
    bytes_sent: &AtomicU64,
    cancel: &AtomicBool,
    bandwidth: Option<&UserBandwidth>,
) -> io::Result<u64> {
    let file = File::open(path)?;
    let mut stream = with_deadlines(stream)?;
    write_handshake(&mut stream, &token.to_le_bytes(), cancel)?;
    stream.flush()?;
    stream_file(stream, file, path, bytes_sent, cancel, bandwidth)
}

fn stream_file(
    stream: TcpStream,
    file: File,
    path: &Path,
    bytes_sent: &AtomicU64,
    cancel: &AtomicBool,
    bandwidth: Option<&UserBandwidth>,
) -> io::Result<u64> {
    stream_file_with_timeout(
        stream,
        file,
        path,
        bytes_sent,
        cancel,
        bandwidth,
        PAYLOAD_STALL_DEADLINE,
    )
}
fn stream_file_with_timeout(
    mut stream: TcpStream,
    mut file: File,
    path: &Path,
    bytes_sent: &AtomicU64,
    cancel: &AtomicBool,
    bandwidth: Option<&UserBandwidth>,
    timeout: Duration,
) -> io::Result<u64> {
    let outcome = (|| {
    // The downloader replies with an 8-byte START_DOWNLOAD offset first. A
    // non-zero offset means it is resuming and already holds that prefix, so
    // seek past it and count it as sent to keep progress absolute.
    let file_size = file.metadata()?.len();
    let offset = read_resume_offset(&mut stream, cancel, STALL_DEADLINE, file_size == 0)?;
    if offset > file_size {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("download offset {offset} exceeds file size {file_size}"),
        ));
    }
    file.seek(SeekFrom::Start(offset))?;
    bytes_sent.fetch_add(offset, Ordering::Relaxed);

    // Multiple files to one recipient share a single wall-clock interval.
    // Empty files, completed resume prefixes and handshakes add no samples.
    let _upload_timing = bandwidth.filter(|_| offset < file_size).map(UserBandwidth::begin_upload);

    let mut buffer = vec![0u8; 64 * 1024];
    let mut streamed = 0u64;
    let mut progress = Instant::now();
    let mut wire = WireProgress::default();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let reading = Instant::now();
        let read = file.read(&mut buffer)?;
        progress += reading.elapsed(); // Local disk latency is not peer silence.
        if read == 0 {
            break;
        }
        let mut written = 0;
        while written < read {
            check_wait(
                cancel,
                progress,
                timeout,
                "No upload payload progress for 60 seconds; the receiving peer or connection stopped accepting data.",
            )?;
            let waiting = Instant::now();
            let allowance = bandwidth
                .map(|b| b.take(read - written, || cancel.load(Ordering::Relaxed)))
                .transpose()?;
            progress += waiting.elapsed(); // Our own speed limits must not time out a healthy peer.
            check_wait(
                cancel,
                progress,
                timeout,
                "No upload payload progress for 60 seconds; the receiving peer or connection stopped accepting data.",
            )?;
            let permitted = allowance.as_ref().map_or(read - written, |a| a.bytes);
            match stream.write(&buffer[written..written + permitted]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => {
                    if let Some(allowance) = allowance {
                        allowance.finish(n);
                    }
                    written += n;
                    bytes_sent.fetch_add(n as u64, Ordering::Relaxed);
                    streamed += n as u64;
                    progress = Instant::now();
                    wire.0 = None;
                }
                Err(e) if transient(&e) => {
                    wire.observe(pending_output(&stream), &mut progress, Instant::now());
                } // Dropping unused allowance refunds both budgets.
                Err(e) => return Err(e),
            }
        }
    }
    stream.flush()?;

    // Completion means the peer has acknowledged the payload, rather than
    // merely that our kernel accepted the final buffered writes. No peer FIN
    // is required. Unsupported platforms retain the previous write behavior.
    loop {
        if let Some(error) = stream.take_error()? { return Err(error); }
        let queued = pending_output(&stream);
        wire.observe(queued, &mut progress, Instant::now());
        if queued.is_none_or(|n| n == 0) { break; }
        check_wait(cancel, progress, timeout,
            "No upload payload progress for 60 seconds; the receiving peer or connection stopped accepting data.")?;
        std::thread::sleep(Duration::from_millis(25));
    }

    // The file is delivered. Soulseek leaves closing to the downloader, but
    // waiting for that costs the upload slot the next peer is queued for —
    // and a peer that leaves the socket open, or whose FIN a NAT swallows,
    // would hold it for the whole wait. Nicotine+ finishes the transfer at
    // delivery for the same reason; the close below is ours to make.
    trace!(
        "[upload] served {} to {:?}",
        path.display(),
        stream.peer_addr()
    );
    Ok(streamed)
    })();
    if outcome.is_err() { abort_failed_stream(&stream); }
    outcome
}

#[cfg(test)]
mod tests {
    use super::serve_file;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    fn read_closed(stream: &mut TcpStream) -> Vec<u8> {
        let mut received = Vec::new();
        match stream.read_to_end(&mut received) {
            Ok(_) => {}
            Err(error) if matches!(error.kind(), std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted | std::io::ErrorKind::BrokenPipe) => {}
            Err(error) => panic!("unexpected error awaiting failed stream close: {error}"),
        }
        received
    }

    #[test]
    fn only_reduced_pending_output_extends_the_payload_progress_deadline() {
        let at = std::time::Instant::now();
        let mut progress = at;
        let mut wire = super::WireProgress::default();
        wire.observe(Some(1000), &mut progress, at + Duration::from_secs(1));
        wire.observe(Some(1000), &mut progress, at + Duration::from_secs(2));
        assert_eq!(progress, at, "ACK probes without delivered data are not progress");
        wire.observe(Some(900), &mut progress, at + Duration::from_secs(3));
        assert_eq!(progress, at + Duration::from_secs(3));
        wire.observe(None, &mut progress, at + Duration::from_secs(4));
        wire.observe(Some(800), &mut progress, at + Duration::from_secs(5));
        assert_eq!(progress, at + Duration::from_secs(3), "a failed query cannot invent progress");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_upload_is_not_completed_while_payload_remains_in_the_kernel_send_queue() {
        use std::os::fd::AsRawFd;
        let content = vec![0x5a; 128 * 1024];
        let path = scratch_file("unacknowledged-upload", &content);
        let dir = path.parent().unwrap().to_path_buf();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let receive_buffer: libc::c_int = 4 * 1024;
        // SAFETY: a valid initialized int and live socket descriptor; the
        // accepted socket inherits the receive window negotiated at SYN.
        unsafe {
            assert_eq!(libc::setsockopt(listener.as_raw_fd(), libc::SOL_SOCKET, libc::SO_RCVBUF,
                (&raw const receive_buffer).cast(), std::mem::size_of_val(&receive_buffer) as _), 0);
        }
        let sender = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut peer, _) = listener.accept().unwrap();
        let send_buffer: libc::c_int = 1024 * 1024;
        // SAFETY: the initialized option has the exact socklen expected.
        unsafe {
            assert_eq!(libc::setsockopt(sender.as_raw_fd(), libc::SOL_SOCKET, libc::SO_SNDBUF,
                (&raw const send_buffer).cast(), std::mem::size_of_val(&send_buffer) as _), 0);
        }
        peer.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let probe = sender.try_clone().unwrap();
        let counter = Arc::new(AtomicU64::new(0)); let sent = counter.clone();
        let (done, result) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let outcome = super::stream_file(super::with_deadlines(sender).unwrap(),
                std::fs::File::open(&path).unwrap(), &path, &sent, &AtomicBool::new(false), None);
            done.send(outcome).unwrap();
        });
        peer.write_all(&0_u64.to_le_bytes()).unwrap();
        let until = std::time::Instant::now() + Duration::from_secs(5);
        while counter.load(Ordering::Relaxed) < content.len() as u64 {
            assert!(std::time::Instant::now() < until, "sender did not fill its payload buffer");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(super::pending_output(&probe).unwrap() > 0);
        assert!(matches!(result.recv_timeout(Duration::from_millis(100)), Err(mpsc::RecvTimeoutError::Timeout)),
            "upload completed before the peer acknowledged buffered payload");
        drop(probe);
        let mut received = Vec::new(); peer.read_to_end(&mut received).unwrap();
        assert_eq!(result.recv_timeout(Duration::from_secs(5)).unwrap().unwrap(), content.len() as u64);
        worker.join().unwrap(); assert_eq!(received, content);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_receiver_that_never_drains_payload_still_hits_its_bounded_deadline() {
        use std::os::fd::AsRawFd;
        let content = vec![0x5a; 1024 * 1024];
        let path = scratch_file("no-payload-drain", &content);
        let dir = path.parent().unwrap().to_path_buf();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let buffer: libc::c_int = 4 * 1024;
        // SAFETY: same inherited receive-window fixture as above.
        unsafe {
            assert_eq!(libc::setsockopt(listener.as_raw_fd(), libc::SOL_SOCKET, libc::SO_RCVBUF,
                (&raw const buffer).cast(), std::mem::size_of_val(&buffer) as _), 0);
        }
        let sender = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut peer, _) = listener.accept().unwrap();
        sender.set_write_timeout(Some(Duration::from_millis(20))).unwrap();
        let (done, result) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            done.send(super::stream_file_with_timeout(sender, std::fs::File::open(&path).unwrap(),
                &path, &AtomicU64::new(0), &AtomicBool::new(false), None, Duration::from_millis(200))).unwrap();
        });
        peer.write_all(&0_u64.to_le_bytes()).unwrap();
        let error = result.recv_timeout(Duration::from_secs(5)).unwrap().unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        worker.join().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut received = Vec::new();
        let reset = peer.read_to_end(&mut received).expect_err("failed upload must discard queued data, not linger in FIN-WAIT");
        assert!(matches!(reset.kind(), std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted));
        assert!(received.len() < content.len());
        drop(peer);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Play the downloader: accept the F connection, drain the PeerInit frame
    /// and the raw token, then reply with the 8-byte START_DOWNLOAD offset.
    /// Returns the connected stream and the token the uploader announced.
    fn accept_and_send_offset(listener: &TcpListener, offset: u64) -> (TcpStream, u32) {
        let (mut stream, token) = accept_transfer(listener);
        stream.write_all(&offset.to_le_bytes()).unwrap();
        stream.flush().unwrap();
        (stream, token)
    }
    fn accept_transfer(listener: &TcpListener) -> (TcpStream, u32) {
        let (mut stream, _) = listener.accept().unwrap();
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).unwrap();
        let mut payload = vec![0u8; u32::from_le_bytes(len_buf) as usize];
        stream.read_exact(&mut payload).unwrap();
        let mut token = [0u8; 4];
        stream.read_exact(&mut token).unwrap();

        (stream, u32::from_le_bytes(token))
    }

    fn scratch_file(name: &str, content: &[u8]) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("soulseek-upload-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("probe.bin");
        std::fs::write(&path, content).unwrap();
        path
    }

    fn spawn_uploader(
        listener: &TcpListener,
        path: std::path::PathBuf,
        token: u32,
        cancel: Arc<AtomicBool>,
    ) -> (
        Arc<AtomicU64>,
        std::thread::JoinHandle<std::io::Result<u64>>,
    ) {
        let port = u32::from(listener.local_addr().unwrap().port());
        let bytes_sent = Arc::new(AtomicU64::new(0));
        let sent_counter = bytes_sent.clone();
        let uploader = std::thread::spawn(move || {
            serve_file(
                "127.0.0.1",
                port,
                "me",
                token,
                &path,
                &sent_counter,
                &cancel,
            )
        });
        (bytes_sent, uploader)
    }

    #[test]
    fn serve_file_streams_the_file_over_an_f_connection() {
        let content: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        let path = scratch_file("stream", &content);

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let dir = path.parent().unwrap().to_path_buf();
        let (bytes_sent, uploader) = spawn_uploader(&listener, path, 777, Arc::default());

        let (mut stream, token) = accept_and_send_offset(&listener, 0);
        assert_eq!(token, 777);

        let mut received = vec![0; content.len()];
        stream.read_exact(&mut received).unwrap();
        assert_eq!(received, content);

        drop(stream);
        assert_eq!(uploader.join().unwrap().unwrap(), 4096);
        assert_eq!(bytes_sent.load(Ordering::Relaxed), 4096);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn empty_upload_accepts_nicotine_close_without_a_resume_offset() {
        for reverse in [false, true] {
            let path = scratch_file(&format!("empty-eof-{reverse}"), b"");
            let dir = path.parent().unwrap().to_path_buf();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let (sender, mut receiver) = {
                let sender = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
                let (receiver, _) = listener.accept().unwrap();
                (sender, receiver)
            };
            let bytes_sent = Arc::new(AtomicU64::new(0));
            let counter = bytes_sent.clone();
            let uploader = std::thread::spawn(move || {
                if reverse {
                    super::serve_pierced(sender, 1780, &path, &counter, &AtomicBool::new(false))
                } else {
                    super::serve_direct(
                        sender,
                        "me",
                        1780,
                        &path,
                        &counter,
                        &AtomicBool::new(false),
                    )
                }
            });
            if !reverse {
                let mut length = [0; 4];
                receiver.read_exact(&mut length).unwrap();
                let mut init = vec![0; u32::from_le_bytes(length) as usize];
                receiver.read_exact(&mut init).unwrap();
            }
            let mut token = [0; 4];
            receiver.read_exact(&mut token).unwrap();
            assert_eq!(u32::from_le_bytes(token), 1780);
            // Actual Nicotine+ behavior: token matched and empty file finalized,
            // then close without FileOffset. Never count a resume prefix/payload.
            drop(receiver);
            assert_eq!(uploader.join().unwrap().unwrap(), 0);
            assert_eq!(bytes_sent.load(Ordering::Relaxed), 0);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn offsetless_close_still_rejects_nonempty_files_and_partial_offsets() {
        for (name, content, offset_prefix) in [
            ("nonempty-eof", b"payload".as_slice(), b"".as_slice()),
            (
                "empty-partial-offset",
                b"".as_slice(),
                b"\0\0\0\0".as_slice(),
            ),
        ] {
            let path = scratch_file(name, content);
            let dir = path.parent().unwrap().to_path_buf();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let (bytes_sent, uploader) = spawn_uploader(&listener, path, 1781, Arc::default());
            let (mut receiver, _) = accept_transfer(&listener);
            receiver.write_all(offset_prefix).unwrap();
            drop(receiver);
            assert_eq!(
                uploader.join().unwrap().unwrap_err().kind(),
                std::io::ErrorKind::UnexpectedEof
            );
            assert_eq!(bytes_sent.load(Ordering::Relaxed), 0);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn empty_offsetless_close_does_not_override_cancellation() {
        let path = scratch_file("empty-cancelled-eof", b"");
        let dir = path.parent().unwrap().to_path_buf();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let (bytes_sent, uploader) = spawn_uploader(&listener, path, 1782, cancel.clone());
        let (receiver, _) = accept_transfer(&listener);
        cancel.store(true, Ordering::Relaxed);
        drop(receiver);
        assert_eq!(
            uploader.join().unwrap().unwrap_err().kind(),
            std::io::ErrorKind::Interrupted
        );
        assert_eq!(bytes_sent.load(Ordering::Relaxed), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn serve_file_accepts_valid_resume_offsets() {
        let content: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        for offset in [1000, content.len()] {
            let path = scratch_file(&format!("valid-offset-{offset}"), &content);
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let dir = path.parent().unwrap().to_path_buf();
            let (bytes_sent, uploader) = spawn_uploader(&listener, path, 779, Arc::default());

            let (mut stream, _) = accept_and_send_offset(&listener, offset as u64);
            let mut received = vec![0; content.len() - offset];
            stream.read_exact(&mut received).unwrap();
            assert_eq!(received, content[offset..]);
            drop(stream);

            assert_eq!(
                uploader.join().unwrap().unwrap(),
                (content.len() - offset) as u64
            );
            assert_eq!(bytes_sent.load(Ordering::Relaxed), content.len() as u64);
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn serve_file_rejects_an_offset_past_end_of_file() {
        let content: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        let path = scratch_file("invalid-offset", &content);

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let dir = path.parent().unwrap().to_path_buf();
        let (bytes_sent, uploader) = spawn_uploader(&listener, path, 780, Arc::default());

        let (mut stream, _) = accept_and_send_offset(&listener, 4097);
        let received = read_closed(&mut stream);
        assert!(received.is_empty());

        let error = uploader.join().unwrap().unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(
            bytes_sent.load(Ordering::Relaxed),
            0,
            "an invalid prefix must not be counted as uploaded"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_downloader_that_never_closes_still_leaves_the_upload_delivered() {
        // The file is on the wire; whether the peer closes, or its FIN is
        // eaten by a NAT, it has had every byte. Waiting for a close that may
        // never come used to fail the transfer and hold its upload slot,
        // which is what starved a queue of everyone behind it.
        let content: Vec<u8> = (0..2048u32).map(|i| (i % 251) as u8).collect();
        let path = scratch_file("no-close", &content);
        let dir = path.parent().unwrap().to_path_buf();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = u32::from(listener.local_addr().unwrap().port());
        let (result_tx, result_rx) = mpsc::channel();

        let uploader = std::thread::spawn(move || {
            let result = serve_file(
                "127.0.0.1",
                port,
                "me",
                783,
                &path,
                &AtomicU64::new(0),
                &AtomicBool::new(false),
            );
            let _ = result_tx.send(result);
        });

        let (mut stream, _) = accept_and_send_offset(&listener, 0);
        let mut received = vec![0; content.len()];
        stream.read_exact(&mut received).unwrap();
        assert_eq!(received, content);

        // Hold the socket open past the grace period and say nothing.
        let served = result_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("delivery must finish the upload, not a wait for a close")
            .expect("a delivered file is a finished upload");
        assert_eq!(served, content.len() as u64);

        drop(stream);
        let _ = uploader.join();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cancellation_interrupts_a_partial_resume_offset_without_sending_payload() {
        let path = scratch_file("cancel-offset", b"payload");
        let dir = path.parent().unwrap().to_path_buf();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let (_, uploader) = spawn_uploader(&listener, path, 778, cancel.clone());
        let (mut stream, _) = accept_transfer(&listener);
        stream.write_all(&[0; 3]).unwrap();
        let at = std::time::Instant::now();
        cancel.store(true, Ordering::Relaxed);
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let received = read_closed(&mut stream);
        assert!(received.is_empty());
        assert!(at.elapsed() < Duration::from_secs(2));
        assert_eq!(
            uploader.join().unwrap().unwrap_err().kind(),
            std::io::ErrorKind::Interrupted
        );
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn reverse_upload_offset_wait_is_cancellable_too() {
        let path = scratch_file("cancel-reverse", b"payload");
        let dir = path.parent().unwrap().to_path_buf();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let worker = std::thread::spawn(move || {
            super::serve_pierced(stream, 9, &path, &AtomicU64::new(0), &stop)
        });
        let mut token = [0; 4];
        peer.read_exact(&mut token).unwrap();
        assert_eq!(u32::from_le_bytes(token), 9);
        cancel.store(true, Ordering::Relaxed);
        let bytes = read_closed(&mut peer);
        assert!(bytes.is_empty());
        assert_eq!(
            worker.join().unwrap().unwrap_err().kind(),
            std::io::ErrorKind::Interrupted
        );
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn resume_offset_retries_poll_timeouts_but_bounds_the_whole_handshake() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(10)))
            .unwrap();
        let feeder = std::thread::spawn(move || {
            for b in 12345_u64.to_le_bytes() {
                std::thread::sleep(Duration::from_millis(25));
                peer.write_all(&[b]).unwrap();
            }
        });
        assert_eq!(
            super::read_resume_offset(
                &mut stream,
                &AtomicBool::new(false),
                Duration::from_secs(2),
                false
            )
            .unwrap(),
            12345
        );
        feeder.join().unwrap();
        let mut peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(10)))
            .unwrap();
        peer.write_all(&[0; 3]).unwrap();
        let at = std::time::Instant::now();
        assert_eq!(
            super::read_resume_offset(
                &mut stream,
                &AtomicBool::new(false),
                Duration::from_millis(120),
                false
            )
            .unwrap_err()
            .kind(),
            std::io::ErrorKind::TimedOut
        );
        assert!(at.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn cancelling_a_receiver_with_a_full_socket_buffer_releases_the_stream_promptly() {
        let path = scratch_file("cancel-blocked-data", b"");
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(64 * 1024 * 1024)
            .unwrap();
        let dir = path.parent().unwrap().to_path_buf();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let (counter, uploader) = spawn_uploader(&listener, path, 779, cancel.clone());
        let (stream, _) = accept_and_send_offset(&listener, 0);
        std::thread::sleep(Duration::from_millis(750));
        let sent = counter.load(Ordering::Relaxed);
        assert!(sent > 0 && sent < 64 * 1024 * 1024);
        let at = std::time::Instant::now();
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(
            uploader.join().unwrap().unwrap_err().kind(),
            std::io::ErrorKind::Interrupted
        );
        assert!(at.elapsed() < Duration::from_secs(2));
        drop(stream);
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn our_own_speed_limit_wait_does_not_use_the_peer_stall_budget() {
        let path = scratch_file("slow-limit", b"abc");
        let dir = path.parent().unwrap().to_path_buf();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let limits = Arc::new(crate::utils::bandwidth::Bandwidth::default());
        limits.configure(1, 0, vec![]);
        let bandwidth = limits.for_user("slow");
        let at = std::time::Instant::now();
        let worker = std::thread::spawn(move || {
            super::stream_file_with_timeout(
                super::with_deadlines(stream).unwrap(),
                std::fs::File::open(&path).unwrap(),
                &path,
                &AtomicU64::new(0),
                &AtomicBool::new(false),
                Some(&bandwidth),
                Duration::from_millis(200),
            )
        });
        peer.write_all(&0_u64.to_le_bytes()).unwrap();
        let mut content = Vec::new();
        peer.read_to_end(&mut content).unwrap();
        assert_eq!(content, b"abc");
        assert_eq!(worker.join().unwrap().unwrap(), 3);
        assert!(at.elapsed() >= Duration::from_secs(1));
        assert_eq!(limits.payload_bytes(), 3);
        let _ = std::fs::remove_dir_all(dir);
    }
}
