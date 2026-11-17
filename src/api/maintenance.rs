use super::*;
use serde::Deserialize;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RequestAction {
    action: crate::maintenance::Action,
}
pub(super) async fn status(State(s): State<ApiState>) -> Response {
    let Ok(permit) = s.maintenance_reads.clone().try_acquire_owned() else {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "Maintenance status is being read; try again shortly",
        );
    };
    let data = s.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        (
            crate::maintenance::stats(&data),
            crate::maintenance::last_result(&data),
        )
    })
    .await;
    match result {
        Ok((stats, last)) => {
            let (stats, stats_error) = match stats {
                Ok(stats) => (Some(stats), None),
                Err(e) => (None, Some(e.to_string())),
            };
            Json(json!({"stats":stats,"stats_error":stats_error,"last_result":last,"pending":crate::maintenance::pending(&s.data_dir),"pending_id":crate::maintenance::pending_id(&s.data_dir),"busy":s.backups.gate.available_permits()==0,"can_restart":s.restart.is_some()})).into_response()
        }
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not read maintenance status",
        ),
    }
}
pub(super) async fn start(
    State(s): State<ApiState>,
    Extension(permit): Extension<backups::Permit>,
    Json(request): Json<RequestAction>,
) -> Response {
    let Some(restart) = s.restart.clone() else {
        return error(
            StatusCode::CONFLICT,
            "Database maintenance requires the supervised daemon; restart support is unavailable",
        );
    };
    let config = s.backup_config.clone();
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        crate::maintenance::stage(&config, request.action)
    })
    .await
    {
        Ok(Ok(id)) => {
            restart.notify_one();
            Json(json!({"status":"restart_requested","id":id})).into_response()
        }
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, &e.to_string()),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not prepare database maintenance",
        ),
    }
}
