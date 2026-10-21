//! Idempotensi `X-Request-Id`: hasil POST disimpan 24 jam; kirim ulang dengan ID sama
//! mengembalikan hasil asli tanpa mutasi ganda.
//!
//! Dipakai di dalam transaksi permintaan: `mulai` mengunci kunci (advisory lock) lalu
//! mencari hasil tersimpan; `simpan` menulis hasil pada transaksi yang sama dengan mutasi,
//! sehingga mutasi dan catatan idempotensi sama-sama berhasil atau sama-sama dibatalkan.
//! Galat tidak disimpan (transaksi dibatalkan), jadi klien boleh mencoba lagi.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use serde_json::Value;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::error::AppError;
use crate::state::AppState;

/// Header `X-Request-Id` (UUID) wajib pada setiap POST.
#[derive(Debug, Clone, Copy)]
pub struct IdPermintaan(pub Uuid);

impl FromRequestParts<AppState> for IdPermintaan {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let nilai = parts
            .headers
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AppError::validasi("Header X-Request-Id wajib", Some("X-Request-Id")))?;
        Uuid::parse_str(nilai.trim())
            .map(IdPermintaan)
            .map_err(|_| AppError::validasi("X-Request-Id harus UUID", Some("X-Request-Id")))
    }
}

pub struct Kunci {
    pub household_id: String,
    pub request_id: Uuid,
    /// Identitas operasi, mis. `POST /pantry/consume`; ID sama untuk operasi lain = 409.
    pub endpoint: String,
}

impl Kunci {
    pub fn baru(household_id: &str, id: IdPermintaan, endpoint: impl Into<String>) -> Self {
        Self {
            household_id: household_id.to_owned(),
            request_id: id.0,
            endpoint: endpoint.into(),
        }
    }
}

/// Mengunci kunci dan mengembalikan hasil tersimpan (status, badan) bila ada dan belum 24 jam.
pub async fn mulai(
    conn: &mut PgConnection,
    kunci: &Kunci,
) -> Result<Option<(StatusCode, Value)>, AppError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(format!("idem:{}:{}", kunci.household_id, kunci.request_id))
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "DELETE FROM idempotensi
          WHERE household_id = $1 AND request_id = $2 AND dibuat_pada < now() - interval '24 hours'",
    )
    .bind(&kunci.household_id)
    .bind(kunci.request_id)
    .execute(&mut *conn)
    .await?;
    let baris: Option<(String, i32, Value)> = sqlx::query_as(
        "SELECT endpoint, status_kode, respons FROM idempotensi
          WHERE household_id = $1 AND request_id = $2",
    )
    .bind(&kunci.household_id)
    .bind(kunci.request_id)
    .fetch_optional(&mut *conn)
    .await?;
    match baris {
        None => Ok(None),
        Some((endpoint, _, _)) if endpoint != kunci.endpoint => {
            Err(AppError::Konflik(crate::error::Detail::baru(
                "IDEMPOTENSI_BENTROK",
                "X-Request-Id sudah dipakai untuk permintaan lain",
                Some("X-Request-Id"),
            )))
        }
        Some((_, status, badan)) => {
            let status = StatusCode::from_u16(status as u16)
                .map_err(|_| AppError::internal("status idempotensi tidak valid"))?;
            Ok(Some((status, badan)))
        }
    }
}

pub async fn simpan(
    conn: &mut PgConnection,
    kunci: &Kunci,
    status: StatusCode,
    badan: &Value,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO idempotensi (household_id, request_id, endpoint, status_kode, respons)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&kunci.household_id)
    .bind(kunci.request_id)
    .bind(&kunci.endpoint)
    .bind(i32::from(status.as_u16()))
    .bind(badan)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Menghapus catatan lebih dari 24 jam (dipanggil berkala dari `main`).
pub async fn bersihkan(db: &PgPool) -> Result<u64, AppError> {
    let r = sqlx::query("DELETE FROM idempotensi WHERE dibuat_pada < now() - interval '24 hours'")
        .execute(db)
        .await?;
    Ok(r.rows_affected())
}
