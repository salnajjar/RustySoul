# Network settings

Settings has dedicated Accounts, File locations & library, Network settings, Appearance, and Status & privacy tabs. Network settings manages incoming TCP connections, transfer counts, speed budgets, UPnP, automatic reconnect, startup connection and connected-peer limits. Changes persist in SQLite. Speed limits apply to current transfers; port changes require disconnecting first.

Linux host networking is the default. The selected transfer port listens directly on the server, with no container port publication to update. This is the behaviour described by [Docker host networking](https://docs.docker.com/engine/network/drivers/host/). The daemon still runs as UID 10001 without capabilities and with a read-only root filesystem. Host firewall and router rules must permit the selected TCP port. RustySoul does not modify the host firewall. HTTP defaults to loopback; use an HTTPS reverse proxy for remote access.

For bridge networking, use the separate compose.bridge.yaml file. Set RUSTYSOUL_TRANSFER_PORT in .env before installation (default 2234), then run docker compose -f compose.bridge.yaml up -d --build. The same port is used for the listener and published TCP mapping. Bridge publication is fixed at container creation: the UI shows that port as read-only, and the API rejects incompatible changes. Edit the deployment value and recreate the container to change it. A saved port from an older deployment is reconciled to the explicitly published port at startup. Automatic UPnP is disabled in bridge mode; use manual forwarding or host networking.

## Transfer counts and speed budgets

Defaults: 2 uploads and 4 downloads in total, with 1 transfer per user in either direction. Total upload slots range from 0 to 16, download slots from 0 to 32. Zero pauses new work; lowering a count lets existing reservations finish. Downloads waiting locally are saved before negotiation and start automatically as slots become available. A busy user's waiting files do not block other eligible users. Download reservations include accepted offers, active transfers and paused sockets. Requests waiting in remote queues do not occupy local download slots, so other users can fill available capacity. At most one unaccepted remote request per user is outstanding. Admission checks enforce total/per-user limits atomically; a locally declined offer returns to waiting without consuming a retry attempt. Accepted offers expire after 30 seconds without a file connection. Cancelling or finishing a reservation frees a slot. See TRANSFER_QUEUE.md for selection, priority and ordering. Interrupted transfers still require explicit resume after a restart; unstarted local waiting entries can begin on the next connection.

Upload and download speed limits each provide a total budget, a default budget for each user, and optional individual username overrides. Budgets are shared across all simultaneous files: two files from one user do not receive two per-user allowances. Total and user caps apply together. Named overrides replace that user's default cap while leaving the total cap in force. Values are KiB/s (1,024 bytes), with zero meaning unlimited. Limits apply to payload bytes; protocol messages and handshakes remain responsive. A short burst of at most one tenth of a second, capped at 64 KiB, allows efficient I/O. Unused read/write allowance is refunded; cancellation and changed limits wake blocked transfers. The resumed prefix is excluded from the upload speed calculation.

For example, total upload 1,000 KiB/s and per-user upload 200 KiB/s allows each peer up to 200 KiB/s while all peers together remain within 1,000 KiB/s. Per-user and total limits can also be used independently by leaving the other value at zero.

## HTTP binding

Compose mounts deploy/container.toml; bridge Compose mounts deploy/container.bridge.toml. Native installation uses /etc/rustysoul-native/rustysoul.toml. Examples:

```toml
# Local reverse proxy or SSH tunnel only:
bind = "127.0.0.1:8484"

# Or choose one adapter's address:
# bind = "192.0.2.10:8484"

# Or listen on all IPv4 adapters, including loopback:
# bind = "0.0.0.0:8484"
# Equivalent aliases: "all:8484" or "all" (port 8484).
allow_remote = true
```

Non-loopback binding requires allow_remote = true. An adapter-specific address must belong to the daemon's network namespace; bridge containers use their own addresses, and the published host address is configured in Compose. With an adapter-specific bind, update any reverse proxy upstream that previously used loopback. Use HTTPS for remote sign-in. Network settings displays the current HTTP binding; changing it requires restarting the daemon.

## UPnP and diagnostics

UPnP uses IGD WANIPConnection/WANPPPConnection services. While connected, RustySoul requests matching internal/external TCP ports with a one-hour lease and renews every twenty minutes. Routers supporting only permanent rules receive a zero-duration lease. RustySoul removes its own rule on disable, disconnect, port change or graceful shutdown. An installation identifier persisted in SQLite identifies owned mappings; port, target address and description are checked before changes. Conflicting or unverifiable rules are left untouched. Failed operations retry and report their state. A permanent rule can remain after an unclean shutdown until a later cleanup or manual router removal.

Check router & connection performs discovery without enabling forwarding when UPnP is off. Diagnostics show the gateway, external IP, mapping/lease and local listener state. A non-public external IP produces an upstream NAT/CGNAT warning. A configured mapping does not prove Internet reachability. Validate forwarding and external reachability on your own network; support varies by gateway.

## Notifications and transfer history

Library-queued notices clear when indexing completes or fails. Other short-lived confirmations expire after six seconds. Errors can be dismissed explicitly. Transfer pages can clear completed, failed, cancelled, interrupted, timed-out or all finished history records. Active, queued and paused work remains, and downloaded files stay on disk. Download record removal persists in SQLite; upload and download history are both persistent in SQLite.

Peer control connections and file connections are separate. An established peer resetting its control socket is handled as ordinary connection closure, without failing an independent file stream. Late messages for closed actors are ignored instead of generating stream-is-None errors. Malformed protocol frames and actual file I/O failures still report errors. A remote peer with no listening port, or a connection timeout, can require server-mediated connection negotiation; local configuration cannot force another peer's router to accept a connection.

UDP transfers, NAT-PMP/PCP and managed relays/VPN configuration remain outside the implemented scope.

Established upload sockets now poll cancellation every half-second while preserving a 30-second peer stall budget. Resume-offset exchange is bounded even for partial handshakes, and intentional local rate-limit waits do not become remote timeouts. Timeout history and logs give readable peer-specific reasons; genuine local filesystem failures remain errors. Direct/reverse connection establishment retains separate 20-second limits. See TRANSFER_RELIABILITY.md.
