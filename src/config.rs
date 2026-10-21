//! Konfigurasi dari variabel lingkungan. Tidak ada rahasia di kode atau repo.

use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bind_addr: String,
}

impl Config {
    /// Membaca `DATABASE_URL` (wajib) dan `BIND_ADDR` (default `0.0.0.0:8080`).
    pub fn dari_env() -> Result<Self, AppError> {
        Self::dari(|k| std::env::var(k).ok())
    }

    pub fn dari(ambil: impl Fn(&str) -> Option<String>) -> Result<Self, AppError> {
        let database_url = ambil("DATABASE_URL")
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| AppError::internal("DATABASE_URL belum diatur"))?;
        Ok(Self {
            database_url,
            bind_addr: ambil("BIND_ADDR").unwrap_or_else(|| "0.0.0.0:8080".to_owned()),
        })
    }
}

#[cfg(test)]
mod uji {
    use super::*;

    #[test]
    fn database_url_wajib() {
        assert!(Config::dari(|_| None).is_err());
    }

    #[test]
    fn bind_addr_punya_default() {
        let c = Config::dari(|k| (k == "DATABASE_URL").then(|| "postgres://x".to_owned())).unwrap();
        assert_eq!(c.bind_addr, "0.0.0.0:8080");
    }
}
