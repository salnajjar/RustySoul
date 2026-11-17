# User discovery testing milestone

Refresh the browser after upgrading. Valid administrator sessions survive browser refreshes and daemon restarts.

1. Open **Browse users**. Enter an exact Soulseek username and choose **Find user**. Soulseek does not provide a global substring username directory here. A recognized offline user and a nonexistent username have different states.
2. Click a username in search results, a transfer, chat or a room member list. Its dialog offers **Browse shares**, **Message**, **Add/Remove buddy** and **Refresh details**. Server share counts/speed and peer description/queue details arrive separately. A profile timeout does not erase the server statistics.
3. Browse shares. Use the folder tree, folder/file filters and paging. Each request returns up to 100 files or folders. The full listing is cached in SQLite, with a configurable default limit of 1,000,000 validated files per user. Saved library tabs have no fixed user-count ceiling; close a tab to remove its cached listing. Refresh retrieves the peer's current shares. Remote peers may be offline, decline browsing or be unreachable in both directions; the listing then shows a retryable error.
4. Choose **Browse my shares**, including while disconnected. This uses the cached local index. For changes made while the daemon was stopped, use manual re-indexing or allow automatic reconciliation to catch up.
5. Add a buddy, restart and verify that the buddy remains. Presence is fetched again after connecting. Join a room and expand **Room members**; filter members and open a profile from their username. Rosters display up to 2,000 members. Up to 512 users are watched for live status; explicit lookups and buddies have priority.
6. In **Settings → Network settings**, check the startup connection preference. New settings default to enabled. In **Settings → File locations & library**, check startup indexing, which defaults to disabled. Cached shares load without a full startup scan; hooks and 60-second reconciliation continue to detect changes. Both preferences persist.

Presence labels are **Online**, **Away**, **Offline**, **Not found**, **Checking** or **Unknown**. Cached search results are not proof a user is still online. Disconnecting clears current remote presence to Unknown. The UI shows unknown data explicitly rather than inventing an online status or zero statistics.

The current application implements user lookup, public profile interests, buddies, room rosters, paged browsing, ignore/ban controls, wishlist/scoped searches, recommendation discovery, retry policies and private-room administration/boards. Broader real-client interoperability remains tracked in [FEATURES.md](FEATURES.md).

Current file validation omits zero-length entries from browsable/downloadable lists; they still count in the local inventory. The deployed library has one such entry (94,029 indexed, 94,028 transferable). This is recorded as a remaining file-browser limitation.

## Shared dislikes

The Discover page has **Shared dislikes**, your saved dislike chips and a like/dislike selector for an entered topic. Dislike chips in user profiles open the same workflow. Likes retain their existing server-backed discovery behavior.

Shared-dislike discovery confirms the selected item in other people's public **dislikes**, then shows **People who also dislike this**, **Things these people like**, and **Other things they dislike**. A person who only likes the topic is excluded. Ratings count matching profiles, rather than implying a global server popularity score. Topics are trimmed and normalized to lowercase, with the existing 80-byte validation limit.

The Soulseek protocol provides related topic users and public interest lists, but no global inverse-dislike directory. RustySoul therefore samples candidate users from related-topic/similar-user replies, buddies, known users, joined-room members and cached public profiles, verifies their public dislikes, and displays checked/matched counts. Own, ignored and banned accounts are excluded. No temporary like is advertised and no nonstandard protocol extension is sent. Protocol references: [Nicotine+ server codes 57, 110 and 112](https://nicotine-plus.org/doc/SLSKPROTOCOL.html).

Fresh public profiles (up to 256 cached profiles, 15-minute freshness) can be reused. Each run queues at most 512 candidates and requests at most 64 fresh profiles, two per 500 milliseconds. Replies and candidate sources get ten-second budgets; the overall run ends after forty seconds. The UI reports timeouts/limits and makes clear that this is a checked sample. Refresh after completion can check more candidates while fresh profiles are reused. Disconnected use can open saved results or derive a sample from fresh cached profiles; a network refresh requires connecting. Results and public profiles use the existing bounded SQLite caches and survive restart.
