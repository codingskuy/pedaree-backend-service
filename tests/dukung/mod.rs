//! Pendukung uji integrasi: kunci uji dibangkitkan saat runtime (tidak ada kunci di repo).
#![allow(dead_code)]

use std::sync::OnceLock;

use argon2::{Algorithm, Argon2, Params, Version};
use axum::body::Body;
use axum::http::{Method, Request};
use axum::Router;
use ed25519_dalek::pkcs8::spki::der::pem::LineEnding;
use ed25519_dalek::pkcs8::{EncodePrivateKey, EncodePublicKey};
use ed25519_dalek::SigningKey;
use http_body_util::BodyExt;
use pedaree_backend_service::config::{Config, Rahasia};
use pedaree_backend_service::state::AppState;
use rand::rngs::OsRng;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

pub const HH: &str = "hh_01J9Z8QK2M4B";
pub const ISSUER: &str = "pedaree-uji";

pub struct PasanganKunci {
    pub privat: String,
    pub publik: String,
}

pub fn kunci_uji() -> &'static PasanganKunci {
    static KUNCI: OnceLock<PasanganKunci> = OnceLock::new();
    KUNCI.get_or_init(buat_kunci)
}

pub fn buat_kunci() -> PasanganKunci {
    let kunci = SigningKey::generate(&mut OsRng);
    PasanganKunci {
        privat: kunci.to_pkcs8_pem(LineEnding::LF).unwrap().to_string(),
        publik: kunci
            .verifying_key()
            .to_public_key_pem(LineEnding::LF)
            .unwrap(),
    }
}

pub fn config_uji() -> Config {
    let k = kunci_uji();
    Config {
        database_url: "postgres://tidak-dipakai".to_owned(),
        bind_addr: "127.0.0.1:0".to_owned(),
        jwt_private_pem: Rahasia::baru(k.privat.clone()),
        jwt_public_pem: k.publik.clone(),
        jwt_issuer: ISSUER.to_owned(),
        service_key: Some(Rahasia::baru("kunci-layanan-uji")),
        titipo_base_url: None,
    }
}

pub fn state(db: PgPool) -> AppState {
    AppState::baru(db, config_uji()).unwrap()
}

/// Argon2id berbiaya rendah agar uji cepat (verifikasi membaca parameter dari hash PHC).
pub fn argon_murah() -> Argon2<'static> {
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(8, 1, 1, None).unwrap(),
    )
}

