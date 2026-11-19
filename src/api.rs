#[path = "api/access.rs"]
mod access;
#[path = "api/backups.rs"]
mod backups;
#[path = "api/maintenance.rs"]
mod maintenance;
use crate::{
    config::Config,
    model::{Command, Snapshot},
};
use axum::{
    Extension, Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::{get, post},
};
use futures_util::StreamExt;
use include_dir::{Dir, include_dir};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    convert::Infallible,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tokio::sync::{Semaphore, watch};
use tokio_stream::wrappers::WatchStream;

static WEB: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/web/dist");

#[derive(Clone)]
pub struct ApiState {
    backup_config: Arc<Config>,
    backups: backups::Manager,
    restart: Option<Arc<tokio::sync::Notify>>,
    data_dir: std::path::PathBuf,
    auth: Option<Arc<crate::auth::Auth>>,
    commands: mpsc::SyncSender<Command>,
    snapshot: watch::Receiver<Snapshot>,
    token_hash: [u8; 32],
    allow_remote: bool,
    streams: Arc<Semaphore>,
    media_streams: Arc<Semaphore>,
    maintenance_reads: Arc<Semaphore>,
    key_flush: Arc<Semaphore>,
    image_jobs: Arc<Semaphore>,
    read_rate: Arc<Mutex<(f64, Instant)>>,
    rate: Arc<Mutex<(f64, Instant)>>,
}
impl ApiState {
    pub fn new(
        config: &Config,
        token: &str,
        commands: mpsc::SyncSender<Command>,
        snapshot: watch::Receiver<Snapshot>,
    ) -> Self {
        Self {
            backup_config: Arc::new(config.clone()),
            backups: backups::Manager::default(),
            restart: None,
            data_dir: config.data_dir.clone(),
            auth: None,
            commands,
            snapshot,
            // Compatibility bearer token, generated at startup. User passwords
            // are verified by Auth with Argon2id, never by this token digest.
            token_hash: Sha256::digest(token).into(),
            allow_remote: config.allow_remote,
            streams: Arc::new(Semaphore::new(16)),
            media_streams: Arc::new(Semaphore::new(4)),
            maintenance_reads: Arc::new(Semaphore::new(1)),
            key_flush: Arc::new(Semaphore::new(1)),
            image_jobs: Arc::new(Semaphore::new(1)),
            read_rate: Arc::new(Mutex::new((16., Instant::now()))),
            rate: Arc::new(Mutex::new((10., Instant::now()))),
        }
    }
    pub fn with_restart(mut self, notify: Arc<tokio::sync::Notify>) -> Self {
        self.restart = Some(notify);
        self
    }
    pub fn with_auth(mut self, auth: crate::auth::Auth) -> Self {
        self.auth = Some(Arc::new(auth));
        self
    }
}

pub fn router(state: ApiState) -> Router {
    let api = Router::new()
        .route("/status", get(status))
        .route("/metrics", get(metrics))
        .route("/api-access", get(access::status).post(access::change))
        .route(
            "/maintenance",
            get(maintenance::status)
                .post(maintenance::start)
                .layer(middleware::from_fn_with_state(state.clone(), backups::gate)),
        )
        .route(
            "/backups",
            get(backups::status)
                .post(backups::create)
                .layer(middleware::from_fn_with_state(state.clone(), backups::gate)),
        )
        .route(
            "/backups/inspect",
            post(backups::inspect)
                .layer(DefaultBodyLimit::max(crate::backup::MAX_UPLOAD + 8192))
                .layer(middleware::from_fn_with_state(state.clone(), backups::gate)),
        )
        .route("/backups/cancel", post(backups::cancel))
        .route(
            "/backups/clear",
            post(backups::clear)
                .layer(middleware::from_fn_with_state(state.clone(), backups::gate)),
        )
        .route(
            "/backups/{id}/file",
            get(backups::download)
                .layer(middleware::from_fn_with_state(state.clone(), backups::gate)),
        )
        .route(
            "/backups/{id}/restore",
            post(backups::restore)
                .layer(middleware::from_fn_with_state(state.clone(), backups::gate)),
        )
        .route("/logs", get(logs))
        .route("/messages", get(message_history))
        .route("/messages/conversations", get(message_conversations))
        .route("/rooms/read", post(read_room))
        .route("/messages/read", post(read_private))
        .route("/rooms/entries", get(room_entries))
        .route("/downloads/file", get(download_file))
        .route("/users/picture", get(user_picture))
        .route(
            "/profile",
            get(own_profile)
                .post(save_profile)
                .layer(DefaultBodyLimit::max(14 * 1024 * 1024))
                .layer(middleware::from_fn_with_state(state.clone(), profile_gate)),
        )
        .route("/transfers/{kind}", get(transfer_history))
        .route("/traffic", get(traffic_history))
        .route("/events", get(events))
        .route("/commands", post(command))
        .route("/folders", get(folders))
        .route("/browse/files", get(browse_files))
        .route("/browse/tree", get(browse_tree))
        .route("/searches/{id}/results", get(search_results))
        .route(
            "/downloads/batch",
            post(batch_download).layer(DefaultBodyLimit::max(2 * 1024 * 1024)),
        )
        .route("/auth/me", get(me))
        .route("/auth/account", post(change_account))
        .route("/auth/logout", post(logout))
        .layer(DefaultBodyLimit::max(8192))
        .layer(middleware::from_fn_with_state(state.clone(), authorize));
    Router::new()
        .route("/health", get(|| async { Json(json!({"status":"ok"})) }))
        .route("/api/v1/auth/login", post(login))
        .route("/", get(index))
        .route("/site.webmanifest", get(web_manifest))
        .route("/assets/{*path}", get(asset))
        .nest("/api/v1", api)
        .layer(middleware::from_fn_with_state(state.clone(), security))
        .layer(DefaultBodyLimit::max(8192))
        .with_state(state)
}
async fn room_entries(
    State(state): State<ApiState>,
    Query(query): Query<crate::rooms::PageQuery>,
) -> Response {
    let account = state.snapshot.borrow().username.clone();
    match tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let db = rusqlite::Connection::open_with_flags(
            state.data_dir.join("rustysoul.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        db.busy_timeout(Duration::from_secs(5))?;
        crate::rooms::page(&db, &account, &query)
    })
    .await
    {
        Ok(Ok(page)) => Json(page).into_response(),
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, &e.to_string()),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not read room entries",
        ),
    }
}
async fn logs(
    State(state): State<ApiState>,
    Query(query): Query<crate::activity::Query>,
) -> Response {
    match tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let db = rusqlite::Connection::open_with_flags(
            state.data_dir.join("rustysoul.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        db.busy_timeout(Duration::from_secs(5))?;
        crate::activity::page(&db, &query)
    })
    .await
    {
        Ok(Ok(page)) => Json(page).into_response(),
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, &e.to_string()),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not read operational history",
        ),
    }
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadRoom {
    room: String,
    through_id: u64,
}
async fn read_room(State(state): State<ApiState>, Json(q): Json<ReadRoom>) -> Response {
    let command = Command::ReadRoom {
        room: q.room,
        through_id: q.through_id,
    };
    queue_read(state, command)
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadPrivate {
    username: String,
    through_id: u64,
}
async fn read_private(State(state): State<ApiState>, Json(q): Json<ReadPrivate>) -> Response {
    queue_read(
        state,
        Command::ReadPrivate {
            username: q.username,
            through_id: q.through_id,
        },
    )
}
fn queue_read(state: ApiState, command: Command) -> Response {
    if let Err(e) = command.validate() {
        return error(StatusCode::BAD_REQUEST, &e);
    }
    {
        let mut rate = state.read_rate.lock().unwrap();
        rate.0 = (rate.0 + rate.1.elapsed().as_secs_f64() * 4.).min(16.);
        rate.1 = Instant::now();
        if rate.0 < 1. {
            return error(
                StatusCode::TOO_MANY_REQUESTS,
                "Read markers are updating too quickly",
            );
        }
        rate.0 -= 1.;
    }
    match state.commands.try_send(command) {
        Ok(()) => (StatusCode::ACCEPTED, Json(json!({"status":"queued"}))).into_response(),
        Err(_) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "The daemon is busy; the unread indicator will retry",
        ),
    }
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PictureQuery {
    username: String,
    v: Option<String>,
}
async fn user_picture(State(state): State<ApiState>, Query(q): Query<PictureQuery>) -> Response {
    if crate::validation::username(&q.username).is_err()
        || q.v.as_ref().is_some_and(|v| v.len() > 128)
    {
        return error(StatusCode::BAD_REQUEST, "Invalid profile picture query");
    }
    let own = q.username == state.snapshot.borrow().username;
    if !own
        && !state
            .snapshot
            .borrow()
            .users
            .get(&q.username)
            .is_some_and(|u| u.has_picture)
    {
        return error(
            StatusCode::NOT_FOUND,
            "No cached picture is available for this user",
        );
    }
    match tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
        let db = rusqlite::Connection::open_with_flags(
            state.data_dir.join("rustysoul.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        db.busy_timeout(Duration::from_secs(5))?;
        let data = if own {
            crate::profile::load(&db)?.picture_bytes()
        } else {
            db.query_row(
                "SELECT data FROM user_pictures WHERE username=?1",
                [q.username],
                |r| r.get::<_, Vec<u8>>(0),
            )?
        };
        anyhow::ensure!(
            !data.is_empty() && data.len() <= crate::profile::MAX_PICTURE,
            "No picture available"
        );
        Ok(data)
    })
    .await
    {
        Ok(Ok(data)) => (
            [
                (
                    header::CONTENT_TYPE,
                    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
                        "image/png"
                    } else {
                        "image/jpeg"
                    },
                ),
                (header::CACHE_CONTROL, "private, no-store"),
            ],
            data,
        )
            .into_response(),
        _ => error(
            StatusCode::NOT_FOUND,
            "No valid profile picture is available",
        ),
    }
}
#[derive(Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FileQuery {
    username: String,
    path: String,
    download: bool,
}
async fn download_file(
    State(state): State<ApiState>,
    Query(q): Query<FileQuery>,
    headers: axum::http::HeaderMap,
) -> Response {
    if crate::validation::username(&q.username).is_err()
        || crate::validation::remote_path(&q.path).is_err()
    {
        return error(StatusCode::BAD_REQUEST, "Invalid download identity");
    }
    let Ok(permit) = state.media_streams.clone().try_acquire_owned() else {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many file streams; close another player and retry",
        );
    };
    let filename = crate::downloads::basename(&q.path).to_string();
    let download = q.download;
    let opened=tokio::task::spawn_blocking(move||->anyhow::Result<std::fs::File>{
        let db=rusqlite::Connection::open_with_flags(state.data_dir.join("rustysoul.sqlite3"),rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;db.busy_timeout(Duration::from_secs(5))?;
        let row=db.query_row("SELECT directory,username,path,size,status,local_path FROM downloads WHERE username=?1 AND path=?2",rusqlite::params![q.username,q.path],|r|Ok(crate::store::SavedDownload{directory:r.get(0)?,username:r.get(1)?,path:r.get(2)?,size:r.get::<_,i64>(3)? as u64,status:r.get(4)?,local_path:r.get(5)?}))?;
        crate::downloads::open_completed(&row)
    }).await;
    let file = match opened {
        Ok(Ok(file)) => file,
        _ => {
            return error(
                StatusCode::NOT_FOUND,
                "The completed file is unavailable. It may have been moved or removed.",
            );
        }
    };
    let size = match file.metadata() {
        Ok(m) => m.len(),
        Err(_) => return error(StatusCode::NOT_FOUND, "File is unavailable"),
    };
    let requested = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let range = match if size == 0 && requested.is_none() {
        Ok((0, 0))
    } else {
        crate::media::range(requested, size)
    } {
        Ok(r) => r,
        Err(_) => {
            let mut r = error(StatusCode::RANGE_NOT_SATISFIABLE, "Invalid byte range");
            r.headers_mut().insert(
                header::CONTENT_RANGE,
                HeaderValue::from_str(&format!("bytes */{size}")).unwrap(),
            );
            return r;
        }
    };
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let mut file = tokio::fs::File::from_std(file);
    if file.seek(std::io::SeekFrom::Start(range.0)).await.is_err() {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not seek in this file",
        );
    }
    let length = if size == 0 { 0 } else { range.1 - range.0 + 1 };
    let stream = tokio_util::io::ReaderStream::with_capacity(file.take(length), 64 * 1024).map(
        move |chunk| {
            let _ = &permit;
            chunk
        },
    );
    let mut response = Body::from_stream(stream).into_response();
    *response.status_mut() = if requested.is_some() {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    let h = response.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(crate::media::mime(&filename)),
    );
    h.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&length.to_string()).unwrap(),
    );
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    if requested.is_some() {
        h.insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes {}-{}/{size}", range.0, range.1)).unwrap(),
        );
    }
    let encoded = filename
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"._-".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect::<String>();
    let disposition = format!(
        "{}; filename*=UTF-8''{}",
        if download { "attachment" } else { "inline" },
        encoded
    );
    h.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&disposition).unwrap(),
    );
    response
}
async fn own_profile(State(state): State<ApiState>) -> Response {
    match tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let db = rusqlite::Connection::open_with_flags(
            state.data_dir.join("rustysoul.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        db.busy_timeout(Duration::from_secs(5))?;
        crate::profile::load(&db)
    })
    .await
    {
        Ok(Ok(p)) => Json(p).into_response(),
        _ => error(StatusCode::INTERNAL_SERVER_ERROR, "Could not load profile"),
    }
}
async fn save_profile(
    State(state): State<ApiState>,
    permit: Option<Extension<ProfilePermit>>,
    Json(settings): Json<crate::profile::Profile>,
) -> Response {
    prepare_profile(state, settings, permit.map(|p| p.0)).await
}

