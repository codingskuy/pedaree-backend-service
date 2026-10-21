//! Status bersama aplikasi. Sengaja minimal: `AppState { db: PgPool }`.

use sqlx::PgPool;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
}

impl AppState {
    pub fn baru(db: PgPool) -> Self {
        Self { db }
    }
}
