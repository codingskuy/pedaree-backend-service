//! Konfigurasi dari variabel lingkungan. Tidak ada rahasia di kode atau repo.

use std::fmt;

use crate::error::AppError;

/// Nilai rahasia; `Debug` tidak pernah mencetak isinya (aman untuk log).
#[derive(Clone)]
pub struct Rahasia(String);

impl Rahasia {
    pub fn baru(nilai: impl Into<String>) -> Self {
        Self(nilai.into())
    }

    pub fn nilai(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Rahasia {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Rahasia(***)")
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bind_addr: String,
    /// Kunci privat Ed25519 (PEM PKCS#8) penanda tangan JWT akses. Wajib.
    pub jwt_private_pem: Rahasia,
    /// Kunci publik Ed25519 (PEM SPKI) pemverifikasi JWT akses. Wajib.
    pub jwt_public_pem: String,
    pub jwt_issuer: String,
    /// Service key least-privilege untuk panggilan keluar ke TitipO (`X-Service-Key`).
    pub service_key: Option<Rahasia>,
    /// Alamat dasar TitipO (mis. `https://api.titipo.chefgenie.id`).
    pub titipo_base_url: Option<String>,
}

/// PEM di variabel lingkungan lazim ditulis satu baris dengan `\n` literal.
fn normalkan_pem(nilai: &str) -> String {
    nilai.replace("\\n", "\n")
}

impl Config {
    /// Membaca konfigurasi dari proses lingkungan.
    pub fn dari_env() -> Result<Self, AppError> {
        Self::dari(|k| std::env::var(k).ok())
    }

    pub fn dari(ambil: impl Fn(&str) -> Option<String>) -> Result<Self, AppError> {
        let wajib = |k: &str| {
            ambil(k)
                .filter(|v| !v.trim().is_empty())
                .ok_or_else(|| AppError::internal(format!("{k} belum diatur")))
        };
        let opsional = |k: &str| ambil(k).filter(|v| !v.trim().is_empty());
        Ok(Self {
            database_url: wajib("DATABASE_URL")?,
            bind_addr: opsional("BIND_ADDR").unwrap_or_else(|| "0.0.0.0:8080".to_owned()),
            jwt_private_pem: Rahasia::baru(normalkan_pem(&wajib("JWT_ED25519_PRIVATE")?)),
            jwt_public_pem: normalkan_pem(&wajib("JWT_ED25519_PUBLIC")?),
            jwt_issuer: opsional("JWT_ISSUER")
                .unwrap_or_else(|| "pedaree-backend-service".to_owned()),
            service_key: opsional("PEDAREE_SERVICE_KEY").map(Rahasia::baru),
            titipo_base_url: opsional("TITIPO_BASE_URL"),
        })
    }
}

#[cfg(test)]
mod uji {
    use super::*;

    fn env(pasangan: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| {
            pasangan
                .iter()
                .find(|(nama, _)| *nama == k)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    const DASAR: &[(&str, &str)] = &[
        ("DATABASE_URL", "postgres://x"),
        (
            "JWT_ED25519_PRIVATE",
            "-----BEGIN PRIVATE KEY-----\\nabc\\n-----END PRIVATE KEY-----",
        ),
        (
            "JWT_ED25519_PUBLIC",
            "-----BEGIN PUBLIC KEY-----\\nxyz\\n-----END PUBLIC KEY-----",
        ),
    ];

    #[test]
    fn database_url_dan_kunci_jwt_wajib() {
        assert!(Config::dari(|_| None).is_err());
        assert!(Config::dari(env(&[("DATABASE_URL", "postgres://x")])).is_err());
    }

    #[test]
    fn punya_default_dan_menormalkan_pem() {
        let c = Config::dari(env(DASAR)).unwrap();
        assert_eq!(c.bind_addr, "0.0.0.0:8080");
        assert_eq!(c.jwt_issuer, "pedaree-backend-service");
        assert!(c.jwt_public_pem.contains('\n'));
        assert!(c.service_key.is_none() && c.titipo_base_url.is_none());
    }

    #[test]
    fn service_key_berasal_dari_konfigurasi_dan_tidak_tercetak() {
        let mut pasangan = DASAR.to_vec();
        pasangan.push(("PEDAREE_SERVICE_KEY", "kunci-sangat-rahasia"));
        let pasangan: &'static [(&str, &str)] = Box::leak(pasangan.into_boxed_slice());
        let c = Config::dari(env(pasangan)).unwrap();
        assert_eq!(
            c.service_key.as_ref().unwrap().nilai(),
            "kunci-sangat-rahasia"
        );
        let cetak = format!("{c:?}");
        assert!(!cetak.contains("kunci-sangat-rahasia"));
        assert!(!cetak.contains("BEGIN PRIVATE KEY"));
    }
}
