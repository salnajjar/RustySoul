use super::*;
use serde::Deserialize;
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Change {
    SetEnabled {
        enabled: bool,
        #[serde(default)]
        current_password: String,
    },
    Create {
        name: String,
        scope: crate::api_access::Scope,
        #[serde(default = "default_days")]
        expires_in_days: u16,
        current_password: String,
    },
    Revoke {
        id: String,
    },
    RevokeAll,
}
fn default_days() -> u16 {
    90
}
pub(super) async fn status(State(s): State<ApiState>) -> Response {
    let Some(auth) = s.auth else {
        return error(
            StatusCode::FORBIDDEN,
            "Sign in with the browser administrator account",
        );
    };
    match tokio::task::spawn_blocking(move || {
        auth.api_keys.flush_usage()?;
        Ok::<_, anyhow::Error>(auth.api_keys.view())
    })
    .await
    {
        Ok(Ok(view)) => Json(view).into_response(),
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, &e.to_string()),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not read API access",
        ),
    }
}
pub(super) async fn change(State(s): State<ApiState>, Json(change): Json<Change>) -> Response {
    let Some(auth) = s.auth else {
        return error(
            StatusCode::FORBIDDEN,
            "Sign in with the browser administrator account",
        );
    };
    match tokio::task::spawn_blocking(move || -> anyhow::Result<serde_json::Value> {
        match change {
            Change::SetEnabled {
                enabled,
                current_password,
            } => {
                if enabled {
                    auth.confirm(&current_password, || auth.api_keys.set_enabled(true))?;
                } else {
                    auth.api_keys.set_enabled(false)?;
                }
            }
            Change::Create {
                name,
                scope,
                expires_in_days,
                current_password,
            } => {
                return auth.confirm(&current_password, || {
                    Ok(serde_json::to_value(auth.api_keys.create(
                        &name,
                        scope,
                        expires_in_days,
                    )?)?)
                });
            }
            Change::Revoke { id } => auth.api_keys.revoke(Some(&id))?,
            Change::RevokeAll => auth.api_keys.revoke(None)?,
        }
        Ok(serde_json::to_value(auth.api_keys.view())?)
    })
    .await
    {
        Ok(Ok(view)) => Json(view).into_response(),
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, &e.to_string()),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not update API access",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    fn fixture() -> (tempfile::TempDir, ApiState, String, mpsc::Receiver<Command>) {
        let t = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&t.path().join("rustysoul.sqlite3")).unwrap();
        store.identity().unwrap();
        let auth = crate::auth::Auth::open(&t.path().join("rustysoul.sqlite3")).unwrap();
        let initial = auth.login("admin", "change-me-now").unwrap();
        let login = auth
            .change(&initial.token, "change-me-now", "owner", "eightpwd")
            .unwrap();
        let (tx, rx) = mpsc::sync_channel(32);
        let (_, view) = watch::channel(Snapshot::default());
        let state = ApiState::new(
            &Config {
                data_dir: t.path().into(),
                ..Default::default()
            },
            &"s".repeat(32),
            tx,
            view,
        )
        .with_auth(auth);
        (t, state, login.token, rx)
    }
    async fn req(
        app: &Router,
        path: &str,
        token: &str,
        body: Option<serde_json::Value>,
    ) -> Response {
        app.clone()
            .oneshot(
                Request::builder()
                    .method(if body.is_some() { "POST" } else { "GET" })
                    .uri(format!("/api/v1/{path}"))
                    .header("Host", "localhost")
                    .header("Authorization", format!("Bearer {token}"))
                    .header("Content-Type", "application/json")
                    .body(Body::from(body.map_or(String::new(), |b| b.to_string())))
                    .unwrap(),
            )
            .await
            .unwrap()
    }
    async fn json(response: Response) -> serde_json::Value {
        serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 65536)
                .await
                .unwrap(),
        )
        .unwrap()
    }
    #[tokio::test]
    async fn integration_keys_obey_scope_and_cannot_manage_security_or_use_cookie_auth() {
        let (_t, state, session, _rx) = fixture();
        let app = router(state.clone());
        assert_eq!(
            req(&app, "api-access", "invalid", None).await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            req(
                &app,
                "api-access",
                &session,
                Some(json!({"action":"set_enabled","enabled":true,"current_password":"wrong"}))
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            req(
                &app,
                "api-access",
                &session,
                Some(json!({"action":"set_enabled","enabled":true,"current_password":"eightpwd"}))
            )
            .await
            .status(),
            StatusCode::OK
        );
        let read=json(req(&app,"api-access",&session,Some(json!({"action":"create","name":"reader","scope":"read_only","expires_in_days":30,"current_password":"eightpwd"}))).await).await;
        let read_key = read["token"].as_str().unwrap();
        assert_eq!(
            req(&app, "status", read_key, None).await.status(),
            StatusCode::OK
        );
        assert_eq!(
            req(&app, "transfers/downloads?offset=0", read_key, None)
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            req(
                &app,
                "commands",
                read_key,
                Some(json!({"type":"search","query":"test"}))
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        let control=json(req(&app,"api-access",&session,Some(json!({"action":"create","name":"controller","scope":"control","expires_in_days":0,"current_password":"eightpwd"}))).await).await;
        let key = control["token"].as_str().unwrap();
        assert_eq!(
            req(
                &app,
                "commands",
                key,
                Some(json!({"type":"search","query":"test"}))
            )
            .await
            .status(),
            StatusCode::ACCEPTED
        );
        for (path, body) in [
            ("api-access", None),
            ("backups", None),
            ("maintenance", Some(json!({"action":"compact"}))),
            ("auth/account", Some(json!({}))),
        ] {
            assert_eq!(
                req(&app, path, key, body).await.status(),
                StatusCode::FORBIDDEN,
                "{path}"
            );
        }
        let cookie = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/status")
                    .header("Host", "localhost")
                    .header("Cookie", format!("rustysoul_session={key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(cookie.status(), StatusCode::UNAUTHORIZED);
        let view = json(req(&app, "api-access", &session, None).await).await;
        assert!(!view.to_string().contains(key));
        assert!(!view.to_string().contains(read_key));
        assert_eq!(
            req(
                &app,
                "api-access",
                &session,
                Some(json!({"action":"set_enabled","enabled":false}))
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            req(&app, "status", key, None).await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            req(&app, "status", &session, None).await.status(),
            StatusCode::OK
        );
    }
    #[tokio::test]
    async fn revocation_closes_an_api_event_stream_and_password_changes_disable_keys() {
        let (_t, state, session, _rx) = fixture();
        let auth = state.auth.clone().unwrap();
        auth.api_keys.set_enabled(true).unwrap();
        let key = auth
            .api_keys
            .create("stream", crate::api_access::Scope::ReadOnly, 30)
            .unwrap();
        let app = router(state);
        let response = req(&app, "events", &key.token, None).await;
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body();
        assert!(body.frame().await.is_some());
        auth.api_keys.revoke(Some(&key.key.id)).unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(3), body.frame())
                .await
                .unwrap()
                .is_none()
        );
        let another = auth
            .api_keys
            .create("other", crate::api_access::Scope::Control, 0)
            .unwrap();
        auth.change(&session, "eightpwd", "new-owner", "differentpwd")
            .unwrap();
        assert!(auth.api_keys.authenticate(&another.token, false).is_none());
        assert!(!auth.api_keys.view().enabled);
    }
}
