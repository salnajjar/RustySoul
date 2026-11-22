# Transfer queues and selection

Downloads and Uploads share selection controls. Click a file to select it; Ctrl/Cmd-click toggles it and Shift-click selects the range in the current displayed order. Ctrl/Cmd+Shift-click adds a range. Checkboxes add/remove individual files. Ranges include loaded offscreen rows, skip completed/history entries, and respect collapsed user groups. A user's **Select all** includes its full loaded group. Filters retain selected identities until cancellation is accepted, the transfer finishes, or selection is cleared.

Use **Cancel selected** for a batch or **Cancel all** beside a username for every unfinished transfer from that user, including files hidden by filters. Completed downloads and their media stay intact. Cancellation is submitted in bounded batches; accepted batches clear their selection and a failed remainder stays selected for retry. Downloads can be cancelled while offline. Pending upload retry timers can also be cancelled while offline. This cancels work rather than deleting retained history.

## Priority and order

Open **Priority & queue** on a waiting or paused file. Choose High, Normal (the default), or Low, then Move to top/up/down/bottom within that priority. Preferences persist in SQLite across restarts and are included in encrypted backups. Editing switches the file ordering to **Local priority & queue order**, preserving user grouping and file selections. Other sort options change presentation only.

Download preferences choose which local requests RustySoul sends next. Remote users control their own upload queues; changing local priority cannot move you ahead in a remote queue. Already requested files keep their remote places. Upload preferences choose the next waiting local offer, with Soulseek privileged users retaining precedence. Accepted offers and active files continue without interruption. Queue positions reported by peers and local order are different values.

## Slot use

Download limits count accepted offers, active transfers and paused sockets. A request waiting in a remote queue does not occupy a local download slot. RustySoul can request work from other users while one peer is busy; at most one unaccepted remote request per user is outstanding. Total and per-user limits are checked atomically when an offer arrives. An offer declined solely because local slots are full returns to local waiting without spending a retry attempt. Accepted offers that never produce a file connection expire after 30 seconds and release their reservation.

A zero slot limit pauses new admissions. Lowering limits leaves already accepted/active transfers intact. Pausing an established socket retains its reservation; cancel it to free the slot. Independent peer limits and configured queue bounds still apply.