#[derive(Clone)]
struct ProfilePermit(Arc<tokio::sync::OwnedSemaphorePermit>);
// Admit at most one large upload before buffering its body. Keep the permit in
// the worker too: a disconnected HTTP request must not start a second optimizer.
async fn profile_gate(State(state): State<ApiState>, mut request: Request, next: Next) -> Response {
    if request.method() != axum::http::Method::POST {
        return next.run(request).await;
    }
    let Ok(permit) = state.image_jobs.clone().try_acquire_owned() else {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "A profile is already processing. Try again shortly.",
        );
    };
    let permit = ProfilePermit(Arc::new(permit));
    request.extensions_mut().insert(permit.clone());
    let response = next.run(request).await;
    drop(permit);
    response
}
async fn prepare_profile(
    state: ApiState,
    settings: crate::profile::Profile,
    permit: Option<ProfilePermit>,
) -> Response {
    if let Err(e) = settings.validate() {
        return error(StatusCode::UNPROCESSABLE_ENTITY, &e.to_string());
    }
    let permit = match permit {
        Some(p) => p,
        None => match state.image_jobs.clone().try_acquire_owned() {
            Ok(p) => ProfilePermit(Arc::new(p)),
            Err(_) => {
                return error(
                    StatusCode::TOO_MANY_REQUESTS,
                    "A profile is already processing. Try again shortly.",
                );
            }
        },
    };
    let path = state.data_dir.join("rustysoul.sqlite3");
    let expected_revision = state.snapshot.borrow().profile.revision;
    match tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let _permit = permit.0;
        let db = rusqlite::Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        db.busy_timeout(Duration::from_secs(5))?;
        settings.normalize_against(&crate::profile::load(&db)?)
    })
    .await
    {
        Ok(Ok(settings)) => admit_command(
            &state,
            Command::UpdateProfilePrepared {
                settings,
                expected_revision,
            },
        ),
        Ok(Err(e)) => error(StatusCode::UNPROCESSABLE_ENTITY, &e.to_string()),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not process this profile picture",
        ),
    }
}
async fn transfer_history(
    State(state): State<ApiState>,
    Path(kind): Path<String>,
    Query(query): Query<crate::transfers::Query>,
) -> Response {
    match tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let db = rusqlite::Connection::open_with_flags(
            state.data_dir.join("rustysoul.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        db.busy_timeout(Duration::from_secs(5))?;
        let snapshot = db.unchecked_transaction()?;
        crate::transfers::page(&snapshot, &kind, &query)
    })
    .await
    {
        Ok(Ok(page)) => Json(page).into_response(),
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, &e.to_string()),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not load transfer history",
        ),
    }
}
async fn traffic_history(
    State(state): State<ApiState>,
    Query(query): Query<crate::traffic::Query>,
) -> Response {
    match tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let db = rusqlite::Connection::open_with_flags(
            state.data_dir.join("rustysoul.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        db.busy_timeout(Duration::from_secs(5))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        crate::traffic::history(&db, &query, now)
    })
    .await
    {
        Ok(Ok(page)) => Json(page).into_response(),
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, &e.to_string()),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not load bandwidth history",
        ),
    }
}
async fn message_history(
    State(state): State<ApiState>,
    Query(query): Query<crate::messages::Query>,
) -> Response {
    match tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let db = rusqlite::Connection::open_with_flags(
            state.data_dir.join("rustysoul.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        db.busy_timeout(Duration::from_secs(5))?;
        crate::messages::page(&db, &query)
    })
    .await
    {
        Ok(Ok(page)) => Json(page).into_response(),
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, &e.to_string()),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not load message history",
        ),
    }
}
async fn message_conversations(State(state): State<ApiState>) -> Response {
    match tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let db = rusqlite::Connection::open_with_flags(
            state.data_dir.join("rustysoul.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        db.busy_timeout(Duration::from_secs(5))?;
        crate::messages::conversations(&db)
    })
    .await
    {
        Ok(Ok(conversations)) => Json(conversations).into_response(),
        _ => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not load saved conversations",
        ),
    }
}
async fn browse_tree(
    State(state): State<ApiState>,
    Query(query): Query<crate::browse::TreeQuery>,
) -> Response {
    if !state.snapshot.borrow().browse.contains_key(&query.username) {
        return error(StatusCode::NOT_FOUND, "Browse this user's shares first");
    }
    match tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let db = rusqlite::Connection::open_with_flags(
            state.data_dir.join("rustysoul.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        db.busy_timeout(Duration::from_secs(5))?;
        crate::browse::tree(&db, &query)
    })
    .await
    {
        Ok(Ok(page)) => Json(page).into_response(),
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, &e.to_string()),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not load folder tree",
        ),
    }
}
async fn search_results(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Query(query): Query<crate::search::SearchQuery>,
) -> Response {
    if !state.snapshot.borrow().searches.iter().any(|s| s.id == id) {
        return error(StatusCode::NOT_FOUND, "Search is no longer available");
    }
    let commands = state.commands.clone();
    match tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let db = rusqlite::Connection::open_with_flags(
            state.data_dir.join("rustysoul.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        db.busy_timeout(Duration::from_secs(5))?;
        crate::search::page(&db, &id, &query)
    })
    .await
    {
        Ok(Ok(page)) => {
            let _ = commands.try_send(Command::WatchUsers {
                usernames: page.users.keys().cloned().collect(),
            });
            Json(page).into_response()
        }
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, &e.to_string()),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not load search results",
        ),
    }
}
async fn browse_files(
    State(state): State<ApiState>,
    Query(query): Query<crate::browse::BrowseQuery>,
) -> Response {
    // Only listings opened through the bounded engine workflow are readable.
    if !state.snapshot.borrow().browse.contains_key(&query.username) {
        return error(StatusCode::NOT_FOUND, "Browse this user's shares first");
    }
    match tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let db = rusqlite::Connection::open_with_flags(
            state.data_dir.join("rustysoul.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        db.busy_timeout(Duration::from_secs(5))?;
        crate::browse::page(&db, &query)
    })
    .await
    {
        Ok(Ok(page)) => Json(page).into_response(),
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, &e.to_string()),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Cannot read share listing",
        ),
    }
}
#[derive(serde::Deserialize)]
struct FolderQuery {
    path: Option<String>,
}
async fn folders(State(state): State<ApiState>, Query(query): Query<FolderQuery>) -> Response {
    let current = state.snapshot.borrow().clone();
    let roots = if current.location_roots.is_empty() {
        std::iter::once(current.locations.downloads)
            .chain(current.locations.shares)
            .filter(|p| !p.is_empty())
            .collect()
    } else {
        current.location_roots
    };
    match tokio::task::spawn_blocking(move || folder_listing(roots, query.path, state.data_dir))
        .await
    {
        Ok(Ok(list)) => Json(list).into_response(),
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, &e.to_string()),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "Cannot browse folders"),
    }
}
fn folder_listing(
    roots: Vec<String>,
    path: Option<String>,
    private: std::path::PathBuf,
) -> anyhow::Result<serde_json::Value> {
    use std::path::{Path, PathBuf};
    let roots: Vec<PathBuf> = roots
        .into_iter()
        .map(std::fs::canonicalize)
        .collect::<Result<_, _>>()?;
    let Some(path) = path.filter(|p| !p.is_empty()) else {
        return Ok(
            json!({"path":null,"parent":null,"roots":roots,"folders":roots.iter().map(|p|json!({"name":p.display().to_string(),"path":p})).collect::<Vec<_>>(),"truncated":false}),
        );
    };
    anyhow::ensure!(path.len() <= 4096, "Folder path is too long");
    let path = std::fs::canonicalize(Path::new(&path))?;
    anyhow::ensure!(
        path.is_dir() && roots.iter().any(|r| path.starts_with(r)),
        "Choose a folder inside an installed storage mount"
    );
    if let Ok(private) = std::fs::canonicalize(private) {
        anyhow::ensure!(
            !path.starts_with(&private) && !private.starts_with(&path),
            "Private application data cannot be browsed"
        );
    }
    let parent = path
        .parent()
        .filter(|p| roots.iter().any(|r| p.starts_with(r)))
        .map(|p| p.to_path_buf());
    let mut directories = vec![];
    let mut truncated = false;
    for entry in std::fs::read_dir(&path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            if directories.len() == 1000 {
                truncated = true;
                break;
            }
            directories.push((
                entry.file_name().to_string_lossy().into_owned(),
                entry.path(),
            ));
        }
    }
    directories.sort_by_key(|a| a.0.to_lowercase());
    Ok(
        json!({"path":path,"parent":parent,"roots":roots,"folders":directories.into_iter().map(|(name,path)|json!({"name":name,"path":path})).collect::<Vec<_>>(),"truncated":truncated}),
    )
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchInput {
    files: Vec<crate::model::DownloadFile>,
}
#[derive(Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct BatchQuery {
    confirm: bool,
}
async fn batch_download(
    state: State<ApiState>,
    Query(query): Query<BatchQuery>,
    Json(input): Json<BatchInput>,
) -> Response {
    if !query.confirm {
        return command(state, Json(Command::DownloadBatch { files: input.files })).await;
    }
    let (reply, receive) = mpsc::sync_channel(1);
    let queued = command(
        state,
        Json(Command::DownloadBatchConfirmed {
            files: input.files,
            reply,
        }),
    )
    .await;
    if queued.status() != StatusCode::ACCEPTED {
        return queued;
    }
    match tokio::task::spawn_blocking(move || receive.recv_timeout(Duration::from_secs(15))).await {
        Ok(Ok(Ok(count))) => (
            StatusCode::ACCEPTED,
            Json(json!({"status":"queued","files":count})),
        )
            .into_response(),
        Ok(Ok(Err(message))) => error(StatusCode::CONFLICT, &message),
        _ => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Download confirmation timed out; selection was kept. Check the queue before retrying.",
        ),
    }
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

