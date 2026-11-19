# Profile and presence

Settings → Profile controls your description, picture, interests, Away behavior and upload-speed reporting.

After opening an interest in Discover, use the thumbs-up **Like** or thumbs-down **Dislike** beside its heading to add it to your own profile. This works for related-topic and shared-dislike views, including while results are loading or Soulseek is disconnected. The selected button reflects the saved profile. Choose the other button to switch preference, or select the active button again to remove it. Existing interests and other profile settings are preserved; saved interests are published when connected.

Upload speed starts at 1 MiB/s until real uploaded payload is measured. A smoothed rate excludes empty idle samples and persists across restarts in one SQLite row. Old transfer resume offsets are not treated as uploaded data. A custom reported speed takes precedence; otherwise a configured total upload limit is reported, falling back to the measured rate when cleared. This is separate from the actual bandwidth limiter in Network settings. Disable reporting to send zero in search replies and after completed uploads. Your own profile displays **Not reported**. Server statistics and other clients can retain earlier averages; hiding speed cannot erase those cached values. Server reports are sent only for real completed uploads, because each report also increments the server's upload count.

Your own profile's queued-transfer count uses the entire live upload queue, including requests outside the browser preview. When disconnected it shows zero and no available slot.

Network settings enables **Automatically use simultaneous downloads offered by a user** by default. RustySoul requests the next file once an earlier offer is accepted and obeys the peer's queue and the global download limit. Peers with only one upload slot do not block downloads from other users. Disable automatic parallelism to enforce your chosen per-user maximum.

Soulseek supports Online and Away while connected. Away keeps searches and transfers working. There is no supported connected invisible status; RustySoul does not send an offline status while silently continuing transfers. See the [protocol's SetStatus documentation](https://nicotine-plus.org/doc/SLSKPROTOCOL.html).

For a new network user, choose **Create a Soulseek account** under Accounts, choose your own username and password, and confirm the password. The server creates an unused username through the normal login protocol; an existing name requires its existing password. No separate website account is needed. RustySoul verifies acceptance before encrypting/saving credentials and connecting, and reports rejection without replacing the saved account. The local RustySoul administrator login stays separate.