pub async fn kirim(
    app: &Router,
    metode: Method,
    path: &str,
    header: &[(&str, &str)],
    badan: Option<Value>,
) -> (u16, Value) {
    let mut req = Request::builder().method(metode).uri(path);
    for (k, v) in header {
        req = req.header(*k, *v);
    }
    let body = match badan {
        Some(j) => {
            req = req.header("content-type", "application/json");
            Body::from(j.to_string())
        }
        None => Body::empty(),
    };
    let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = res.status().as_u16();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

pub async fn post(app: &Router, path: &str, badan: Value) -> (u16, Value) {
    kirim(app, Method::POST, path, &[], Some(badan)).await
}

// ---------------------------------------------------------------------------
// Klien uji terautentikasi dengan jam tetap (hari ini WIB = 2026-10-21).

use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use pedaree_backend_service::auth::buat_pengguna;
use pedaree_backend_service::rute;
use pedaree_backend_service::waktu::Jam;
use serde_json::json;

pub const HARI_INI: &str = "2026-10-21";

pub fn jam_tetap() -> Jam {
    let t: DateTime<Utc> = Utc.with_ymd_and_hms(2026, 10, 21, 5, 0, 0).unwrap();
    Arc::new(move || t)
}

/// Jam uji yang bisa dimajukan (untuk dedup 24 jam, retry, dsb).
#[derive(Clone)]
pub struct JamUji(Arc<std::sync::Mutex<DateTime<Utc>>>);

impl JamUji {
    pub fn baru() -> Self {
        Self(Arc::new(std::sync::Mutex::new(
            Utc.with_ymd_and_hms(2026, 10, 21, 5, 0, 0).unwrap(),
        )))
    }

    pub fn jam(&self) -> Jam {
        let dalam = self.0.clone();
        Arc::new(move || *dalam.lock().unwrap())
    }

    pub fn sekarang(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }

    pub fn majukan(&self, durasi: chrono::Duration) {
        let mut t = self.0.lock().unwrap();
        *t += durasi;
    }
}

pub struct Klien {
    pub app: Router,
    pub db: PgPool,
    pub token: String,
    pub household: String,
}

impl Klien {
    pub async fn baru(db: &PgPool) -> Self {
        Self::untuk(db, HH, "budi").await
    }

    pub async fn dengan_jam(db: &PgPool, jam: Jam) -> Self {
        Self::untuk_jam(db, HH, "budi", jam).await
    }

    pub async fn untuk(db: &PgPool, household: &str, pengguna: &str) -> Self {
        Self::untuk_jam(db, household, pengguna, jam_tetap()).await
    }

    pub async fn untuk_jam(db: &PgPool, household: &str, pengguna: &str, jam: Jam) -> Self {
        buat_pengguna(
            db,
            household,
            pengguna,
            "rahasia-banget-123",
            &argon_murah(),
        )
        .await
        .unwrap();
        let state = state(db.clone()).dengan_jam(jam);
        let app = rute(state);
        let (s, j) = post(
            &app,
            "/auth/login",
            json!({"idPengguna": pengguna, "kataSandi": "rahasia-banget-123"}),
        )
        .await;
        assert_eq!(s, 200, "{j}");
        Self {
            app,
            db: db.clone(),
            token: j["tokenAkses"].as_str().unwrap().to_owned(),
            household: household.to_owned(),
        }
    }

    fn header(&self, rid: Option<&str>) -> Vec<(String, String)> {
        let mut h = vec![
            ("authorization".to_owned(), format!("Bearer {}", self.token)),
            ("x-household-id".to_owned(), self.household.clone()),
        ];
        if let Some(r) = rid {
            h.push(("x-request-id".to_owned(), r.to_owned()));
        }
        h
    }

    pub async fn kirim(
        &self,
        metode: Method,
        path: &str,
        rid: Option<&str>,
        badan: Option<Value>,
    ) -> (u16, Value) {
        let h = self.header(rid);
        let h: Vec<(&str, &str)> = h.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        kirim(&self.app, metode, path, &h, badan).await
    }

    pub async fn get(&self, path: &str) -> (u16, Value) {
        self.kirim(Method::GET, path, None, None).await
    }

    /// POST dengan X-Request-Id baru.
    pub async fn post(&self, path: &str, badan: Value) -> (u16, Value) {
        let rid = uuid::Uuid::new_v4().to_string();
        self.kirim(Method::POST, path, Some(&rid), Some(badan))
            .await
    }

    pub async fn post_rid(&self, path: &str, rid: &str, badan: Value) -> (u16, Value) {
        self.kirim(Method::POST, path, Some(rid), Some(badan)).await
    }

    /// Membuat item lewat API dan mengembalikan id-nya.
    pub async fn item(&self, nama: &str, satuan: &str, minimum: f64, umur: i64) -> String {
        let (s, j) = self
            .post(
                "/pantry/items",
                json!({"nama": nama, "kategori": "KERING", "satuanSimpan": satuan, "satuanTampil": satuan,
                       "stokMinimum": minimum, "umurSimpanHariDefault": umur, "lokasiDefault": "SUHU_RUANG"}),
            )
            .await;
        assert_eq!(s, 201, "{j}");
        j["id"].as_str().unwrap().to_owned()
    }

    /// Mencatat masuk lewat API; `masuk` dan `kdl` berupa YYYY-MM-DD.
    pub async fn masuk(
        &self,
        item: &str,
        jumlah: f64,
        satuan: &str,
        masuk: &str,
        kdl: Option<&str>,
    ) -> String {
        let (s, j) = self
            .post(
                "/pantry/batches",
                json!({"pantryItemId": item, "jumlah": jumlah, "satuan": satuan, "tanggalMasuk": masuk,
                       "tanggalKedaluwarsa": kdl, "lokasi": "SUHU_RUANG", "sumber": "MASUK_BELANJA"}),
            )
            .await;
        assert_eq!(s, 201, "{j}");
        j["id"].as_str().unwrap().to_owned()
    }

    pub async fn aktif_x10(&self, batch: &str) -> i64 {
        sqlx::query_scalar("SELECT jumlah_aktif_x10 FROM stok_batch WHERE id = $1")
            .bind(batch)
            .fetch_one(&self.db)
            .await
            .unwrap()
    }

    pub async fn jumlah_mutasi(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM mutasi_stok")
            .fetch_one(&self.db)
            .await
            .unwrap()
    }
}
