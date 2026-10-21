//! Uji penangani provider state Pact (PDR-BE-TEST-PACT). Hanya dengan `--features pact-state`.
//! Ini menguji state handler, BUKAN verifikasi kontrak konsumen (belum ada pact konsumen nyata).
#![cfg(feature = "pact-state")]

use serde_json::{json, Value};
use sqlx::PgPool;

mod dukung;
use dukung::{kirim, post, state, HH};

#[sqlx::test]
async fn state_menyiapkan_akun_item_dan_mengosongkan_dapur(db: PgPool) {
    let app = pedaree_backend_service::rute(state(db.clone()));
    let st = |badan: Value| {
        let app = app.clone();
        async move { post(&app, "/_pact/state", badan).await }
    };

    let (s, j) = st(json!({"state": "pengguna terdaftar", "params": {"idPengguna": "pact", "kataSandi": "rahasia-pact-12345"}})).await;
    assert_eq!(s, 200, "{j}");
    // idempoten: panggilan kedua tidak gagal
    let (s, _) = st(json!({"state": "pengguna terdaftar", "params": {"idPengguna": "pact", "kataSandi": "rahasia-pact-12345"}})).await;
    assert_eq!(s, 200);
    let (s, j) = post(
        &app,
        "/auth/login",
        json!({"idPengguna": "pact", "kataSandi": "rahasia-pact-12345"}),
    )
    .await;
    assert_eq!(s, 200, "{j}");
    let token = format!("Bearer {}", j["tokenAkses"].as_str().unwrap());
    let daftar = || {
        let app = app.clone();
        let token = token.clone();
        async move {
            kirim(
                &app,
                axum::http::Method::GET,
                "/pantry/items",
                &[("authorization", token.as_str()), ("x-household-id", HH)],
                None,
            )
            .await
        }
    };

    let (s, j) = st(json!({"state": "item dengan stok", "params": {"nama": "Beras", "jumlahGram": 1500, "stokMinimumGram": 500}})).await;
    assert_eq!(s, 200, "{j}");
    assert!(j["pantryItemId"].as_str().unwrap().starts_with("itm_"));
    let (s, j) = daftar().await;
    assert_eq!(s, 200, "{j}");
    assert_eq!(j["data"][0]["nama"], "Beras");
    assert_eq!(j["data"][0]["stokTampil"], 1500.0);
    assert_eq!(j["data"][0]["statusStok"], "AMAN");

    let (s, _) = st(json!({"state": "dapur kosong"})).await;
    assert_eq!(s, 200);
    let (_, j) = daftar().await;
    assert!(j["data"].as_array().unwrap().is_empty());

    let (s, j) = st(json!({"state": "tidak ada"})).await;
    assert_eq!((s, j["field"].as_str()), (400, Some("state")));
    let (s, _) = st(json!({"state": "apa saja", "action": "teardown"})).await;
    assert_eq!(s, 200);
}
