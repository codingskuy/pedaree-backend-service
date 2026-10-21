//! Koneksi basis data `pedaree` dan migrasi berversi (`sqlx::migrate!`).

use sqlx::migrate::Migrator;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

use crate::error::AppError;

/// Migrasi tertanam (`migrations/NNN_nama.up.sql` + `.down.sql`).
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

pub async fn sambung(database_url: &str) -> Result<PgPool, AppError> {
    PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await
        .map_err(AppError::from)
}

pub async fn migrasi(db: &PgPool) -> Result<(), AppError> {
    MIGRATOR.run(db).await?;
    Ok(())
}
