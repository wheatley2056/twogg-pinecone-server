//! Fehler als JSON (`{"error": "…"}`) mit passendem Statuscode.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "Nicht gefunden")
    }

    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "Bitte anmelden")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if self.status.is_server_error() {
            tracing::error!("{}", self.message);
        }
        // Interne Details nicht nach außen geben.
        let message = if self.status.is_server_error() {
            "Interner Fehler".to_string()
        } else {
            self.message
        };
        (self.status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::RowNotFound => Self::not_found(),
            e => Self::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Datenbank: {e}")),
        }
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Datei: {e}"))
    }
}

/// Wie `Json<T>`, aber ungültige Eingaben werden als lesbarer 400-Fehler gemeldet.
pub struct JsonBody<T>(pub T);

impl<S, T> FromRequest<S> for JsonBody<T>
where
    Json<T>: FromRequest<S, Rejection = JsonRejection>,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        Json::<T>::from_request(req, state)
            .await
            .map(|Json(v)| JsonBody(v))
            .map_err(|e| ApiError::bad_request(format!("Ungültige Eingabe: {}", e.body_text())))
    }
}

/// Für Formularfelder: `null` wie „nicht angegeben“ behandeln.
pub fn null_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + serde::Deserialize<'de>,
{
    use serde::Deserialize;
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}
