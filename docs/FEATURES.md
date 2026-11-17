# Feature coverage

RustySoul is a beta Linux Soulseek daemon with a browser UI and API. Features below have automated coverage; beta testing across more peers, routers and distributions remains useful. [Testing](TESTING.md) distinguishes fixtures from independent-client tests.

| Area | Available features |
| --- | --- |
| Accounts | First-login administrator replacement, eight-character password minimum, persistent protected sessions, UI-managed verified Soulseek credentials, startup connection |
| Search | Streaming results, lossless/lossy/all filters, user/folder grouping, speed/presence, sorting, wishlist, repeat/edit searches, batch selection and infinite scrolling |
| People | User search, own/remote share trees, profiles/pictures, presence, buddies, likes/dislikes discovery |
| Transfers | Persistent download/upload queues, total/per-user slots and rates, retries, priorities/manual ordering, bulk selection/cancellation, grouped infinite scrolling, history clearing, completed audio playback |
| Library | Multiple mounted roots, folder picker, incremental indexing, filesystem hooks, startup preference and live progress |
| Network | Configurable peer port, direct/reverse connections, UPnP discovery/renewal/removal, reachability diagnostics, bridge publication guidance |
| Chat | Multiple room tabs, unread indicators, private messages, room boards/private access, compatible Unicode emoji/emoticon display, consecutive-message grouping |
| Statistics | Per-user volumes, all-time totals/ratio, compact historical traffic buckets with graph direction toggles |
| Sharing | Intuitive configurable leech protection, profile/public share settings, privilege status and purchase link |
| Operations | Logs/activity, status/metrics, themes, scoped API keys, database maintenance, encrypted backup/restore, native systemd installation |

Detailed guides: [API](API.md), [transfer reliability](TRANSFER_RELIABILITY.md), [queue controls](TRANSFER_QUEUE.md), [storage protection](STORAGE_PROTECTION.md), [discovery](USER_DISCOVERY.md), [privacy/security](SECURITY.md), [maintenance](MAINTENANCE.md), [backup](BACKUPS.md), [monitoring](MONITORING.md).

The supplied independent-client test matrices cover real Soulfind, slskd and Nicotine+ behavior. Findings in other clients are documented in [interoperability](INTEROPERABILITY.md) and [Soulfind/slskd](SOULFIND_SLSKD.md); repairing those implementations is outside this project's scope. See [remaining work](TODO.md) for beta limitations.
