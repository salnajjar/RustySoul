use super::*;
use crate::backup::{Cancel, Prepared};
use anyhow::Context;
use axum::extract::Multipart;
use serde::Deserialize;
use std::sync::atomic::Ordering;
use tokio::io::AsyncWriteExt;

// Shared with the outer security middleware so it cannot cancel backup work early.
pub(super) const OPERATION_TIMEOUT: Duration = Duration::from_secs(30 * 60);
#[derive(Clone)]
pub(super) struct Manager {
    pub gate: Arc<Semaphore>,
    current: Arc<Mutex<Option<Arc<Prepared>>>>,
    abort: Arc<Mutex<Option<futures_util::future::AbortHandle>>>,
}
impl Default for Manager {
    fn default() -> Self {
        Self {
            gate: Arc::new(Semaphore::new(1)),
            current: Arc::new(Mutex::new(None)),
            abort: Arc::new(Mutex::new(None)),
        }
    }
}
#[derive(Clone)]
pub(super) struct Permit {
    _hold: Arc<tokio::sync::OwnedSemaphorePermit>,
}
struct RequestCancel(Cancel);
impl Drop for RequestCancel {
    fn drop(&mut self) {
        self.0.0.store(true, Ordering::Relaxed);
    }
}

struct MultipartHeaders {
    pattern: Vec<u8>,
    failure: Vec<usize>,
    matched: usize,
    headers: bool,
    header_bytes: usize,
    last: u32,
    total: usize,
    initial: Vec<u8>,
    initial_at: usize,
}
impl MultipartHeaders {
    fn new(boundary: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !boundary.is_empty()
                && boundary.len() <= 70
                && boundary
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)),
            "Invalid multipart boundary"
        );
        let pattern = format!("\r\n--{boundary}").into_bytes();
        let mut failure = vec![0; pattern.len()];
        for i in 1..pattern.len() {
            let mut j = failure[i - 1];
            while j > 0 && pattern[i] != pattern[j] {
                j = failure[j - 1];
            }
            if pattern[i] == pattern[j] {
                j += 1;
            }
            failure[i] = j;
        }
        Ok(Self {
            pattern,
            failure,
            matched: 0,
            headers: true,
            header_bytes: 0,
            last: 0,
            total: 0,
            initial: format!("--{boundary}\r\n").into_bytes(),
            initial_at: 0,
        })
    }
    fn check(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.total = self.total.saturating_add(bytes.len());
        if self.total > crate::backup::MAX_UPLOAD + 8192 {
            return Err(std::io::Error::other("Backup request is too large"));
        }
        for &b in bytes {
            if self.initial_at < self.initial.len() {
                if b != self.initial[self.initial_at] {
                    return Err(std::io::Error::other(
                        "Multipart upload must start with its boundary",
                    ));
                }
                self.initial_at += 1;
                continue;
            }
            if self.headers {
                self.header_bytes += 1;
                if self.header_bytes > 8192 {
                    return Err(std::io::Error::other(
                        "Multipart field headers are too large",
                    ));
                }
                self.last = self.last.wrapping_shl(8) | u32::from(b);
                if self.last == 0x0d0a0d0a {
                    self.headers = false;
                    self.matched = 0;
                }
            } else {
                while self.matched > 0 && b != self.pattern[self.matched] {
                    self.matched = self.failure[self.matched - 1];
                }
                if b == self.pattern[self.matched] {
                    self.matched += 1;
                }
                if self.matched == self.pattern.len() {
                    self.headers = true;
                    self.header_bytes = 0;
                    self.last = 0;
                    self.matched = 0;
                }
            }
        }
        Ok(())
    }
}
pub(super) async fn gate(
    State(state): State<ApiState>,
    mut request: Request,
    next: Next,
) -> Response {
    if request.method() == axum::http::Method::GET
        && (request.uri().path().ends_with("/backups")
            || request.uri().path().ends_with("/maintenance"))
    {
        return next.run(request).await;
    }
    if crate::maintenance::pending(&state.data_dir)
        || state.data_dir.join("restore-pending").exists()
    {
        return error(
            StatusCode::CONFLICT,
            "A database restart operation is queued; wait for RustySoul to return",
        );
    }
    let Ok(permit) = state.backups.gate.clone().try_acquire_owned() else {
        return error(
            StatusCode::CONFLICT,
            "Another backup operation or download is active; try again when it finishes",
        );
    };
    let held = Permit {
        _hold: Arc::new(permit),
    };
    request.extensions_mut().insert(held.clone());
    if request.uri().path().ends_with("/backups/inspect") {
        let boundary = request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| {
                let parts = v.split(';').map(str::trim).collect::<Vec<_>>();
                if parts.len() != 2 || !parts[0].eq_ignore_ascii_case("multipart/form-data") {
                    return None;
                }
                parts[1]
                    .strip_prefix("boundary=")
                    .map(|b| b.trim_matches('"'))
            });
        let mut guard = match boundary.and_then(|b| MultipartHeaders::new(b).ok()) {
            Some(g) => g,
            None => {
                return error(
                    StatusCode::BAD_REQUEST,
                    "Choose a valid multipart backup upload",
                );
            }
        };
        let (parts, body) = request.into_parts();
        let stream = body.into_data_stream().map(move |chunk| {
            let bytes = chunk.map_err(std::io::Error::other)?;
            guard.check(&bytes)?;
            Ok::<_, std::io::Error>(bytes)
        });
        request = Request::from_parts(parts, Body::from_stream(stream));
    }
    let cancellable = request.method() == axum::http::Method::POST
        && (request.uri().path().ends_with("/backups")
            || request.uri().path().ends_with("/backups/inspect"));
    let response = if cancellable {
        let (abort, registration) = futures_util::future::AbortHandle::new_pair();
        *state.backups.abort.lock().unwrap() = Some(abort);
        let result = tokio::time::timeout(
            OPERATION_TIMEOUT,
            futures_util::future::Abortable::new(next.run(request), registration),
        )
        .await;
        *state.backups.abort.lock().unwrap() = None;
        match result {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => error(StatusCode::BAD_REQUEST, "Backup operation cancelled"),
            Err(_) => error(
                StatusCode::REQUEST_TIMEOUT,
                "Backup operation exceeded 30 minutes",
            ),
        }
    } else {
        next.run(request).await
    };
    drop(held);
    response
}
fn describe(p: &Prepared, restart: bool) -> serde_json::Value {
    json!({"id":p.id,"kind":if p.export{"export"}else{"restore"},"summary":p.summary,"bytes":if p.export{std::fs::metadata(p.file()).map_or(0,|m|m.len())}else{p.summary.database_bytes},"can_restart":restart})
}
pub(super) async fn status(State(s): State<ApiState>) -> Response {
    let current = s.backups.current.lock().unwrap();
    Json(json!({"busy":s.backups.gate.available_permits()==0,"ready":current.as_ref().map(|p|describe(p,s.restart.is_some())),"can_restart":s.restart.is_some(),"last_result":crate::backup::last_result(&s.data_dir)})).into_response()
}
pub(super) async fn cancel(State(s): State<ApiState>) -> Response {
    if let Some(abort) = s.backups.abort.lock().unwrap().take() {
        abort.abort();
        Json(json!({"status":"cancelling"})).into_response()
    } else {
        error(
            StatusCode::CONFLICT,
            "No cancellable backup operation is active",
        )
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Create {
    passphrase: String,
    current_password: String,
}
pub(super) async fn create(
    State(s): State<ApiState>,
    Extension(permit): Extension<Permit>,
    Json(q): Json<Create>,
) -> Response {
    let Some(auth) = s.auth.clone() else {
        return error(
            StatusCode::FORBIDDEN,
            "Administrator sign-in is required for backups",
        );
    };
    if let Err(e) = crate::backup::passphrase(&q.passphrase) {
        return error(StatusCode::UNPROCESSABLE_ENTITY, &e.to_string());
    }
    let cancel = RequestCancel(Cancel::default());
    let stop = cancel.0.clone();
    let config = s.backup_config.clone();
    let revision = *auth.revision.borrow();
    let work = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        auth.confirm(&q.current_password, || Ok(()))?;
        crate::backup::export(&config, &q.passphrase, &stop)
    })
    .await;
    match work {
        Ok(Ok(p)) => {
            if s.auth
                .as_ref()
                .is_none_or(|a| *a.revision.borrow() != revision)
            {
                return error(
                    StatusCode::UNAUTHORIZED,
                    "Account changed; sign in and try again",
                );
            }
            let response = describe(&p, s.restart.is_some());
            *s.backups.current.lock().unwrap() = Some(Arc::new(p));
            Json(response).into_response()
        }
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, &e.to_string()),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "Backup creation failed"),
    }
}
pub(super) async fn inspect(
    State(s): State<ApiState>,
    Extension(permit): Extension<Permit>,
    mut form: Multipart,
) -> Response {
    if s.auth.is_none() {
        return error(
            StatusCode::FORBIDDEN,
            "Administrator sign-in is required for backups",
        );
    }
    let cancel = RequestCancel(Cancel::default());
    let stop = cancel.0.clone();
    let config = s.backup_config.clone();
    let result = async {
        let dir = crate::backup::workspace(&s.data_dir)?;
        let path = dir.path().join("input.rsb");
        let mut pass = None;
        let mut uploaded = false;
        while let Some(mut field) = form.next_field().await? {
            match field.name() {
                Some("passphrase") if pass.is_none() && !uploaded => {
                    let mut bytes = Vec::new();
                    while let Some(chunk) = field.chunk().await? {
                        anyhow::ensure!(
                            bytes.len() + chunk.len() <= 1024,
                            "Backup passphrase is too long"
                        );
                        bytes.extend_from_slice(&chunk);
                    }
                    let value = String::from_utf8(bytes)?;
                    crate::backup::passphrase(&value)?;
                    pass = Some(value);
                }
                Some("file") if pass.is_some() && !uploaded => {
                    let mut options = tokio::fs::OpenOptions::new();
                    options.write(true).create_new(true);
                    #[cfg(unix)]
                    options.mode(0o600);
                    let mut file = options.open(&path).await?;
                    let mut total = 0usize;
                    let mut checked = 0usize;
                    while let Some(chunk) = field.chunk().await? {
                        total = total.saturating_add(chunk.len());
                        anyhow::ensure!(
                            total <= crate::backup::MAX_UPLOAD,
                            "Backup file exceeds the 2 GiB limit"
                        );
                        if checked == 0 || total.saturating_sub(checked) >= 1024 * 1024 {
                            crate::backup::check_upload_space(&s.data_dir, 1024 * 1024)?;
                            checked = total;
                        }
                        file.write_all(&chunk).await?;
                    }
                    file.sync_all().await?;
                    uploaded = true;
                }
                _ => anyhow::bail!("Send one passphrase followed by one backup file"),
            }
        }
        anyhow::ensure!(uploaded, "Choose a backup file");
        let pass = pass.context("Enter the backup passphrase")?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let _input = dir;
            crate::backup::preview(&path, &pass, &config, &stop)
        })
        .await
        .context("Backup validation worker failed")?
    }
    .await;
    match result {
        Ok(p) => {
            let response = describe(&p, s.restart.is_some());
            *s.backups.current.lock().unwrap() = Some(Arc::new(p));
            Json(response).into_response()
        }
        Err(e) => error(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}
pub(super) async fn download(
    State(s): State<ApiState>,
    Path(id): Path<String>,
    Extension(permit): Extension<Permit>,
) -> Response {
    let p = s
        .backups
        .current
        .lock()
        .unwrap()
        .as_ref()
        .filter(|p| p.id == id && p.export)
        .cloned();
    let Some(p) = p else {
        return error(
            StatusCode::NOT_FOUND,
            "Backup download has expired; create it again",
        );
    };
    let file = match tokio::fs::File::open(p.file()).await {
        Ok(file) => file,
        Err(_) => return error(StatusCode::NOT_FOUND, "Backup file is unavailable"),
    };
    let length = file.metadata().await.map_or(0, |m| m.len());
    let stream = tokio_util::io::ReaderStream::with_capacity(file, 64 * 1024).map(move |chunk| {
        let _keep = (&permit, &p);
        chunk
    });
    let mut response = Body::from_stream(stream).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&length.to_string()).unwrap(),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"rustysoul-backup.rsb\""),
    );
    response
}
pub(super) async fn clear(
    State(s): State<ApiState>,
    Extension(_permit): Extension<Permit>,
) -> Response {
    s.backups.current.lock().unwrap().take();
    Json(json!({"status":"cleared"})).into_response()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Restore {
    current_password: String,
    confirmation: String,
}
pub(super) async fn restore(
    State(s): State<ApiState>,
    Path(id): Path<String>,
    Extension(permit): Extension<Permit>,
    Json(q): Json<Restore>,
) -> Response {
    let Some(restart) = s.restart.clone() else {
        return error(
            StatusCode::CONFLICT,
            "Use the offline restore command for this daemon",
        );
    };
    if q.confirmation != "RESTORE" {
        return error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Type RESTORE to confirm replacing application state",
        );
    }
    let Some(auth) = s.auth.clone() else {
        return error(StatusCode::FORBIDDEN, "Administrator sign-in is required");
    };
    let p = s
        .backups
        .current
        .lock()
        .unwrap()
        .as_ref()
        .filter(|p| p.id == id && !p.export)
        .cloned();
    let Some(p) = p else {
        return error(
            StatusCode::CONFLICT,
            "Validate the backup again before restoring",
        );
    };
    let username = p.summary.administrator.clone();
    let config = s.backup_config.clone();
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        auth.confirm(&q.current_password, || crate::backup::stage(&p, &config))
    })
    .await;
    match result {
        Ok(Ok(())) => {
            s.backups.current.lock().unwrap().take();
            restart.notify_one();
            Json(json!({"status":"restart_requested","administrator":username})).into_response()
        }
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, &e.to_string()),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Restore preparation failed",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use tower::ServiceExt;
    fn fixture(
        force: bool,
    ) -> (
        tempfile::TempDir,
        crate::config::Config,
        ApiState,
        String,
        Arc<tokio::sync::Notify>,
    ) {
        let temp = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let downloads = temp.path().join("downloads");
        std::fs::create_dir(&downloads).unwrap();
        let config = crate::config::Config {
            data_dir: temp.path().into(),
            download_dir: Some(downloads),
            ..Default::default()
        };
        crate::store::Store::open(&temp.path().join("rustysoul.sqlite3")).unwrap();
        crate::vault::Vault::open(&temp.path().join("credentials.key"), false).unwrap();
        let auth = crate::auth::Auth::open(&temp.path().join("rustysoul.sqlite3")).unwrap();
        let mut login = auth.login("admin", "change-me-now").unwrap();
        if !force {
            login = auth
                .change(&login.token, "change-me-now", "backup-admin", "eightpwd")
                .unwrap();
        }
        let (tx, _rx) = mpsc::sync_channel(32);
        let (_update, view) = watch::channel(Snapshot::default());
        let restart = Arc::new(tokio::sync::Notify::new());
        let state = ApiState::new(&config, &"s".repeat(32), tx, view)
            .with_auth(auth)
            .with_restart(restart.clone());
        (temp, config, state, login.token, restart)
    }
    async fn req(
        app: &Router,
        path: &str,
        body: Option<Vec<u8>>,
        token: &str,
        content: &str,
    ) -> Response {
        let method = if body.is_some() { "POST" } else { "GET" };
        let mut request = Request::builder()
            .method(method)
            .uri(format!("/api/v1/{path}"))
            .header("Host", "localhost");
        if !token.is_empty() {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        if body.is_some() {
            request = request.header("Content-Type", content);
        }
        app.clone()
            .oneshot(request.body(Body::from(body.unwrap_or_default())).unwrap())
            .await
            .unwrap()
    }
    async fn json(response: Response) -> serde_json::Value {
        serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap()
    }
    fn form(bytes: &[u8], pass: &str) -> Vec<u8> {
        let mut out=format!("--rs-backup-test-boundary-long-unique\r\nContent-Disposition: form-data; name=\"passphrase\"\r\n\r\n{pass}\r\n--rs-backup-test-boundary-long-unique\r\nContent-Disposition: form-data; name=\"file\"; filename=\"backup.rsb\"\r\nContent-Type: application/octet-stream\r\n\r\n").into_bytes();
        out.extend(bytes);
        out.extend(b"\r\n--rs-backup-test-boundary-long-unique--\r\n");
        out
    }

    #[test]
    fn multipart_headers_are_bounded_even_after_binary_data_and_across_chunks() {
        let body=b"--rs-test\r\nContent-Disposition: form-data; name=\"passphrase\"\r\n\r\npassword123\r\n--rs-test\r\nContent-Disposition: form-data; name=\"file\"; filename=\"backup.rsb\"\r\n\r\nbinary\0data\r\n--rs-test--\r\n";
        for size in [1, 2, 7, 64] {
            let mut guard = MultipartHeaders::new("rs-test").unwrap();
            for chunk in body.chunks(size) {
                guard.check(chunk).unwrap();
            }
        }
        let mut guard = MultipartHeaders::new("rs-test").unwrap();
        assert!(guard.check(&vec![b'a'; 9000]).is_err());
        let mut guard = MultipartHeaders::new("rs-test").unwrap();
        guard
            .check(b"--rs-test\r\nName: file\r\n\r\nbinary\r\n--rs-test\r\nName: ")
            .unwrap();
        assert!(guard.check(&vec![b'x'; 8193]).is_err());
    }
    #[tokio::test]
    async fn a_silent_upload_can_be_cancelled_without_holding_the_backup_slot() {
        let (_temp, _config, state, token, _) = fixture(false);
        let app = router(state.clone());
        let app_task = app.clone();
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/backups/inspect")
            .header("Host", "localhost")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "multipart/form-data; boundary=rs-test")
            .body(Body::from_stream(futures_util::stream::pending::<
                Result<axum::body::Bytes, std::io::Error>,
            >()))
            .unwrap();
        let task = tokio::spawn(async move { app_task.oneshot(request).await.unwrap() });
        for _ in 0..100 {
            if state.backups.gate.available_permits() == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(
            req(
                &app,
                "backups/cancel",
                Some(b"{}".to_vec()),
                &token,
                "application/json"
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(state.backups.gate.available_permits(), 1);
    }
    #[tokio::test]
    async fn backup_routes_require_replaced_account_and_serialize_expensive_work() {
        let (_temp, _config, state, token, _) = fixture(true);
        let app = router(state);
        assert_eq!(
            req(&app, "backups", None, "", "").await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            req(
                &app,
                "backups",
                Some(b"{}".to_vec()),
                &token,
                "application/json"
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        let (_temp, _config, state, token, _) = fixture(false);
        let held = state.backups.gate.clone().try_acquire_owned().unwrap();
        let app = router(state);
        let status = req(&app, "backups", None, &token, "").await;
        assert_eq!(status.status(), StatusCode::OK);
        assert_eq!(json(status).await["busy"], true);
        assert_eq!(
            req(&app, "maintenance", None, &token, "").await.status(),
            StatusCode::OK
        );
        assert_eq!(
            req(
                &app,
                "maintenance",
                Some(br#"{"action":"compact"}"#.to_vec()),
                &token,
                "application/json"
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            req(
                &app,
                "backups/inspect",
                Some(vec![0; 100]),
                &token,
                "multipart/form-data; boundary=x"
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        drop(held);
        assert_eq!(
            req(
                &app,
                "backups",
                Some(br#"{"current_password":"wrong","passphrase":"backup-password"}"#.to_vec()),
                &token,
                "application/json"
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
    }
    #[tokio::test]
    async fn authenticated_export_preview_confirmation_and_restart_preserve_media() {
        let (_temp, config, state, token, restart) = fixture(false);
        let media = config.download_dir.as_ref().unwrap().join("media.flac");
        std::fs::write(&media, b"intact").unwrap();
        let app = router(state);
        let response = req(
            &app,
            "backups",
            Some(br#"{"current_password":"eightpwd","passphrase":"backup-password"}"#.to_vec()),
            &token,
            "application/json",
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let ready = json(response).await;
        let id = ready["id"].as_str().unwrap();
        let response = req(&app, &format!("backups/{id}/file"), None, &token, "").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response.headers()[header::CONTENT_DISPOSITION]
                .to_str()
                .unwrap()
                .contains("attachment")
        );
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap();
        let response = req(
            &app,
            "backups/inspect",
            Some(form(&bytes, "wrong-password")),
            &token,
            "multipart/form-data; boundary=rs-backup-test-boundary-long-unique",
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let response = req(
            &app,
            "backups/inspect",
            Some(form(&bytes, "backup-password")),
            &token,
            "multipart/form-data; boundary=rs-backup-test-boundary-long-unique",
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let ready = json(response).await;
        assert_eq!(ready["kind"], "restore");
        let id = ready["id"].as_str().unwrap();
        assert_eq!(
            req(
                &app,
                &format!("backups/{id}/restore"),
                Some(br#"{"current_password":"eightpwd","confirmation":"NO"}"#.to_vec()),
                &token,
                "application/json"
            )
            .await
            .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            req(
                &app,
                &format!("backups/{id}/restore"),
                Some(br#"{"current_password":"wrong","confirmation":"RESTORE"}"#.to_vec()),
                &token,
                "application/json"
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert!(!config.data_dir.join("restore-pending").exists());
        assert_eq!(
            req(
                &app,
                &format!("backups/{id}/restore"),
                Some(br#"{"current_password":"eightpwd","confirmation":"RESTORE"}"#.to_vec()),
                &token,
                "application/json"
            )
            .await
            .status(),
            StatusCode::OK
        );
        tokio::time::timeout(Duration::from_secs(1), restart.notified())
            .await
            .unwrap();
        drop(app);
        crate::backup::apply_pending(&config).unwrap();
        assert_eq!(std::fs::read(media).unwrap(), b"intact");
        let auth = crate::auth::Auth::open(&config.data_dir.join("rustysoul.sqlite3")).unwrap();
        assert!(auth.session(&token).is_none());
        assert!(auth.login("backup-admin", "eightpwd").is_ok());
    }
}
