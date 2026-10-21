//! pedaree-backend-service: sumber kebenaran stok dapur (FEFO) Pedaree.

pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod state;

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::error::{rute_tidak_ada, AppError};
use crate::state::AppState;

#[derive(Serialize)]
struct Sehat {
    status: &'static str,
    db: &'static str,
}

async fn sehat(State(state): State<AppState>) -> Result<Json<Sehat>, AppError> {
    sqlx::query("SELECT 1").execute(&state.db).await?;
    Ok(Json(Sehat {
        status: "ok",
        db: "pedaree",
    }))
}

/// Router aplikasi. Path memakai sintaks Axum 0.8 (`{id}`).
pub fn rute(state: AppState) -> Router {
    Router::new()
        .route("/kesehatan", get(sehat))
        .merge(auth::rute())
        .fallback(rute_tidak_ada)
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(30),
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
