use std::sync::Arc;

use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};

use crate::auth::session::SessionState;
use crate::db;
use crate::db::display_preferences::UpdateDisplayPreferences;
use crate::error::AppError;

fn map_update_error(message: String) -> AppError {
    let is_invalid_preference = message.starts_with("Invalid density:")
        || message.starts_with("Invalid theme:")
        || message.starts_with("Invalid compose_format:")
        || message.starts_with("Invalid animation_mode:");

    if is_invalid_preference {
        AppError::BadRequest(message)
    } else {
        AppError::InternalError(message)
    }
}

/// `GET /api/settings/display`
pub async fn get_display_preferences(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
) -> Result<Response, AppError> {
    let prefs = db::pool::with_user_db(&db_pool_manager, &session.user_hash, |conn| {
        db::display_preferences::get_preferences(conn)
    })
    .await
    .map_err(AppError::InternalError)?;

    Ok(Json(prefs).into_response())
}

/// `PUT /api/settings/display`
pub async fn update_display_preferences(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Json(data): Json<UpdateDisplayPreferences>,
) -> Result<Response, AppError> {
    let prefs = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        db::display_preferences::update_preferences(conn, &data)
    })
    .await
    .map_err(map_update_error)?;

    Ok(Json(prefs).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    use axum::http::StatusCode;
    use axum::Extension;
    use axum::response::IntoResponse;
    use serde_json::json;
    use tempfile::TempDir;

    fn test_db_pool_manager(data_dir: &str) -> Arc<db::pool::DbPoolManager> {
        Arc::new(db::pool::DbPoolManager::new(
            data_dir.to_string(),
            4,
            std::time::Duration::from_secs(600),
            500,
        ))
    }

    fn test_session() -> SessionState {
        SessionState {
            account_id: "acc-1".to_string(),
            email: "alice@example.com".to_string(),
            password: "password".to_string(),
            user_hash: crate::auth::user_data::hash_email("alice@example.com"),
            imap_host: "imap.example.com".to_string(),
            imap_port: 993,
            imap_tls: true,
            smtp_host: "smtp.example.com".to_string(),
            smtp_port: 587,
            smtp_tls: true,
            last_accessed: Instant::now(),
            timeout_override: None,
            folder_key: [0u8; 32],
        }
    }

    #[tokio::test]
    async fn test_invalid_preferences_map_to_bad_request() {
        let tmp = TempDir::new().unwrap();
        let db_pool_manager = test_db_pool_manager(tmp.path().to_str().unwrap());
        let session = test_session();
        crate::auth::user_data::provision_user_data(tmp.path().to_str().unwrap(), &session.user_hash)
            .expect("failed to provision user db");

        let result = update_display_preferences(
            Extension(session),
            Extension(db_pool_manager),
            Json(UpdateDisplayPreferences {
                density: None,
                theme: None,
                language: None,
                compose_format: None,
                deep_index: None,
                animation_mode: Some(Some("ultra".to_string())),
                mobile_nav_style: None,
                mobile_nav_tabs: None,
                mobile_compose: None,
                undo_send_delay: None,
            }),
        )
        .await;

        match result {
            Err(AppError::BadRequest(message)) => {
                assert!(message.contains("Invalid animation_mode"));
            }
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[test]
    fn test_map_update_error_invalid_animation_mode_returns_bad_request() {
        let err = map_update_error("Invalid animation_mode: ultra".to_string());

        match err {
            AppError::BadRequest(message) => {
                assert!(message.contains("Invalid animation_mode"));
            }
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_invalid_preferences_map_to_http_400_response() {
        let tmp = TempDir::new().unwrap();
        let db_pool_manager = test_db_pool_manager(tmp.path().to_str().unwrap());
        let session = test_session();
        crate::auth::user_data::provision_user_data(tmp.path().to_str().unwrap(), &session.user_hash)
            .expect("failed to provision user db");

        let result = update_display_preferences(
            Extension(session),
            Extension(db_pool_manager),
            Json(UpdateDisplayPreferences {
                density: None,
                theme: None,
                language: None,
                compose_format: None,
                deep_index: None,
                animation_mode: Some(Some("ultra".to_string())),
                mobile_nav_style: None,
                mobile_nav_tabs: None,
                mobile_compose: None,
                undo_send_delay: None,
            }),
        )
        .await;

        let response = result
            .expect_err("invalid preferences should return an error")
            .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_animation_mode_deserialize_omitted_is_none() {
        let payload = json!({
            "theme": "dark"
        });

        let parsed: UpdateDisplayPreferences =
            serde_json::from_value(payload).expect("payload should deserialize");

        assert_eq!(parsed.animation_mode, None);
    }

    #[test]
    fn test_animation_mode_deserialize_null_is_some_none() {
        let payload = json!({
            "animation_mode": null
        });

        let parsed: UpdateDisplayPreferences =
            serde_json::from_value(payload).expect("payload should deserialize");

        assert_eq!(parsed.animation_mode, Some(None));
    }

    #[test]
    fn test_animation_mode_deserialize_value_is_some_some() {
        let payload = json!({
            "animation_mode": "medium"
        });

        let parsed: UpdateDisplayPreferences =
            serde_json::from_value(payload).expect("payload should deserialize");

        assert_eq!(
            parsed.animation_mode,
            Some(Some("medium".to_string()))
        );
    }
}
