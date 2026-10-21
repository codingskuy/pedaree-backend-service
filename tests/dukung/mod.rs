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
