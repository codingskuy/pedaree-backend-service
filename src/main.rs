use pedaree_backend_service::config::Config;
use pedaree_backend_service::state::AppState;
use pedaree_backend_service::{db, rute};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sqlx=warn".into()),
        )
        .init();

    let config = Config::dari_env().expect("konfigurasi tidak valid");
    let pool = db::sambung(&config.database_url)
        .await
        .expect("gagal menyambung ke basis data");
    db::migrasi(&pool).await.expect("migrasi gagal");

    let state = AppState::baru(pool, config.clone()).expect("kunci JWT tidak valid");
    let app = rute(state);
    let pendengar = tokio::net::TcpListener::bind(&config.bind_addr)
        .await
        .expect("gagal mengikat port");
    tracing::info!(alamat = %config.bind_addr, "pedaree-backend-service berjalan");
    axum::serve(pendengar, app).await.expect("server berhenti");
}