async fn security(State(state): State<ApiState>, request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|s| s.to_str().ok())
        .unwrap_or("");
    if !state.allow_remote {
        let name = host
            .parse::<axum::http::uri::Authority>()
            .ok()
            .map(|a| a.host().to_owned())
            .unwrap_or_default();
        if !matches!(name.as_str(), "localhost" | "127.0.0.1" | "[::1]" | "::1") {
            return error(StatusCode::FORBIDDEN, "Untrusted Host header");
        }
    }
    if let Some(origin) = request.headers().get(header::ORIGIN) {
        let same = origin
            .to_str()
            .ok()
            .and_then(|v| v.parse::<axum::http::Uri>().ok())
            .is_some_and(|u| {
                matches!(u.scheme_str(), Some("http" | "https"))
                    && u.authority().is_some_and(|a| a.as_str() == host)
            });
        if !same {
            return error(
                StatusCode::FORBIDDEN,
                "Cross-origin requests are not allowed",
            );
        }
    }
    let path = request.uri().path();
    let is_post = request.method() == axum::http::Method::POST;
    let backup_work = is_post
        && (matches!(
            path,
            "/api/v1/backups" | "/api/v1/backups/inspect" | "/api/v1/maintenance"
        ) || (path.starts_with("/api/v1/backups/") && path.ends_with("/restore")));
    let timeout = if backup_work {
        backups::OPERATION_TIMEOUT
    } else if is_post && path == "/api/v1/profile" {
        Duration::from_secs(180)
    } else {
        Duration::from_secs(10)
    };
    let mut response = match tokio::time::timeout(timeout, next.run(request)).await {
        Ok(response) => response,
        Err(_) => error(StatusCode::REQUEST_TIMEOUT, "Request timed out"),
    };
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; media-src 'self'; manifest-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        "permissions-policy",
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    response
}

