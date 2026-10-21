//! Penangani provider state untuk verifikasi Pact (PDR-BE-TEST-PACT). Hanya dikompilasi dengan
//! fitur `pact-state`; TIDAK BOLEH aktif di rilis (mengubah/menghapus data tanpa autentikasi).
//!
//! `pact_verifier_cli --state-change-url http://127.0.0.1:PORT/_pact/state` memanggil
//! `POST /_pact/state` dengan `{"state": "...", "params": {...}, "action": "setup"|"teardown"}`.
//! State yang dikenal (nama dipilih konsumen di pact-nya; tambahkan state baru di sini):
//!
//! | state | params | efek |
//! |---|---|---|
//! | `pengguna terdaftar` | `idPengguna`, `kataSandi`, `household` | membuat akun (idempoten) |
//! | `item dengan stok` | `household`, `nama`, `jumlahGram`, `stokMinimumGram` | item + satu batch + mutasi MASUK |
//! | `dapur kosong` | `household` | menghapus data stok dan sinyal household |

use argon2::Argon2;
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::buat_pengguna;
use crate::error::{AppError, JsonBaku};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
struct Permintaan {
    state: String,
    #[serde(default)]
    params: Value,
    #[serde(default)]
    action: Option<String>,
}

fn teks<'a>(params: &'a Value, kunci: &str, bawaan: &'a str) -> &'a str {
    params.get(kunci).and_then(Value::as_str).unwrap_or(bawaan)
}

fn angka(params: &Value, kunci: &str, bawaan: i64) -> i64 {
    params.get(kunci).and_then(Value::as_i64).unwrap_or(bawaan)
}

async fn ubah_state(
    State(state): State<AppState>,
    JsonBaku(p): JsonBaku<Permintaan>,
) -> Result<Json<Value>, AppError> {
    if p.action.as_deref() == Some("teardown") {
        return Ok(Json(json!({})));
    }
    let hh = teks(&p.params, "household", "hh_01J9Z8QK2M4B").to_owned();
    match p.state.as_str() {
        "pengguna terdaftar" => {
            let id = teks(&p.params, "idPengguna", "budi");
            let sandi = teks(&p.params, "kataSandi", "rahasia-banget-123");
            match buat_pengguna(&state.db, &hh, id, sandi, &Argon2::default()).await {
                Ok(_) | Err(AppError::Konflik(_)) => {}
                Err(e) => return Err(e),
            }
            Ok(Json(json!({ "household": hh })))
        }
        "item dengan stok" => {
            let nama = teks(&p.params, "nama", "Beras");
            let jumlah_x10 = angka(&p.params, "jumlahGram", 1000) * 10;
            let minimum_x10 = angka(&p.params, "stokMinimumGram", 0) * 10;
            let item = format!("itm_{}", ulid::Ulid::new());
            let batch = format!("btc_{}", ulid::Ulid::new());
            let mut tx = state.db.begin().await?;
            sqlx::query(
                "INSERT INTO pantry_items (id, household_id, nama, kategori, satuan_simpan, satuan_tampil,
                                           stok_minimum_x10, umur_simpan_hari_default, lokasi_default)
                 VALUES ($1, $2, $3, 'KERING', 'gram', 'gram', $4, 30, 'SUHU_RUANG')",
            )
            .bind(&item)
            .bind(&hh)
            .bind(nama)
            .bind(minimum_x10)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO stok_batch (id, household_id, pantry_item_id, jumlah_awal_x10, jumlah_aktif_x10,
                                         tanggal_masuk, lokasi, sumber)
                 VALUES ($1, $2, $3, $4, $4, CURRENT_DATE, 'SUHU_RUANG', 'MASUK_BELANJA')",
            )
            .bind(&batch)
            .bind(&hh)
            .bind(&item)
            .bind(jumlah_x10)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO mutasi_stok (id, household_id, batch_id, jenis, jumlah_x10, sisa_setelah_x10, alasan, aktor)
                 VALUES ($1, $2, $3, 'MASUK', $4, $4, 'state pact', 'pact')",
            )
            .bind(format!("mut_{}", ulid::Ulid::new()))
            .bind(&hh)
            .bind(&batch)
            .bind(jumlah_x10)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok(Json(json!({ "pantryItemId": item, "batchId": batch })))
        }
        "dapur kosong" => {
            let mut tx = state.db.begin().await?;
            for sql in [
                "DELETE FROM sinyal_stok WHERE household_id = $1",
                "DELETE FROM mutasi_stok WHERE household_id = $1",
                "DELETE FROM stok_batch WHERE household_id = $1",
                "DELETE FROM pantry_items WHERE household_id = $1",
            ] {
                sqlx::query(sql).bind(&hh).execute(&mut *tx).await?;
            }
            tx.commit().await?;
            Ok(Json(json!({})))
        }
        lain => Err(AppError::validasi(
            format!("provider state tidak dikenal: {lain}"),
            Some("state"),
        )),
    }
}

pub fn rute() -> Router<AppState> {
    Router::new().route("/_pact/state", post(ubah_state))
}
