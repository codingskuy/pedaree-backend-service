//! Pengirim outbox `sinyal_stok` ke TitipO `POST /sinyal/pedaree-stok`.
//!
//! - Header `X-Service-Key` (service key least-privilege dari konfigurasi).
//! - `201` = `dicatat_sebagai_anjuran`; `200` = `duplikat_diabaikan` (bukan galat).
//! - Kegagalan sementara (jaringan, 5xx, 408, 429): retry 3x dengan jeda 5/30/300 detik, lalu
//!   DLQ. Kegagalan permanen (4xx lain, mis. 422 `JENIS_SINYAL_TIDAK_DIKENAL`): langsung DLQ.
//! - Hanya satu endpoint yang pernah dipanggil; tidak ada pembuatan pesanan TitipO.

use std::time::Duration as DurasiStd;

use chrono::{DateTime, Duration, Utc};
use reqwest::{Client, StatusCode};
use serde_json::Value;
use sqlx::PgPool;

use crate::config::{Config, Rahasia};
use crate::error::AppError;

pub const PATH_PENERIMA: &str = "/sinyal/pedaree-stok";

/// Jeda retry setelah kegagalan ke-1/2/3; kegagalan ke-4 = DLQ.
pub fn tunda_retry(gagal_ke: i32) -> Option<Duration> {
    match gagal_ke {
        1 => Some(Duration::seconds(5)),
        2 => Some(Duration::seconds(30)),
        3 => Some(Duration::seconds(300)),
        _ => None,
    }
}

#[derive(Clone)]
pub struct Tujuan {
    pub base_url: String,
    pub service_key: Rahasia,
}

impl Tujuan {
    /// Tersedia hanya bila `TITIPO_BASE_URL` dan `PEDAREE_SERVICE_KEY` keduanya diatur.
    pub fn dari_config(config: &Config) -> Option<Self> {
        Some(Self {
            base_url: config
                .titipo_base_url
                .clone()?
                .trim_end_matches('/')
                .to_owned(),
            service_key: config.service_key.clone()?,
        })
    }

    fn url(&self) -> String {
        format!("{}{PATH_PENERIMA}", self.base_url)
    }
}

