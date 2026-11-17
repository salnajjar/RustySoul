# Soulseek privileges

Your network account's privilege status appears in the top-bar star button and Settings > Accounts > Soulseek privileges. The star opens that account section. Status distinguishes unknown, checking, active, confirmed inactive, locally elapsed and unavailable states. Disconnecting, changing account or restarting clears live observations; RustySoul asks again after login rather than trusting a saved balance.

The server's remaining seconds are shown as time remaining, the last successful check and an estimated local-time expiry. Time counts down using a monotonic daemon clock; the expiry is an estimate based on the daemon's wall clock and the server reply. A timeout retains and labels the last known balance. A zero balance is only confirmed by an actual server response; an unanswered query never becomes "no active privileges".

## Purchasing or extending

Get/Extend Soulseek privileges opens the [official Soulseek page](https://www.slsknet.org/qtlogin.php) in a separate tab with the **connected Soulseek username**, safely URL-encoded. Confirm that username on the official page. The administrator account and Soulseek password are not included. Copy Soulseek username, an alternative official donation page and official donation-help links are available. Prices and payment methods come from Soulseek's site; RustySoul does not hard-code them or handle payments.

Returning to RustySoul triggers a fresh check and starts a bounded activation watch. During that watch it checks at most once every 30 seconds for five minutes; an observed increase ends the watch early. Soulseek's [confirmation page](https://www.slsknet.org/thankyou.html) says activation can take a few minutes. A link click alone never marks a purchase successful. You can refresh again if processing takes longer. Purchase links are offered for official `slsknet.org` servers only; custom-server accounts are not directed to credit an unrelated official-network account.

## Checks and API

Login starts a check automatically. Connected accounts are normally checked every five minutes. Requests time out after 20 seconds; unavailable status retries after a minute, and elapsed positive balances are rechecked after 30 seconds. Manual and return-to-tab checks coalesce while pending and within a ten-second cooldown. All scheduling runs on the existing control loop, with no new thread, database history or schema change.

`POST /api/v1/commands` accepts `{"type":"check_privileges"}` and optional `"watch_for_activation":true`. It uses normal administrator authentication, first-login/CSRF checks, rate limits and asynchronous command admission. `/status` and SSE expose `privileges` with `official_server`, `remaining_seconds`, `checked_at`, `expires_at`, `pending`, `watching` and `error`. Unknown times are null. These values concern the configured Soulseek account, not the administrator or a remote user's balance. Raw status is live and is not persisted in SQLite or restored from backups.

The protocol uses Soulseek server code 92: an empty request and a u32 remaining-seconds reply. The SDK exposes a revision for every reply, including successive zero values, so a successful refresh cannot remain stuck waiting for a changed number. Failed actor sends report disconnection. Existing privileged-peer upload ordering remains in force.