async fn authorize(State(state): State<ApiState>, request: Request, next: Next) -> Response {
    let token = bearer(request.headers());
    if let Some(auth) = &state.auth {
        let Some(account) = auth.session(token) else {
            if request.headers().contains_key(header::AUTHORIZATION)
                && let Some((scope, flush)) = auth.api_keys.authenticate(token, true)
            {
                if !scope.allows(
                    request.method().as_str(),
                    request
                        .extensions()
                        .get::<axum::extract::MatchedPath>()
                        .map_or(request.uri().path(), |p| p.as_str()),
                ) {
                    return error(
                        StatusCode::FORBIDDEN,
                        "This API key does not allow that operation. Use the browser administrator account for account, API-key, backup or maintenance controls.",
                    );
                }
                if flush && let Ok(permit) = state.key_flush.clone().try_acquire_owned() {
                    let auth = auth.clone();
                    tokio::task::spawn_blocking(move || {
                        let _permit = permit;
                        if let Err(error) = auth.api_keys.flush_usage() {
                            tracing::warn!(%error,"Could not save API key usage metadata");
                        }
                    });
                }
                return next.run(request).await;
            }
            return error(StatusCode::UNAUTHORIZED, "Sign in to continue");
        };
        if request.headers().get(header::AUTHORIZATION).is_none()
            && !matches!(
                *request.method(),
                axum::http::Method::GET | axum::http::Method::HEAD
            )
            && request
                .headers()
                .get("x-rustysoul-request")
                .and_then(|v| v.to_str().ok())
                != Some("1")
        {
            return error(
                StatusCode::FORBIDDEN,
                "Browser requests require the same-origin request header",
            );
        }
        if account.must_change
            && !matches!(
                request.uri().path(),
                "/api/v1/auth/me"
                    | "/api/v1/auth/account"
                    | "/api/v1/auth/logout"
                    | "/auth/me"
                    | "/auth/account"
                    | "/auth/logout"
            )
        {
            return error(
                StatusCode::FORBIDDEN,
                "Change both the deployment username and password before using RustySoul",
            );
        }
        return next.run(request).await;
    }
    let candidate: [u8; 32] = Sha256::digest(token).into();
    if !bool::from(candidate.ct_eq(&state.token_hash)) {
        let mut response = error(StatusCode::UNAUTHORIZED, "A valid API token is required");
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        return response;
    }
    next.run(request).await
}

fn bearer(headers: &axum::http::HeaderMap) -> &str {
    if headers.contains_key(header::AUTHORIZATION) {
        return headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("");
    }
    headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.split(';')
                .find_map(|c| c.trim().strip_prefix("rustysoul_session="))
        })
        .unwrap_or("")
}
fn session_response(result: crate::auth::Login, headers: &axum::http::HeaderMap) -> Response {
    let secure = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        == Some("https");
    let cookie = format!(
        "rustysoul_session={}; Path=/api/v1; HttpOnly; SameSite=Strict; Max-Age={}{}",
        result.token,
        crate::auth::SESSION_SECONDS,
        if secure { "; Secure" } else { "" }
    );
    let mut response = Json(result).into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
    response
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginInput {
    username: String,
    password: String,
}
async fn login(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Json(input): Json<LoginInput>,
) -> Response {
    let Some(auth) = state.auth else {
        return error(StatusCode::NOT_FOUND, "Login is unavailable");
    };
    if !auth.admit_login() {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many sign-in attempts; try again shortly",
        );
    }
    match tokio::task::spawn_blocking(move || auth.login(&input.username, &input.password)).await {
        Ok(Ok(result)) => {
            crate::activity::record("info", "activity", "Administrator signed in");
            session_response(result, &headers)
        }
        _ => {
            crate::activity::record("warn", "activity", "Administrator sign-in rejected");
            error(StatusCode::UNAUTHORIZED, "Invalid username or password")
        }
    }
}
async fn me(State(state): State<ApiState>, headers: axum::http::HeaderMap) -> Response {
    match state.auth.and_then(|a| a.session(bearer(&headers))) {
        Some(a) => Json(a).into_response(),
        None => error(StatusCode::UNAUTHORIZED, "Sign in again"),
    }
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountInput {
    current_password: String,
    username: String,
    password: String,
}
async fn change_account(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Json(input): Json<AccountInput>,
) -> Response {
    let Some(auth) = state.auth else {
        return error(StatusCode::NOT_FOUND, "Account management unavailable");
    };
    if !auth.admit_login() {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many account attempts; try again shortly",
        );
    }
    if let Err(e) = crate::auth::validate(&input.username, &input.password) {
        return error(StatusCode::UNPROCESSABLE_ENTITY, &e.to_string());
    }
    let token = bearer(&headers).to_string();
    match tokio::task::spawn_blocking(move || {
        auth.change(
            &token,
            &input.current_password,
            &input.username,
            &input.password,
        )
    })
    .await
    {
        Ok(Ok(result)) => session_response(result, &headers),
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, &e.to_string()),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "Account update failed"),
    }
}
async fn logout(State(state): State<ApiState>, headers: axum::http::HeaderMap) -> Response {
    if let Some(auth) = state.auth
        && auth.logout(bearer(&headers)).is_err()
    {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not revoke session; try signing out again",
        );
    }
    let mut response = Json(json!({"status":"signed_out"})).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static(
            "rustysoul_session=; Path=/api/v1; HttpOnly; SameSite=Strict; Max-Age=0",
        ),
    );
    response
}

async fn status(State(state): State<ApiState>) -> Json<Snapshot> {
    Json(state.snapshot.borrow().clone())
}
async fn metrics(State(state): State<ApiState>) -> Response {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        crate::system::metrics(&state.snapshot.borrow(), now),
    )
        .into_response()
}

async fn command(State(state): State<ApiState>, Json(command): Json<Command>) -> Response {
    if let Command::UpdateProfile { settings } = command {
        return prepare_profile(state, settings, None).await;
    }
    admit_command(&state, command)
}
fn admit_command(state: &ApiState, command: Command) -> Response {
    if let Err(e) = command.validate() {
        return error(StatusCode::UNPROCESSABLE_ENTITY, &e);
    }
    {
        let mut rate = state
            .rate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        rate.0 = (rate.0 + rate.1.elapsed().as_secs_f64() * 0.5).min(10.);
        rate.1 = Instant::now();
        if rate.0 < 1. {
            return error(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many commands; try again shortly",
            );
        }
        rate.0 -= 1.;
    }
    match state.commands.try_send(command) {
        Ok(()) => (StatusCode::ACCEPTED, Json(json!({"status": "queued"}))).into_response(),
        Err(mpsc::TrySendError::Full(_)) => error(
            StatusCode::TOO_MANY_REQUESTS,
            "Daemon is busy; try again shortly",
        ),
        Err(mpsc::TrySendError::Disconnected(_)) => {
            error(StatusCode::SERVICE_UNAVAILABLE, "Daemon is stopping")
        }
    }
}

async fn events(State(state): State<ApiState>, headers: axum::http::HeaderMap) -> Response {
    let Ok(permit) = state.streams.clone().try_acquire_owned() else {
        return error(StatusCode::TOO_MANY_REQUESTS, "Too many event streams");
    };
    let stream = WatchStream::new(state.snapshot).map(move |snapshot| {
        let _keep_permit_alive = &permit;
        Ok::<_, Infallible>(Event::default().event("state").json_data(snapshot).unwrap())
    });
    let auth = state.auth.clone();
    let mut revision = auth.as_ref().map(|a| a.revision.subscribe());
    let token = bearer(&headers).to_string();
    let revoked = async move {
        if let Some(ref mut rev) = revision {
            loop {
                tokio::select! { _=rev.changed()=>{}, _=tokio::time::sleep(Duration::from_secs(1))=>{} }
                if auth.as_ref().is_some_and(|a| {
                    a.session(&token).is_none() && a.api_keys.authenticate(&token, false).is_none()
                }) {
                    break;
                }
            }
        } else {
            std::future::pending::<()>().await;
        }
    };
    Sse::new(stream.take_until(revoked))
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}