pub fn klien_http() -> Result<Client, AppError> {
    Client::builder()
        .timeout(DurasiStd::from_secs(10))
        .build()
        .map_err(|e| AppError::internal(format!("klien HTTP: {e}")))
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Ringkasan {
    pub terkirim: usize,
    pub duplikat: usize,
    pub diulang: usize,
    pub dlq: usize,
}

enum Hasil {
    Terkirim(String),
    Sementara(String),
    Permanen(String),
}

async fn kirim_satu(klien: &Client, tujuan: &Tujuan, payload: &Value) -> Hasil {
    let respons = klien
        .post(tujuan.url())
        .header("X-Service-Key", tujuan.service_key.nilai())
        .json(payload)
        .send()
        .await;
    let respons = match respons {
        Ok(r) => r,
        Err(e) => return Hasil::Sementara(format!("jaringan: {e}")),
    };
    let status = respons.status();
    let badan: Value = respons.json().await.unwrap_or(Value::Null);
    match status {
        StatusCode::CREATED => Hasil::Terkirim(
            badan["status"]
                .as_str()
                .unwrap_or("dicatat_sebagai_anjuran")
                .to_owned(),
        ),
        // Duplikat (item_id + jenis + tanggal) bukan galat.
        StatusCode::OK => Hasil::Terkirim(
            badan["status"]
                .as_str()
                .unwrap_or("duplikat_diabaikan")
                .to_owned(),
        ),
        s if s.is_server_error()
            || s == StatusCode::REQUEST_TIMEOUT
            || s == StatusCode::TOO_MANY_REQUESTS =>
        {
            Hasil::Sementara(format!("HTTP {s}"))
        }
        s => Hasil::Permanen(format!("HTTP {s}: {badan}")),
    }
}

#[derive(sqlx::FromRow)]
struct Antre {
    id: String,
    payload: Value,
    percobaan: i32,
}

/// Mengirim sinyal yang jatuh tempo. Aman dijalankan paralel (`SKIP LOCKED`).
pub async fn kirim_antrean(
    db: &PgPool,
    klien: &Client,
    tujuan: &Tujuan,
    sekarang: DateTime<Utc>,
) -> Result<Ringkasan, AppError> {
    let mut ringkasan = Ringkasan::default();
    loop {
        let mut tx = db.begin().await?;
        let batch: Vec<Antre> = sqlx::query_as(
            "SELECT id, payload, percobaan FROM sinyal_stok
              WHERE status = 'ANTRE' AND coba_lagi_pada <= $1
              ORDER BY coba_lagi_pada, id LIMIT 20
              FOR UPDATE SKIP LOCKED",
        )
        .bind(sekarang)
        .fetch_all(&mut *tx)
        .await?;
        if batch.is_empty() {
            return Ok(ringkasan);
        }
        for s in batch {
            match kirim_satu(klien, tujuan, &s.payload).await {
                Hasil::Terkirim(jawaban) => {
                    if jawaban == "duplikat_diabaikan" {
                        ringkasan.duplikat += 1;
                    } else {
                        ringkasan.terkirim += 1;
                    }
                    sqlx::query(
                        "UPDATE sinyal_stok SET status = 'TERKIRIM', dikirim_pada = $2, jawaban = $3,
                                percobaan = percobaan + 1, galat_terakhir = NULL WHERE id = $1",
                    )
                    .bind(&s.id)
                    .bind(sekarang)
                    .bind(&jawaban)
                    .execute(&mut *tx)
                    .await?;
                }
                Hasil::Sementara(galat) => {
                    let gagal_ke = s.percobaan + 1;
                    match tunda_retry(gagal_ke) {
                        Some(jeda) => {
                            ringkasan.diulang += 1;
                            sqlx::query(
                                "UPDATE sinyal_stok SET percobaan = $2, coba_lagi_pada = $3, galat_terakhir = $4
                                  WHERE id = $1",
                            )
                            .bind(&s.id)
                            .bind(gagal_ke)
                            .bind(sekarang + jeda)
                            .bind(&galat)
                            .execute(&mut *tx)
                            .await?;
                        }
                        None => {
                            ringkasan.dlq += 1;
                            tandai_dlq(&mut tx, &s.id, gagal_ke, &galat).await?;
                        }
                    }
                }
                Hasil::Permanen(galat) => {
                    ringkasan.dlq += 1;
                    tandai_dlq(&mut tx, &s.id, s.percobaan + 1, &galat).await?;
                }
            }
        }
        tx.commit().await?;
    }
}

async fn tandai_dlq(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: &str,
    percobaan: i32,
    galat: &str,
) -> Result<(), AppError> {
    tracing::error!(sinyal = %id, galat = %galat, "sinyal pedaree.stok.v1 masuk DLQ");
    sqlx::query(
        "UPDATE sinyal_stok SET status = 'DLQ', percobaan = $2, galat_terakhir = $3 WHERE id = $1",
    )
    .bind(id)
    .bind(percobaan)
    .bind(galat)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Latar belakang: kirim antrean tiap 5 detik.
pub async fn jalankan_pengirim(db: PgPool, tujuan: Tujuan) {
    let klien = match klien_http() {
        Ok(k) => k,
        Err(e) => {
            tracing::error!(galat = %e, "pengirim sinyal tidak dapat dimulai");
            return;
        }
    };
    let mut detak = tokio::time::interval(DurasiStd::from_secs(5));
    loop {
        detak.tick().await;
        match kirim_antrean(&db, &klien, &tujuan, Utc::now()).await {
            Ok(r) if r != Ringkasan::default() => tracing::info!(?r, "antrean sinyal diproses"),
            Ok(_) => {}
            Err(e) => tracing::warn!(galat = %e, "gagal memproses antrean sinyal"),
        }
    }
}

/// Latar belakang: `expiry-sweep` tiap hari 06:00 WIB; menyusul sekali saat start bila terlewat.
pub async fn jalankan_expiry_sweep(db: PgPool) {
    use crate::sinyal::{detik_sampai_sweep, jalankan_sweep_harian, sudah_lewat_jam_sweep};
    if sudah_lewat_jam_sweep(Utc::now()) {
        lakukan_sweep(&db).await;
    }
    loop {
        tokio::time::sleep(DurasiStd::from_secs(detik_sampai_sweep(Utc::now()))).await;
        lakukan_sweep(&db).await;
    }

    async fn lakukan_sweep(db: &PgPool) {
        match jalankan_sweep_harian(db, Utc::now()).await {
            Ok(Some(n)) => tracing::info!(sinyal = n, "expiry-sweep selesai"),
            Ok(None) => tracing::info!("expiry-sweep hari ini sudah dijalankan"),
            Err(e) => tracing::error!(galat = %e, "expiry-sweep gagal"),
        }
    }
}

#[cfg(test)]
mod uji {
    use super::*;

    #[test]
    fn retry_3x_5_30_300_detik_lalu_dlq() {
        assert_eq!(tunda_retry(1), Some(Duration::seconds(5)));
        assert_eq!(tunda_retry(2), Some(Duration::seconds(30)));
        assert_eq!(tunda_retry(3), Some(Duration::seconds(300)));
        assert_eq!(tunda_retry(4), None);
    }
}
