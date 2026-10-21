//! Status bersama aplikasi.

use std::sync::Arc;

use sqlx::PgPool;

use crate::auth::KunciJwt;
use crate::config::Config;
use crate::error::AppError;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
    pub kunci: Arc<KunciJwt>,
}

impl AppState {
    pub fn baru(db: PgPool, config: Config) -> Result<Self, AppError> {
        let kunci = KunciJwt::dari_config(&config)?;
        Ok(Self {
            db,
            config: Arc::new(config),
            kunci: Arc::new(kunci),
        })
    }
}
