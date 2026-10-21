//! Galat baku layanan: satu `AppError` yang memetakan 400/401/404/409/422/429/500
//! ke badan JSON `{kode, pesan, field}` (kontrak `AppError` di `docs/api/openapi.yaml`).

use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

/// Badan galat yang dikirim ke klien.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Detail {
    pub kode: String,
    pub pesan: String,
    pub field: Option<String>,
}

impl Detail {
    pub fn baru(kode: &str, pesan: impl Into<String>, field: Option<&str>) -> Self {
        Self {
            kode: kode.to_owned(),
            pesan: pesan.into(),
            field: field.map(str::to_owned),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// 400: format atau field salah.
    #[error("validasi gagal: {}", .0.pesan)]
    Validasi(Detail),
    /// 401: token/sesi tidak sah, audiens salah.
    #[error("tidak sah: {}", .0.pesan)]
    TidakSah(Detail),
    /// 404: sumber daya tidak ditemukan.
    #[error("tidak ada: {}", .0.pesan)]
    TidakAda(Detail),
    /// 409: konflik (nama ganda, refresh dipakai ulang, konflik opname).
    #[error("konflik: {}", .0.pesan)]
    Konflik(Detail),
    /// 422: aturan domain dilanggar.
    #[error("tak terproses: {}", .0.pesan)]
    TakTerproses(Detail),
    /// 429: melebihi batas laju.
    #[error("batas laju: {}", .0.pesan)]
    BatasLaju(Detail),
    /// 500: galat internal; rincian hanya masuk log, bukan respons.
    #[error("galat internal: {0}")]
    Internal(String),
}

impl AppError {
    pub fn validasi(pesan: impl Into<String>, field: Option<&str>) -> Self {
        Self::Validasi(Detail::baru("VALIDASI_GAGAL", pesan, field))
    }

    pub fn tidak_sah(pesan: impl Into<String>) -> Self {
        Self::TidakSah(Detail::baru("TIDAK_SAH", pesan, None))
    }

    pub fn tidak_ada(pesan: impl Into<String>) -> Self {
        Self::TidakAda(Detail::baru("TIDAK_ADA", pesan, None))
    }

    pub fn konflik(pesan: impl Into<String>) -> Self {
        Self::Konflik(Detail::baru("KONFLIK", pesan, None))
    }

    pub fn tak_terproses(kode: &str, pesan: impl Into<String>, field: Option<&str>) -> Self {
        Self::TakTerproses(Detail::baru(kode, pesan, field))
    }

    pub fn batas_laju(pesan: impl Into<String>) -> Self {
        Self::BatasLaju(Detail::baru("BATAS_LAJU", pesan, None))
    }

    pub fn internal(pesan: impl Into<String>) -> Self {
        Self::Internal(pesan.into())
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::Validasi(_) => StatusCode::BAD_REQUEST,
            Self::TidakSah(_) => StatusCode::UNAUTHORIZED,
            Self::TidakAda(_) => StatusCode::NOT_FOUND,
            Self::Konflik(_) => StatusCode::CONFLICT,
            Self::TakTerproses(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::BatasLaju(_) => StatusCode::TOO_MANY_REQUESTS,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn detail(&self) -> Detail {
        match self {
            Self::Validasi(d)
            | Self::TidakSah(d)
            | Self::TidakAda(d)
            | Self::Konflik(d)
            | Self::TakTerproses(d)
            | Self::BatasLaju(d) => d.clone(),
            Self::Internal(_) => Detail::baru("GALAT_SERVER", "Terjadi galat internal", None),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        if let Self::Internal(rincian) = &self {
            tracing::error!(rincian = %rincian, "galat internal");
        }
        (self.status(), Json(self.detail())).into_response()
    }
}

/// Konstrain basis data khusus (lihat migrasi 002) dipetakan ke aturan domain.
impl From<sqlx::Error> for AppError {
    fn from(galat: sqlx::Error) -> Self {
        if let sqlx::Error::Database(db) = &galat {
            match db.code().as_deref() {
                Some("23505") => {
                    return Self::konflik("Data dengan nilai unik yang sama sudah ada");
                }
                Some("PDI01") => {
                    return Self::tak_terproses(
                        "BATAS_ITEM_AKTIF",
                        "Maksimum 400 item aktif per rumah tangga",
                        None,
                    );
                }
                Some("PDB01") => {
                    return Self::tak_terproses(
                        "BATAS_BATCH",
                        "Maksimum 2.000 batch berisi per rumah tangga",
                        None,
                    );
                }
                _ => {}
            }
        }
        Self::Internal(format!("basis data: {galat}"))
    }
}

impl From<sqlx::migrate::MigrateError> for AppError {
    fn from(galat: sqlx::migrate::MigrateError) -> Self {
        Self::Internal(format!("migrasi: {galat}"))
    }
}

impl From<JsonRejection> for AppError {
    fn from(r: JsonRejection) -> Self {
        Self::validasi(format!("Badan JSON tidak valid: {}", r.body_text()), None)
    }
}

impl From<QueryRejection> for AppError {
    fn from(r: QueryRejection) -> Self {
        Self::validasi(
            format!("Parameter query tidak valid: {}", r.body_text()),
            None,
        )
    }
}

impl From<PathRejection> for AppError {
    fn from(r: PathRejection) -> Self {
        Self::validasi(
            format!("Parameter path tidak valid: {}", r.body_text()),
            None,
        )
    }
}

/// `Json` dengan penolakan berbentuk `AppError` (bukan teks polos bawaan axum).
#[derive(axum::extract::FromRequest)]
#[from_request(via(axum::Json), rejection(AppError))]
pub struct JsonBaku<T>(pub T);

/// `Query` dengan penolakan berbentuk `AppError`.
#[derive(axum::extract::FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(AppError))]
pub struct QueryBaku<T>(pub T);

/// `Path` dengan penolakan berbentuk `AppError`.
#[derive(axum::extract::FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(AppError))]
pub struct PathBaku<T>(pub T);

/// Fallback rute tak dikenal: tetap berbentuk galat baku.
pub async fn rute_tidak_ada() -> AppError {
    AppError::TidakAda(Detail::baru("RUTE_TIDAK_ADA", "Rute tidak ditemukan", None))
}

#[cfg(test)]
mod uji {
    use super::*;
    use http_body_util::BodyExt;

    async fn badan(r: Response) -> (StatusCode, serde_json::Value) {
        let status = r.status();
        let bytes = r.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn setiap_varian_dipetakan_ke_status_dan_bentuk_baku() {
        let kasus = [
            (
                AppError::validasi("x", Some("jumlah")),
                400,
                "VALIDASI_GAGAL",
            ),
            (AppError::tidak_sah("x"), 401, "TIDAK_SAH"),
            (AppError::tidak_ada("x"), 404, "TIDAK_ADA"),
            (AppError::konflik("x"), 409, "KONFLIK"),
            (
                AppError::tak_terproses("STOK_HABIS", "x", None),
                422,
                "STOK_HABIS",
            ),
            (AppError::batas_laju("x"), 429, "BATAS_LAJU"),
            (AppError::internal("rahasia"), 500, "GALAT_SERVER"),
        ];
        for (galat, status, kode) in kasus {
            let (s, json) = badan(galat.into_response()).await;
            assert_eq!(s.as_u16(), status);
            assert_eq!(json["kode"], kode);
            assert!(json["pesan"].is_string());
            assert!(json.get("field").is_some(), "field wajib ada (boleh null)");
        }
    }

    #[tokio::test]
    async fn galat_internal_tidak_membocorkan_rincian() {
        let (_, json) = badan(AppError::internal("password=abc di query").into_response()).await;
        assert!(!json.to_string().contains("password"));
    }

    #[tokio::test]
    async fn field_diteruskan_ke_klien() {
        let (_, json) = badan(AppError::validasi("kurang", Some("jumlah")).into_response()).await;
        assert_eq!(json["field"], "jumlah");
    }
}
