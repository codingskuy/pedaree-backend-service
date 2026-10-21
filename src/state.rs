//! Status bersama aplikasi.

use std::sync::Arc;

use sqlx::PgPool;

use crate::auth::KunciJwt;
use crate::config::Config;
use crate::error::AppError;
use crate::waktu::{jam_sistem, Jam};

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
    pub kunci: Arc<KunciJwt>,
    /// Jam yang dapat diganti di uji (kedaluwarsa dihitung per hari WIB).
    pub jam: Jam,
}

impl AppState {
    pub fn baru(db: PgPool, config: Config) -> Result<Self, AppError> {
        let kunci = KunciJwt::dari_config(&config)?;
        Ok(Self {
            db,
            config: Arc::new(config),
            kunci: Arc::new(kunci),
            jam: jam_sistem(),
        })
    }

    pub fn dengan_jam(mut self, jam: Jam) -> Self {
        self.jam = jam;
        self
    }
}