async fn index() -> Response {
    serve_asset("index.html")
}
async fn web_manifest() -> Response {
    serve_asset("site.webmanifest")
}
async fn asset(Path(path): Path<String>) -> Response {
    serve_asset(&format!("assets/{path}"))
}
fn serve_asset(path: &str) -> Response {
    let Some(file) = WEB.get_file(path) else {
        return error(StatusCode::NOT_FOUND, "Not found");
    };
    let mime = if path.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if path.ends_with(".webmanifest") {
        "application/manifest+json"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".js") {
        "text/javascript; charset=utf-8"
    } else if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else {
        "application/octet-stream"
    };
    ([(header::CONTENT_TYPE, mime)], Body::from(file.contents())).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use tower::ServiceExt;
    #[tokio::test]
    async fn attribute_and_message_search_queries_work_through_authenticated_readonly_http_routes()
    {
        use http_body_util::BodyExt;
        let dir = tempfile::tempdir().unwrap();
        let mut store = crate::store::Store::open(&dir.path().join("rustysoul.sqlite3")).unwrap();
        let search = crate::model::SearchView {
            id: "quality-http".into(),
            query: "music".into(),
            ..Default::default()
        };
        crate::search::create(&store.connection, &search).unwrap();
        crate::search::append(
            &store.connection,
            &search.id,
            [
                crate::model::FileView {
                    username: "alice".into(),
                    path: "Music/good.flac".into(),
                    size: 1024,
                    free_slots: 1,
                    speed: 2048,
                    attributes: [(1, 120), (4, 96000), (5, 24)].into(),
                },
                crate::model::FileView {
                    username: "bob".into(),
                    path: "Music/low.mp3".into(),
                    size: 1024,
                    free_slots: 0,
                    speed: 1024,
                    attributes: [(0, 192), (1, 120), (2, 0), (4, 44100)].into(),
                },
            ],
            100,
        )
        .unwrap();
        store
            .save_messages(&[
                crate::model::MessageView {
                    id: 1,
                    target: "club".into(),
                    username: "Alice".into(),
                    text: "Jazz recommendation".into(),
                    private: false,
                    timestamp: 1,
                },
                crate::model::MessageView {
                    id: 2,
                    target: "club".into(),
                    username: "bob".into(),
                    text: "Jazz from someone else".into(),
                    private: false,
                    timestamp: 2,
                },
            ])
            .unwrap();
        let (_, snapshot) = watch::channel(Snapshot {
            searches: vec![search],
            ..Default::default()
        });
        let (commands, _) = mpsc::sync_channel(32);
        let app = router(ApiState::new(
            &Config {
                data_dir: dir.path().into(),
                ..Default::default()
            },
            &"s".repeat(32),
            commands,
            snapshot,
        ));
        // Library/history writers must coexist with the API's read-only snapshot.
        let _writer = crate::store::write_transaction(&store.connection).unwrap();
        for (uri, property, expected) in [
            (
                "/api/v1/searches/quality-http/results?min_sample_rate=48000&min_bit_depth=24&min_size=100&free_slots=true",
                "files",
                "Music/good.flac",
            ),
            (
                "/api/v1/messages?target=club&private=false&filter=jazz&username=ALICE",
                "messages",
                "Jazz recommendation",
            ),
        ] {
            assert_eq!(
                app.clone()
                    .oneshot(req("GET", uri, None, ""))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::UNAUTHORIZED
            );
            let response = app
                .clone()
                .oneshot(req("GET", uri, Some(&"s".repeat(32)), ""))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body: serde_json::Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert_eq!(body[property].as_array().unwrap().len(), 1);
            assert_eq!(
                body[property][0][if property == "files" { "path" } else { "text" }],
                expected
            );
        }
        for uri in [
            "/api/v1/searches/quality-http/results?min_bitrate=320&max_bitrate=128",
            "/api/v1/searches/quality-http/results?min_bit_depth=-1",
            "/api/v1/messages?target=club&username=bad%0Aname",
        ] {
            assert_eq!(
                app.clone()
                    .oneshot(req("GET", uri, Some(&"s".repeat(32)), ""))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::BAD_REQUEST
            );
        }
    }
    #[tokio::test]
    async fn backup_work_can_outlast_the_ordinary_request_deadline() {
        let (commands, _receiver) = mpsc::sync_channel(32);
        let (_, snapshot) = watch::channel(Snapshot::default());
        let state = ApiState::new(&Config::default(), &"s".repeat(32), commands, snapshot);
        let cases = [
            ("POST", "/api/v1/backups", StatusCode::OK),
            ("POST", "/api/v1/maintenance", StatusCode::OK),
            ("POST", "/api/v1/backups/inspect", StatusCode::OK),
            ("POST", "/api/v1/backups/backup-id/restore", StatusCode::OK),
            ("POST", "/api/v1/profile", StatusCode::OK),
            ("POST", "/api/v1/commands", StatusCode::REQUEST_TIMEOUT),
            ("GET", "/api/v1/backups", StatusCode::REQUEST_TIMEOUT),
            ("POST", "/api/v1/backups/clear", StatusCode::REQUEST_TIMEOUT),
        ];
        let requests = cases.into_iter().map(|(method, path, expected)| {
            let app = Router::new()
                .route(
                    path,
                    axum::routing::any(|| async {
                        tokio::time::sleep(Duration::from_secs(11)).await;
                        Json(json!({"finished":true}))
                    }),
                )
                .layer(middleware::from_fn_with_state(state.clone(), security));
            async move {
                let response = app
                    .oneshot(
                        Request::builder()
                            .method(method)
                            .uri(path)
                            .header("Host", "localhost")
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), expected, "{method} {path}");
                assert!(
                    response
                        .headers()
                        .contains_key(header::CONTENT_SECURITY_POLICY)
                );
            }
        });
        futures_util::future::join_all(requests).await;
    }
    #[test]
    fn folder_browser_stays_inside_mounts_and_excludes_private_data() {
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("library");
        let private = library.join("private");
        std::fs::create_dir_all(private.join("keys")).unwrap();
        std::fs::create_dir(library.join("Album")).unwrap();
        std::fs::write(library.join("track.flac"), b"music").unwrap();
        let roots = vec![library.display().to_string()];
        assert!(
            folder_listing(
                roots.clone(),
                Some(dir.path().display().to_string()),
                private.clone()
            )
            .is_err()
        );
        assert!(
            folder_listing(
                roots.clone(),
                Some(private.join("keys").display().to_string()),
                private.clone()
            )
            .is_err()
        );
        let album = folder_listing(
            roots.clone(),
            Some(library.join("Album").display().to_string()),
            private.clone(),
        )
        .unwrap();
        assert_eq!(album["folders"].as_array().unwrap().len(), 0);
        assert!(folder_listing(roots, Some(library.display().to_string()), private).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn folder_browser_does_not_follow_symlinks_outside_mounts() {
        let dir = tempfile::tempdir().unwrap();
        let mount = dir.path().join("mount");
        std::fs::create_dir(&mount).unwrap();
        std::os::unix::fs::symlink(dir.path(), mount.join("escape")).unwrap();
        let roots = vec![mount.display().to_string()];
        let list = folder_listing(
            roots.clone(),
            Some(mount.display().to_string()),
            dir.path().join("private"),
        )
        .unwrap();
        assert_eq!(list["folders"].as_array().unwrap().len(), 0);
        assert!(
            folder_listing(
                roots,
                Some(mount.join("escape").display().to_string()),
                dir.path().join("private")
            )
            .is_err()
        );
    }
    #[tokio::test]
    async fn confirmed_batches_wait_for_durable_admission_and_report_rejection() {
        for expected in [Ok(1_usize), Err("Download queue is full".to_string())] {
            let (app, receive) = app();
            let result = expected.clone();
            let worker = std::thread::spawn(move || match receive.recv().unwrap() {
                Command::DownloadBatchConfirmed { files, reply } => {
                    assert_eq!(files[0].path, "Music\\song.flac");
                    reply.send(result).unwrap();
                }
                _ => panic!("Expected confirmation command"),
            });
            let response = app
                .oneshot(req(
                    "POST",
                    "/api/v1/downloads/batch?confirm=true",
                    Some(&"s".repeat(32)),
                    r#"{"files":[{"username":"peer","path":"Music\\song.flac","size":10}]}"#,
                ))
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                if expected.is_ok() {
                    StatusCode::ACCEPTED
                } else {
                    StatusCode::CONFLICT
                }
            );
            worker.join().unwrap();
        }
    }
    #[tokio::test]
    async fn operational_logs_require_auth_and_filter_inert_messages() {
        use http_body_util::BodyExt;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&dir.path().join("rustysoul.sqlite3")).unwrap();
        crate::activity::append(
            &store.connection,
            &[crate::activity::Entry {
                id: 0,
                at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                level: "error".into(),
                source: "system".into(),
                message: "Permission denied <script>literal</script>".into(),
            }],
        )
        .unwrap();
        let (tx, _rx) = mpsc::sync_channel(32);
        let (_, snapshot) = watch::channel(Snapshot::default());
        let app = router(ApiState::new(
            &Config {
                data_dir: dir.path().into(),
                ..Default::default()
            },
            &"s".repeat(32),
            tx,
            snapshot,
        ));
        assert_eq!(
            app.clone()
                .oneshot(req("GET", "/api/v1/logs", None, ""))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let response = app
            .oneshot(req(
                "GET",
                "/api/v1/logs?level=error&filter=denied",
                Some(&"s".repeat(32)),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["entries"].as_array().unwrap().len(), 1);
        assert!(
            json["entries"][0]["message"]
                .as_str()
                .unwrap()
                .contains("<script>")
        );
    }
    #[tokio::test]
    async fn completed_audio_and_profile_pictures_require_auth_and_support_safe_seeking() {
        use http_body_util::BodyExt;
        use image::ImageEncoder;
        let dir = tempfile::tempdir().unwrap();
        let mut store = crate::store::Store::open(&dir.path().join("rustysoul.sqlite3")).unwrap();
        let folder = dir.path().join("downloads");
        std::fs::create_dir_all(folder.join("alice/Music")).unwrap();
        std::fs::write(folder.join("alice/Music/song.flac"), b"0123456789").unwrap();
        store
            .save_downloads(&[crate::store::SavedDownload {
                directory: folder.to_string_lossy().into_owned(),
                local_path: "alice/Music/song.flac".into(),
                username: "alice".into(),
                path: "Music\\song.flac".into(),
                size: 10,
                status: "completed".into(),
            }])
            .unwrap();
        std::fs::write(folder.join("alice/Music/empty.txt"), b"").unwrap();
        store.connection.execute("INSERT INTO downloads(username,path,size,status,directory,local_path) VALUES('alice','Music/empty.txt',0,'completed',?1,'alice/Music/empty.txt')",[folder.to_str().unwrap()]).unwrap();
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[0, 0, 255], 1, 1, image::ExtendedColorType::Rgb8)
            .unwrap();
        use base64::Engine as _;
        let profile = crate::profile::Profile {
            picture: Some(base64::engine::general_purpose::STANDARD.encode(&png)),
            ..Default::default()
        }
        .normalize()
        .unwrap();
        crate::profile::save(&store.connection, &profile).unwrap();
        let revision = crate::profile::cache_picture(&store.connection, "bob", Some(&png))
            .unwrap()
            .unwrap();
        let mut view = Snapshot {
            username: "alice".into(),
            ..Default::default()
        };
        view.users.insert(
            "bob".into(),
            crate::model::UserView {
                has_picture: true,
                picture_revision: revision,
                ..Default::default()
            },
        );
        let (_updates, snapshot) = watch::channel(view);
        let (tx, _) = mpsc::sync_channel(32);
        let token = "s".repeat(32);
        let app = router(ApiState::new(
            &Config {
                data_dir: dir.path().into(),
                ..Default::default()
            },
            &token,
            tx,
            snapshot,
        ));
        let path = "/api/v1/downloads/file?username=alice&path=Music%5Csong.flac";
        for route in [
            path,
            "/api/v1/users/picture?username=alice",
            "/api/v1/users/picture?username=bob",
        ] {
            assert_eq!(
                app.clone()
                    .oneshot(req("GET", route, None, ""))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::UNAUTHORIZED
            );
        }
        let empty = "/api/v1/downloads/file?username=alice&path=Music%2Fempty.txt&download=true";
        assert_eq!(
            app.clone()
                .oneshot(req("GET", empty, None, ""))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        for method in ["GET", "HEAD"] {
            let response = app
                .clone()
                .oneshot(req(method, empty, Some(&token), ""))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CONTENT_LENGTH], "0");
            assert!(!response.headers().contains_key(header::CONTENT_RANGE));
            assert!(
                response.headers()[header::CONTENT_DISPOSITION]
                    .to_str()
                    .unwrap()
                    .starts_with("attachment;")
            );
            assert!(
                response
                    .into_body()
                    .collect()
                    .await
                    .unwrap()
                    .to_bytes()
                    .is_empty()
            );
        }
        for value in ["bytes=0-", "bytes=-1", "bytes=0-0"] {
            let mut r = req("GET", empty, Some(&token), "");
            r.headers_mut()
                .insert(header::RANGE, HeaderValue::from_static(value));
            let response = app.clone().oneshot(r).await.unwrap();
            assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
            assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes */0");
        }
        for (range, expected) in [
            ("bytes=2-5", b"2345".as_slice()),
            ("bytes=-3", b"789".as_slice()),
        ] {
            let mut request = req("GET", path, Some(&token), "");
            request
                .headers_mut()
                .insert(header::RANGE, HeaderValue::from_str(range).unwrap());
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
            assert!(
                response.headers()[header::CONTENT_SECURITY_POLICY]
                    .to_str()
                    .unwrap()
                    .contains("media-src 'self'")
            );
            assert_eq!(
                response
                    .into_body()
                    .collect()
                    .await
                    .unwrap()
                    .to_bytes()
                    .as_ref(),
                expected
            );
        }
        let mut request = req("GET", path, Some(&token), "");
        request
            .headers_mut()
            .insert(header::RANGE, HeaderValue::from_static("bytes=10-"));
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::RANGE_NOT_SATISFIABLE
        );
        let head = app
            .clone()
            .oneshot(req("HEAD", path, Some(&token), ""))
            .await
            .unwrap();
        assert_eq!(head.headers()[header::CONTENT_LENGTH], "10");
        assert!(
            head.into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .is_empty()
        );
        for name in ["alice", "bob"] {
            let response = app
                .clone()
                .oneshot(req(
                    "GET",
                    &format!("/api/v1/users/picture?username={name}"),
                    Some(&token),
                    "",
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
            assert!(
                response
                    .into_body()
                    .collect()
                    .await
                    .unwrap()
                    .to_bytes()
                    .starts_with(b"\x89PNG\r\n\x1a\n")
            );
        }
        std::fs::remove_file(folder.join("alice/Music/song.flac")).unwrap();
        assert_eq!(
            app.oneshot(req("GET", path, Some(&token), ""))
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    #[tokio::test]
    async fn batch_route_accepts_large_valid_selection_and_rejects_invalid_files_atomically() {
        let (app, rx) = app();
        let token = "s".repeat(32);
        let files: Vec<_> = (0..128).map(|i| json!({"username":"peer", "path":format!("Music/Album/long track title number {i}.flac"), "size":1024})).collect();
        let body = json!({"files": files}).to_string();
        assert!(body.len() > 8192);
        assert_eq!(
            app.clone()
                .oneshot(req("POST", "/api/v1/downloads/batch", Some(&token), &body))
                .await
                .unwrap()
                .status(),
            StatusCode::ACCEPTED
        );
        assert!(
            matches!(rx.try_recv().unwrap(), Command::DownloadBatch{files} if files.len()==128)
        );
        for body in [
            json!({"files":[]}),
            json!({"files":[{"username":"peer","path":"../secret","size":1}]}),
        ] {
            assert_eq!(
                app.clone()
                    .oneshot(req(
                        "POST",
                        "/api/v1/downloads/batch",
                        Some(&token),
                        &body.to_string()
                    ))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::UNPROCESSABLE_ENTITY
            );
            assert!(rx.try_recv().is_err());
        }
        assert_eq!(
            app.oneshot(req("POST", "/api/v1/downloads/batch", None, &body))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    fn app() -> (Router, mpsc::Receiver<Command>) {
        let (tx, rx) = mpsc::sync_channel(32);
        let (_, view) = watch::channel(Snapshot::default());
        (
            router(ApiState::new(&Config::default(), &"s".repeat(32), tx, view)),
            rx,
        )
    }
    #[tokio::test]
    async fn http_only_sessions_restore_and_cookie_mutations_require_csrf_header() {
        let dir = tempfile::tempdir().unwrap();
        let auth = crate::auth::Auth::open(&dir.path().join("db")).unwrap();
        let (tx, _rx) = mpsc::sync_channel(32);
        let (_, snapshot) = watch::channel(Snapshot::default());
        let app = router(
            ApiState::new(&Config::default(), &"s".repeat(32), tx, snapshot).with_auth(auth),
        );
        let mut request = req(
            "POST",
            "/api/v1/auth/login",
            None,
            r#"{"username":"admin","password":"change-me-now"}"#,
        );
        request
            .headers_mut()
            .insert("x-forwarded-proto", HeaderValue::from_static("https"));
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response.headers()[header::SET_COOKIE].to_str().unwrap();
        assert!(
            cookie.contains("HttpOnly")
                && cookie.contains("SameSite=Strict")
                && cookie.contains("Secure")
                && cookie.contains("Max-Age=2592000")
        );
        let cookie = cookie.split(';').next().unwrap().to_string();
        let cookie_req = |method, path, body, cookie: &str, csrf: bool| {
            let mut r = req(method, path, None, body);
            r.headers_mut()
                .insert(header::COOKIE, HeaderValue::from_str(cookie).unwrap());
            if csrf {
                r.headers_mut()
                    .insert("x-rustysoul-request", HeaderValue::from_static("1"));
            }
            r
        };
        assert_eq!(
            app.clone()
                .oneshot(cookie_req("GET", "/api/v1/status", "", &cookie, false))
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        let response = app
            .clone()
            .oneshot(cookie_req(
                "POST",
                "/api/v1/auth/account",
                r#"{"username":"owner","current_password":"change-me-now","password":"eightpwd"}"#,
                &cookie,
                true,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let next = response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        assert_eq!(
            app.clone()
                .oneshot(cookie_req("GET", "/api/v1/auth/me", "", &cookie, false))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            app.clone()
                .oneshot(cookie_req("GET", "/api/v1/auth/me", "", &next, false))
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            app.clone()
                .oneshot(cookie_req(
                    "POST",
                    "/api/v1/commands",
                    r#"{"type":"connect"}"#,
                    &next,
                    false
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        let response = app
            .clone()
            .oneshot(cookie_req("POST", "/api/v1/auth/logout", "{}", &next, true))
            .await
            .unwrap();
        assert!(
            response.headers()[header::SET_COOKIE]
                .to_str()
                .unwrap()
                .contains("Max-Age=0")
        );
        assert_eq!(
            app.oneshot(cookie_req("GET", "/api/v1/auth/me", "", &next, false))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    #[tokio::test]
    async fn saved_messages_require_auth_and_return_archives_without_an_open_conversation() {
        use http_body_util::BodyExt;
        let dir = tempfile::tempdir().unwrap();
        let mut store = crate::store::Store::open(&dir.path().join("rustysoul.sqlite3")).unwrap();
        store
            .save_messages(&[crate::model::MessageView {
                id: 1,
                target: "archived-peer".into(),
                username: "self".into(),
                text: "saved private message".into(),
                private: true,
                timestamp: 1,
            }])
            .unwrap();
        let (_, snapshot) = watch::channel(Snapshot::default());
        let (tx, _) = mpsc::sync_channel(32);
        let app = router(ApiState::new(
            &Config {
                data_dir: dir.path().into(),
                ..Default::default()
            },
            &"s".repeat(32),
            tx,
            snapshot,
        ));
        for path in [
            "/api/v1/messages?target=archived-peer&private=true",
            "/api/v1/messages/conversations",
        ] {
            assert_eq!(
                app.clone()
                    .oneshot(req("GET", path, None, ""))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::UNAUTHORIZED
            );
            let response = app
                .clone()
                .oneshot(req("GET", path, Some(&"s".repeat(32)), ""))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = String::from_utf8(
                response
                    .into_body()
                    .collect()
                    .await
                    .unwrap()
                    .to_bytes()
                    .to_vec(),
            )
            .unwrap();
            assert!(body.contains("archived-peer"));
        }
        for path in [
            "/api/v1/messages?target=&private=true",
            "/api/v1/messages?target=archived-peer&private=true&before_id=2&after_id=1",
        ] {
            assert_eq!(
                app.clone()
                    .oneshot(req("GET", path, Some(&"s".repeat(32)), ""))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::BAD_REQUEST
            );
        }
    }
    #[tokio::test]
    async fn browse_pages_require_auth_and_an_open_listing_and_validate_queries() {
        use http_body_util::BodyExt;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&dir.path().join("rustysoul.sqlite3")).unwrap();
        let listing = crate::browse::save(
            &store.connection,
            "alice",
            [crate::model::FileView {
                username: "alice".into(),
                path: "Music\\album\\song.flac".into(),
                size: 10,
                free_slots: 0,
                speed: 0,
                attributes: Default::default(),
            }],
        )
        .unwrap();
        let mut view = Snapshot::default();
        view.browse.insert("alice".into(), listing);
        let (_updates, snapshot) = watch::channel(view);
        let (tx, _) = mpsc::sync_channel(32);
        let app = router(ApiState::new(
            &Config {
                data_dir: dir.path().into(),
                ..Default::default()
            },
            &"s".repeat(32),
            tx,
            snapshot,
        ));
        let path = "/api/v1/browse/files?username=alice";
        assert_eq!(
            app.clone()
                .oneshot(req("GET", path, None, ""))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let response = app
            .clone()
            .oneshot(req("GET", path, Some(&"s".repeat(32)), ""))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let page: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(page["total"], 1);
        assert_eq!(page["files"][0]["path"], "Music\\album\\song.flac");
        let tree_path = "/api/v1/browse/tree?username=alice&all=true";
        assert_eq!(
            app.clone()
                .oneshot(req("GET", tree_path, None, ""))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let response = app
            .clone()
            .oneshot(req("GET", tree_path, Some(&"s".repeat(32)), ""))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let tree: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(tree["total"], 2);
        assert_eq!(tree["folders"].as_array().unwrap().len(), 2);
        assert!(
            tree["folders"]
                .as_array()
                .unwrap()
                .iter()
                .any(|folder| folder["path"] == "Music\\album" && folder["parent"] == "Music")
        );
        for (path, expected) in [
            (
                "/api/v1/browse/tree?username=bob&all=true",
                StatusCode::NOT_FOUND,
            ),
            (
                "/api/v1/browse/tree?username=alice&all=true&offset=1",
                StatusCode::BAD_REQUEST,
            ),
        ] {
            assert_eq!(
                app.clone()
                    .oneshot(req("GET", path, Some(&"s".repeat(32)), ""))
                    .await
                    .unwrap()
                    .status(),
                expected
            );
        }
        assert_eq!(
            app.clone()
                .oneshot(req(
                    "GET",
                    "/api/v1/browse/files?username=bob",
                    Some(&"s".repeat(32)),
                    ""
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            app.oneshot(req(
                "GET",
                "/api/v1/browse/files?username=alice&offset=2999999",
                Some(&"s".repeat(32)),
                ""
            ))
            .await
            .unwrap()
            .status(),
            StatusCode::BAD_REQUEST
        );
    }
    #[tokio::test]
    async fn first_login_gate_cannot_be_bypassed_and_password_change_revokes_sessions() {
        use http_body_util::BodyExt;
        let dir = tempfile::tempdir().unwrap();
        let auth = crate::auth::Auth::open(&dir.path().join("db")).unwrap();
        let session = auth.login("admin", "change-me-now").unwrap();
        let (tx, _) = mpsc::sync_channel(32);
        let (_, snapshot) = watch::channel(Snapshot::default());
        let app = router(
            ApiState::new(&Config::default(), &"s".repeat(32), tx, snapshot).with_auth(auth),
        );
        for route in [
            "/api/v1/status",
            "/api/v1/events",
            "/api/v1/commands",
            "/api/v1/metrics",
        ] {
            let response = app
                .clone()
                .oneshot(req(
                    if route.ends_with("commands") {
                        "POST"
                    } else {
                        "GET"
                    },
                    route,
                    Some(&session.token),
                    r#"{"type":"connect"}"#,
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
        assert_eq!(
            app.clone()
                .oneshot(req("GET", "/api/v1/status", Some(&"s".repeat(32)), ""))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let response=app.clone().oneshot(req("POST","/api/v1/auth/account",Some(&session.token),r#"{"username":"testadmin","current_password":"change-me-now","password":"new-private-password"}"#)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let result: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        let token = result["token"].as_str().unwrap();
        assert_eq!(
            app.clone()
                .oneshot(req("GET", "/api/v1/status", Some(&session.token), ""))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            app.oneshot(req("GET", "/api/v1/status", Some(token), ""))
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
    }
    fn req(method: &str, path: &str, token: Option<&str>, body: &str) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1:8484");
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }
    #[tokio::test]
    async fn all_api_routes_require_authentication() {
        for route in [
            "/api/v1/status",
            "/api/v1/events",
            "/api/v1/commands",
            "/api/v1/maintenance",
            "/api/v1/messages/read",
            "/api/v1/rooms/read",
        ] {
            let (app, _) = app();
            let response = app
                .oneshot(req(
                    if route.ends_with("commands") || route.ends_with("/read") {
                        "POST"
                    } else {
                        "GET"
                    },
                    route,
                    None,
                    "{}",
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{route}");
        }
    }
    #[tokio::test]
    async fn private_read_markers_validate_before_queuing_and_share_the_read_rate_limit() {
        let (app, received) = app();
        let token = "s".repeat(32);
        for body in [
            r#"{"username":"","through_id":1}"#,
            r#"{"username":"peer","through_id":18446744073709551615}"#,
        ] {
            assert_eq!(
                app.clone()
                    .oneshot(req("POST", "/api/v1/messages/read", Some(&token), body))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::BAD_REQUEST
            );
            assert!(received.try_recv().is_err());
        }
        assert_eq!(
            app.clone()
                .oneshot(req(
                    "POST",
                    "/api/v1/messages/read",
                    Some(&token),
                    r#"{"username":"Peer","through_id":7}"#
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::ACCEPTED
        );
        assert!(
            matches!(received.try_recv().unwrap(), Command::ReadPrivate { username, through_id: 7 } if username=="Peer")
        );
        for _ in 0..15 {
            assert_eq!(
                app.clone()
                    .oneshot(req(
                        "POST",
                        "/api/v1/rooms/read",
                        Some(&token),
                        r#"{"room":"room","through_id":1}"#
                    ))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::ACCEPTED
            );
            received.try_recv().unwrap();
        }
        assert_eq!(
            app.oneshot(req(
                "POST",
                "/api/v1/messages/read",
                Some(&token),
                r#"{"username":"Peer","through_id":7}"#
            ))
            .await
            .unwrap()
            .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }
    #[tokio::test]
    async fn metrics_are_authenticated_readonly_text_without_private_snapshot_fields() {
        use http_body_util::BodyExt;
        let (commands, received) = mpsc::sync_channel(4);
        let (_updates, snapshot) = watch::channel(Snapshot {
            username: "private-username".into(),
            system: crate::system::View {
                database_error: Some("private-message".into()),
                ..Default::default()
            },
            ..Default::default()
        });
        let app = router(ApiState::new(
            &Config::default(),
            &"s".repeat(32),
            commands,
            snapshot,
        ));
        let response = app
            .clone()
            .oneshot(req("GET", "/api/v1/metrics", Some(&"s".repeat(32)), ""))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/plain; version=0.0.4; charset=utf-8"
        );
        assert!(
            response.headers()[header::CACHE_CONTROL]
                .to_str()
                .unwrap()
                .contains("no-store")
        );
        let text = String::from_utf8(
            response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert!(text.contains("# TYPE rustysoul_payload_uploaded_bytes_total counter\n"));
        assert!(!text.contains("private-username") && !text.contains("private-message"));
        assert!(
            received.try_recv().is_err(),
            "Metrics must not queue engine work"
        );
        assert_eq!(
            app.oneshot(req("POST", "/api/v1/metrics", Some(&"s".repeat(32)), "{}"))
                .await
                .unwrap()
                .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }
    #[tokio::test]
    async fn valid_token_can_read_status_and_headers_are_hardened() {
        let (app, _) = app();
        let response = app
            .oneshot(req("GET", "/api/v1/status", Some(&"s".repeat(32)), ""))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(
            response.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
    }
    #[tokio::test]
    async fn blocks_cross_origin_and_dns_rebinding() {
        for (name, value) in [
            ("origin", "https://evil.example"),
            ("host", "evil.example:8484"),
        ] {
            let (app, _) = app();
            let mut request = req("GET", "/api/v1/status", Some(&"s".repeat(32)), "");
            request.headers_mut().insert(
                axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
            assert_eq!(
                app.oneshot(request).await.unwrap().status(),
                StatusCode::FORBIDDEN
            );
        }
    }
    #[tokio::test]
    async fn rejects_bad_payload_before_it_reaches_engine() {
        let (app, rx) = app();
        let response = app
            .oneshot(req(
                "POST",
                "/api/v1/commands",
                Some(&"s".repeat(32)),
                r#"{"type":"download","username":"person","path":"..\\secret","size":10}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(rx.try_recv().is_err());
    }
    #[tokio::test]
    async fn accepted_means_queued_and_rate_limit_is_enforced() {
        let (app, rx) = app();
        for _ in 0..10 {
            assert_eq!(
                app.clone()
                    .oneshot(req(
                        "POST",
                        "/api/v1/commands",
                        Some(&"s".repeat(32)),
                        r#"{"type":"connect"}"#
                    ))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::ACCEPTED
            );
            assert!(rx.try_recv().is_ok());
        }
        assert_eq!(
            app.oneshot(req(
                "POST",
                "/api/v1/commands",
                Some(&"s".repeat(32)),
                r#"{"type":"connect"}"#
            ))
            .await
            .unwrap()
            .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }
    #[tokio::test]
    async fn profile_uploads_admit_ten_mb_on_a_worker_and_keep_internal_commands_private() {
        use base64::Engine as _;
        use image::ImageEncoder;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&dir.path().join("rustysoul.sqlite3")).unwrap();
        let (tx, rx) = mpsc::sync_channel(8);
        let (_updates, snapshot) = watch::channel(Snapshot::default());
        let state = ApiState::new(
            &Config {
                data_dir: dir.path().into(),
                ..Default::default()
            },
            &"s".repeat(32),
            tx,
            snapshot,
        );
        let app = router(state.clone());
        let permit = state.image_jobs.clone().acquire_owned().await.unwrap();
        assert_eq!(
            app.clone()
                .oneshot(req("POST", "/api/v1/profile", Some(&"s".repeat(32)), "{}"))
                .await
                .unwrap()
                .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(
            app.clone()
                .oneshot(req("GET", "/api/v1/profile", Some(&"s".repeat(32)), ""))
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        drop(permit);
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[12, 34, 56, 78], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        png.resize(crate::profile::MAX_INPUT_PICTURE, 0);
        let body = serde_json::to_string(&crate::profile::Profile {
            picture: Some(base64::engine::general_purpose::STANDARD.encode(png)),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            app.clone()
                .oneshot(req("POST", "/api/v1/profile", Some(&"s".repeat(32)), &body))
                .await
                .unwrap()
                .status(),
            StatusCode::ACCEPTED
        );
        let Command::UpdateProfilePrepared {
            settings,
            expected_revision,
        } = rx.try_recv().unwrap()
        else {
            panic!("Only prepared images may reach the control loop");
        };
        assert_eq!(expected_revision, 0);
        assert_eq!(
            image::load_from_memory(&settings.picture_bytes())
                .unwrap()
                .to_rgba8()
                .as_raw(),
            &[12, 34, 56, 78]
        );
        assert!(
            crate::profile::load(&store.connection)
                .unwrap()
                .picture
                .is_none(),
            "The worker must not bypass the engine's revision check"
        );
        assert_eq!(
            app.clone()
                .oneshot(req(
                    "POST",
                    "/api/v1/commands",
                    Some(&"s".repeat(32)),
                    r#"{"type":"update_profile_prepared","settings":{},"expected_revision":0}"#
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            app.oneshot(req(
                "POST",
                "/api/v1/profile",
                Some(&"s".repeat(32)),
                r#"{"picture":"not an image"}"#
            ))
            .await
            .unwrap()
            .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert!(rx.try_recv().is_err());
        assert_eq!(state.image_jobs.available_permits(), 1);
    }
    #[tokio::test]
    async fn web_app_manifest_and_icons_are_public_typed_and_keep_api_authentication() {
        use http_body_util::BodyExt;
        let (app, _) = app();
        let response = app
            .clone()
            .oneshot(req("GET", "/site.webmanifest", None, ""))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/manifest+json"
        );
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert!(
            response.headers()[header::CONTENT_SECURITY_POLICY]
                .to_str()
                .unwrap()
                .contains("manifest-src 'self'")
        );
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(manifest["name"], "RustySoul");
        assert_eq!(manifest["id"], "/");
        assert_eq!(manifest["start_url"], "/");
        assert_eq!(manifest["scope"], "/");
        assert_eq!(manifest["display"], "standalone");
        assert_eq!(manifest["icons"][0]["src"], "/assets/icons/rustysoul.svg");
        assert_eq!(manifest["icons"][0]["sizes"], "any");
        assert_eq!(manifest["icons"][0]["type"], "image/svg+xml");
        let response = app
            .clone()
            .oneshot(req("GET", "/assets/icons/rustysoul.svg", None, ""))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/svg+xml");
        assert_eq!(
            response.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let svg = std::str::from_utf8(&body).unwrap();
        assert!(svg.contains("<svg") && svg.contains("viewBox=\"0 0 40 40\""));
        assert!(svg.contains("fill=\"#344332\"") && svg.contains("stroke=\"#d7e7ae\""));
        assert!(!svg.contains("<script") && !svg.contains("href="));
        for file in [
            "favicon-16x16.png",
            "favicon-32x32.png",
            "favicon-48x48.png",
            "apple-touch-icon.png",
            "icon-192.png",
            "icon-512.png",
        ] {
            assert_eq!(
                app.clone()
                    .oneshot(req("GET", &format!("/assets/icons/{file}"), None, ""))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::NOT_FOUND
            );
        }
        assert_eq!(
            app.clone()
                .oneshot(req("GET", "/assets/icons/original.png", None, ""))
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            app.oneshot(req("GET", "/api/v1/status", None, ""))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    #[tokio::test]
    async fn bounds_request_bodies_and_does_not_leak_assets() {
        let (app, _) = app();
        assert_eq!(
            app.clone()
                .oneshot(req(
                    "POST",
                    "/api/v1/commands",
                    Some(&"s".repeat(32)),
                    &" ".repeat(9000)
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(
            app.oneshot(req("GET", "/assets/../../Cargo.toml", None, ""))
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
    }
}
