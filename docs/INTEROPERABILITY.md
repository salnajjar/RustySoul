# Independent client interoperability

`scripts/verify-nicotine-interop.py` runs an external, unmodified Nicotine+ 3.4.0.dev2 core and a compiled RustySoul daemon as separate clients. Only the central login/address/rendezvous server is synthetic. Both clients perform their own real peer framing, compressed share listings, queue negotiation, file sockets and filesystem writes. Every account, share and destination is temporary and every connection stays on loopback. The test never uses production data or the public Soulseek network.

Run on Linux or WSL with Python 3.11+ and a Linux executable:

```sh
python3 scripts/verify-nicotine-interop.py --binary target/release/rustysoul --nicotine ../nicotine-plus
python3 scripts/verify-nicotine-interop.py --binary target/release/rustysoul --nicotine ../nicotine-plus --fragment
```

The default external checkout path is `../nicotine-plus`; override it with `--nicotine /path/to/nicotine-source`. `--mode direct` or `--mode reverse` selects one case. This dependency is used only by the verification runner and is not bundled into the application or runtime image.

Each case asserts:

- Bidirectional browse through the actual clients, with exact sizes, Unicode filenames and empty files.
- Byte-identical downloads/uploads in both directions, including zero-byte completion recorded by RustySoul's uploader.
- A real queue-position response while upload admission is paused, followed by completion when a slot opens.
- A partial RustySoul download, pause, network disconnect/reconnect and exact resumption offset observed by Nicotine+'s uploader.
- A partial Nicotine+ download, its own pause/retry controls, and its exact outgoing resume offset.
- Three queued cancellation/resume cycles followed by exact completion.

Direct cases withhold brokered requests, proving direct connections work independently. Reverse cases advertise port zero and forward rendezvous requests to the real peer listeners, including control and file connections. The delayed/fragmented cases pass peer-stream prefixes through a loopback proxy that splits them into seven-byte writes with a short delay; the remaining file payload streams normally. This is a functional interoperability check, not an extended soak.

The supplied Nicotine+ snapshot closes an incoming connection when its PeerInit frame is incomplete in `_process_peer_init_input`. The proxy therefore keeps that first frame together only toward Nicotine+; incoming RustySoul handshakes and subsequent stream prefixes remain fragmented. The reference source is unchanged. This exception does not establish that Nicotine+ handles arbitrary handshake fragmentation.

This is an external-client compatibility note, not an outstanding RustySoul defect. Repairing Nicotine+ is outside our implementation scope. The coalescing accommodation belongs to the test proxy; it is not a production workaround built into RustySoul.

The tests exposed two RustySoul defects. Nicotine+ finalizes an empty download after matching the file token and closes without sending an offset; RustySoul now accepts that EOF only for a genuinely empty file with no partial offset received. Non-empty transfers, truncated offsets and cancellation retain their existing failure/cancellation behavior. Disconnecting a paused download also used to leave its file worker/socket alive; per-attempt disconnect signalling now releases it while keeping the prefix and prevents a retired attempt from being resumed as though its socket were still usable. Active uploads receive their cancellation signal when the session disconnects.

Actual Soulfind/slskd deployment and its four independent transfer cases are now covered in [SOULFIND_SLSKD.md](SOULFIND_SLSKD.md). All 87 SDK server cases also ran with real Soulfind servers and no skips. The SDK harness remains optional in ordinary local builds; the new pod runner requires it and rejects skip messages. Other client versions, public server behavior, real NAT/firewall combinations and arbitrary mid-stream network faults remain separate coverage.
