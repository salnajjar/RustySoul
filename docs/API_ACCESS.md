# External API access

Open **Settings > API access** after completing the initial administrator credential change. External integration keys are disabled by default. Enter the current administrator password to enable them, then create a named key with **Read only** or **Read and control** permissions. The UI offers 30-day, 90-day (default), one-year or no-expiry keys. Save the secret when it appears: it is shown only once. Create a separate key for each integration; at most 32 keys are retained, including expired keys until revoked.

The UI lists each key's permissions, expiry and last-used timestamp. Revoke a key or all keys at any time. Disabling external API access immediately suspends all keys, including existing event streams; re-enabling restores access for unrevoked, unexpired keys. Browser sign-in continues to work. Changing administrator credentials or restoring a backup permanently revokes the integration keys and disables external access; create new keys afterwards.

Use the base URL shown on this page and send the key only in an HTTP header:

```sh
curl -H "Authorization: Bearer YOUR_API_KEY" https://YOUR_SERVER/api/v1/status
```

Use HTTPS for remote integrations. Keys are not accepted through cookies, query parameters or URLs. Existing same-origin protections remain: this setting does not enable CORS or change the HTTP bind address. Configure listening addresses in the deployment TOML and retain your TLS proxy. A named key never bypasses the mandatory initial account change.

## Permissions

Read-only keys can read `/status`, `/metrics`, `/events`, `/logs`, `/profile`, `/traffic`, `/folders`, `/users/picture`, `/downloads/file`, `/browse/files`, `/browse/tree`, `/rooms/entries`, `/transfers/{kind}` and `/searches/{id}/results`, relative to `/api/v1`. This includes retained chats, file metadata and downloaded file contents; share keys only with trusted integrations.

Read-and-control keys additionally allow POST `/commands`, `/downloads/batch` and `/profile`. They can initiate searches and transfers, send messages, and change application, network, sharing, file-location and Soulseek-account settings through ordinary validated commands. These are broad control permissions, not a download-only role.

Administrator account/session routes, API-key management, encrypted backup/restore and database maintenance require an administrator browser session and are excluded from both scopes. Newly added routes are denied to named keys unless explicitly added to the allowlist. Existing request bounds, command rate limits and queue admission apply to keys as well. HTTP 401 means invalid/expired/revoked/disabled credentials; 403 means insufficient permissions. A 202 command response acknowledges admission, not completion.

## Management API

These routes require a signed-in administrator session (and the normal browser mutation header when using its cookie):

- GET `/api/v1/api-access`: `{enabled, keys, maximum_keys}`. Keys contain `id`, `name`, `scope`, UTC-second `created_at`, nullable `expires_at` and `last_used_at`. No secrets/hashes are returned.
- POST with `{action:"set_enabled",enabled:true,current_password:"..."}`. Disabling does not require the password again.
- POST with `{action:"create",name:"Monitoring",scope:"read_only",expires_in_days:90,current_password:"..."}` returns `{key,token}` once. Scope can also be `control`; expiry 1..365 days or 0 for no expiry.
- POST with `{action:"revoke",id:"KEY_UUID"}` or `{action:"revoke_all"}`.

## Storage and recovery

Secrets use 256 random bits and only their SHA-256 hashes are stored in private `api-access.sqlite3` alongside the main application database. The auxiliary SQLite file is mode 0600 on Linux. It is bound to this installation's identity and current administrator credentials. Normal restarts, upgrades and database maintenance retain keys; installation/credential changes revoke them. Authentication and revocation use a bounded in-memory cache, with coalesced last-used writes approximately once a minute. A key listing flushes current usage; an abrupt shutdown may lose the newest usage timestamp, not a revocation.

Portable `.rsb` backups exclude this auxiliary database, so restoring an old application backup cannot revive previously revoked integration keys. Full deployment backups should preserve it with the main database, key and configuration when exact installation recovery is required. Main application schema remains 11; the separate API-key store has its own schema version.
