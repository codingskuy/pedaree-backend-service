//! Uji auth (PDR-BE-003): JWT akses 15 menit audiens `pedaree`, refresh 7 hari dengan
//! reuse-detection, penolakan token audiens lain, argon2.

use axum::http::Method;
use axum::routing::get;
use axum::{Json, Router};
use chrono::Utc;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use pedaree_backend_service::auth::{buat_pengguna, Pengguna};
use pedaree_backend_service::rute;
use pedaree_backend_service::state::AppState;
use serde_json::{json, Value};
use sqlx::PgPool;

mod dukung;
use dukung::{argon_murah, kirim, post, HH, ISSUER};

async fn siapkan(db: &PgPool) -> (Router, AppState) {
    buat_pengguna(db, HH, "budi", "rahasia-banget-123", &argon_murah())
        .await
        .unwrap();
    let state = dukung::state(db.clone());
    // Rute uji untuk ekstraktor `Pengguna` (kontrak v1.0.0 belum punya endpoint "saya").
    let app = rute(state.clone()).merge(
        Router::new()
            .route(
                "/_saya",
                get(|p: Pengguna| async move { Json(json!({"id": p.id, "hh": p.household_id})) }),
            )
            .with_state(state.clone()),
    );
    (app, state)
}

async fn login(app: &Router) -> Value {
    let (s, j) = post(
        app,
        "/auth/login",
        json!({"idPengguna": "budi", "kataSandi": "rahasia-banget-123", "idPerangkat": "dev_1"}),
    )
    .await;
    assert_eq!(s, 200, "{j}");
    j
}

#[derive(serde::Deserialize, Debug)]
struct KlaimUji {
    iss: String,
    sub: String,
    aud: String,
    hh: String,
    exp: i64,
    iat: i64,
}

fn klaim_dari(token: &str) -> KlaimUji {
    let mut v = Validation::new(Algorithm::EdDSA);
    v.set_audience(&["pedaree"]);
    v.set_issuer(&[ISSUER]);
    let kunci = DecodingKey::from_ed_pem(dukung::kunci_uji().publik.as_bytes()).unwrap();
    jsonwebtoken::decode::<KlaimUji>(token, &kunci, &v)
        .unwrap()
        .claims
}

fn token_buatan(aud: &str, iss: &str, exp_offset: i64, kunci_pem: &str, alg: Algorithm) -> String {
    let sekarang = Utc::now().timestamp();
    let klaim = json!({
        "iss": iss, "sub": "usr_X", "aud": aud, "hh": HH,
        "exp": sekarang + exp_offset, "iat": sekarang, "jti": "j1"
    });
    let kunci = match alg {
        Algorithm::EdDSA => EncodingKey::from_ed_pem(kunci_pem.as_bytes()).unwrap(),
        _ => EncodingKey::from_secret(b"rahasia-hmac"),
    };
    jsonwebtoken::encode(&Header::new(alg), &klaim, &kunci).unwrap()
}

#[sqlx::test]
async fn login_menerbitkan_jwt_15_menit_audiens_pedaree_dan_refresh(db: PgPool) {
    let (app, _) = siapkan(&db).await;
    let j = login(&app).await;
    assert_eq!(j["audiens"], "pedaree");
    assert_eq!(j["kedaluwarsaDetik"], 900);
    let k = klaim_dari(j["tokenAkses"].as_str().unwrap());
    assert_eq!(k.aud, "pedaree");
    assert_eq!(k.iss, ISSUER);
    assert_eq!(k.hh, HH);
    assert!(k.sub.starts_with("usr_"));
    assert_eq!(k.exp - k.iat, 900, "umur akses tepat 15 menit");
    assert!(j["tokenRefresh"].as_str().unwrap().starts_with("rt_"));

    // refresh berumur 7 hari dan hanya hash-nya yang disimpan
    let (sisa_detik, hash): (f64, String) = sqlx::query_as(
        "SELECT EXTRACT(EPOCH FROM (kedaluwarsa_pada - dibuat_pada))::float8, token_hash FROM refresh_token",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert!((sisa_detik - 7.0 * 86400.0).abs() < 5.0);
    assert_ne!(hash, j["tokenRefresh"].as_str().unwrap());
}

#[sqlx::test]
async fn kata_sandi_disimpan_sebagai_argon2id(db: PgPool) {
    let _ = siapkan(&db).await;
    let hash: String = sqlx::query_scalar("SELECT kata_sandi_hash FROM pengguna")
        .fetch_one(&db)
        .await
        .unwrap();
    assert!(hash.starts_with("$argon2id$"), "{hash}");
    assert!(!hash.contains("rahasia-banget"));
}

#[sqlx::test]
async fn login_gagal_menjawab_401_seragam_untuk_sandi_salah_dan_akun_tak_ada(db: PgPool) {
    let (app, _) = siapkan(&db).await;
    let (s1, j1) = post(
        &app,
        "/auth/login",
        json!({"idPengguna": "budi", "kataSandi": "salah"}),
    )
    .await;
    let (s2, j2) = post(
        &app,
        "/auth/login",
        json!({"idPengguna": "hantu", "kataSandi": "apa-saja"}),
    )
    .await;
    assert_eq!((s1, s2), (401, 401));
    assert_eq!(j1, j2, "badan galat tidak membedakan akun ada/tidak");
    assert_eq!(j1["kode"], "TIDAK_SAH");
}

#[sqlx::test]
async fn login_nama_tidak_peka_huruf_dan_validasi_400(db: PgPool) {
    let (app, _) = siapkan(&db).await;
    let (s, _) = post(
        &app,
        "/auth/login",
        json!({"idPengguna": " BUDI ", "kataSandi": "rahasia-banget-123"}),
    )
    .await;
    assert_eq!(s, 200);
    let (s, j) = post(
        &app,
        "/auth/login",
        json!({"idPengguna": "", "kataSandi": "x"}),
    )
    .await;
    assert_eq!(s, 400);
    assert_eq!(j["field"], "idPengguna");
    let (s, j) = post(&app, "/auth/login", json!({"idPengguna": "budi"})).await;
    assert_eq!(s, 400, "field wajib hilang = 400 berbentuk AppError");
    assert_eq!(j["kode"], "VALIDASI_GAGAL");
}

#[sqlx::test]
async fn pengguna_nonaktif_tidak_bisa_login(db: PgPool) {
    let (app, _) = siapkan(&db).await;
    sqlx::query("UPDATE pengguna SET aktif = FALSE")
        .execute(&db)
        .await
        .unwrap();
    let (s, _) = post(
        &app,
        "/auth/login",
        json!({"idPengguna": "budi", "kataSandi": "rahasia-banget-123"}),
    )
    .await;
    assert_eq!(s, 401);
}

#[sqlx::test]
async fn refresh_merotasi_token_dalam_keluarga_yang_sama(db: PgPool) {
    let (app, _) = siapkan(&db).await;
    let a = login(&app).await;
    let (s, b) = post(
        &app,
        "/auth/refresh",
        json!({"tokenRefresh": a["tokenRefresh"]}),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_ne!(a["tokenRefresh"], b["tokenRefresh"]);
    assert_eq!(b["kedaluwarsaDetik"], 900);
    klaim_dari(b["tokenAkses"].as_str().unwrap());
    let keluarga: i64 = sqlx::query_scalar("SELECT count(DISTINCT keluarga_id) FROM refresh_token")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(keluarga, 1);
}

#[sqlx::test]
async fn refresh_dipakai_ulang_mencabut_seluruh_keluarga(db: PgPool) {
    let (app, _) = siapkan(&db).await;
    let a = login(&app).await;
    let (_, b) = post(
        &app,
        "/auth/refresh",
        json!({"tokenRefresh": a["tokenRefresh"]}),
    )
    .await;
    let (_, c) = post(
        &app,
        "/auth/refresh",
        json!({"tokenRefresh": b["tokenRefresh"]}),
    )
    .await;

    // Penyerang memutar ulang token lama A -> terdeteksi.
    let (s, j) = post(
        &app,
        "/auth/refresh",
        json!({"tokenRefresh": a["tokenRefresh"]}),
    )
    .await;
    assert_eq!(s, 401);
    assert_eq!(j["kode"], "REFRESH_DIPAKAI_ULANG");

    // Token terbaru C (milik pengguna sah) ikut tidak berlaku: wajib login ulang.
    let (s, _) = post(
        &app,
        "/auth/refresh",
        json!({"tokenRefresh": c["tokenRefresh"]}),
    )
    .await;
    assert_eq!(s, 401);
    let aktif: i64 =
        sqlx::query_scalar("SELECT count(*) FROM refresh_token WHERE dicabut_pada IS NULL")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(aktif, 0);

    // Login ulang berhasil dan menghasilkan keluarga baru yang berfungsi.
    let baru = login(&app).await;
    let (s, _) = post(
        &app,
        "/auth/refresh",
        json!({"tokenRefresh": baru["tokenRefresh"]}),
    )
    .await;
    assert_eq!(s, 200);
}

#[sqlx::test]
async fn pemakaian_serentak_satu_refresh_hanya_satu_yang_menang(db: PgPool) {
    let (app, _) = siapkan(&db).await;
    let a = login(&app).await;
    let (r1, r2) = tokio::join!(
        post(
            &app,
            "/auth/refresh",
            json!({"tokenRefresh": a["tokenRefresh"]})
        ),
        post(
            &app,
            "/auth/refresh",
            json!({"tokenRefresh": a["tokenRefresh"]})
        ),
    );
    let mut kode = [r1.0, r2.0];
    kode.sort();
    assert_eq!(kode, [200, 401]);
}

#[sqlx::test]
async fn refresh_kedaluwarsa_dan_tak_dikenal_ditolak(db: PgPool) {
    let (app, _) = siapkan(&db).await;
    let a = login(&app).await;
    sqlx::query("UPDATE refresh_token SET kedaluwarsa_pada = now() - interval '1 second'")
        .execute(&db)
        .await
        .unwrap();
    let (s, j) = post(
        &app,
        "/auth/refresh",
        json!({"tokenRefresh": a["tokenRefresh"]}),
    )
    .await;
    assert_eq!((s, j["kode"].as_str()), (401, Some("REFRESH_KEDALUWARSA")));
    let (s, _) = post(&app, "/auth/refresh", json!({"tokenRefresh": "rt_ngawur"})).await;
    assert_eq!(s, 401);
    let (s, j) = post(&app, "/auth/refresh", json!({})).await;
    assert_eq!((s, j["kode"].as_str()), (400, Some("VALIDASI_GAGAL")));
}

fn bearer(token: &str) -> [(&'static str, String); 2] {
    [
        ("authorization", format!("Bearer {token}")),
        ("x-household-id", HH.to_owned()),
    ]
}

async fn saya(app: &Router, header: &[(&str, &str)]) -> (u16, Value) {
    let (s, j) = kirim(app, Method::GET, "/_saya", header, None).await;
    (s, j)
}

#[sqlx::test]
async fn token_valid_dan_household_cocok_diterima(db: PgPool) {
    let (app, _) = siapkan(&db).await;
    let a = login(&app).await;
    let [(k1, v1), (k2, v2)] = bearer(a["tokenAkses"].as_str().unwrap());
    let (s, j) = saya(&app, &[(k1, &v1), (k2, &v2)]).await;
    assert_eq!(s, 200, "{j}");
    assert_eq!(j["hh"], HH);
}

#[sqlx::test]
async fn token_audiens_lain_issuer_lain_kedaluwarsa_dan_alg_lain_ditolak_401(db: PgPool) {
    let (app, _) = siapkan(&db).await;
    let pem = dukung::kunci_uji().privat.as_str();
    let kasus = [
        (
            "audiens titipo",
            token_buatan("titipo", ISSUER, 600, pem, Algorithm::EdDSA),
        ),
        (
            "audiens titeny",
            token_buatan("titeny", ISSUER, 600, pem, Algorithm::EdDSA),
        ),
        (
            "issuer lain",
            token_buatan("pedaree", "layanan-lain", 600, pem, Algorithm::EdDSA),
        ),
        (
            "kedaluwarsa",
            token_buatan("pedaree", ISSUER, -3600, pem, Algorithm::EdDSA),
        ),
        (
            "kunci lain",
            token_buatan(
                "pedaree",
                ISSUER,
                600,
                &dukung::buat_kunci().privat,
                Algorithm::EdDSA,
            ),
        ),
        (
            "HS256",
            token_buatan("pedaree", ISSUER, 600, pem, Algorithm::HS256),
        ),
    ];
    for (nama, token) in kasus {
        let [(k1, v1), (k2, v2)] = bearer(&token);
        let (s, j) = saya(&app, &[(k1, &v1), (k2, &v2)]).await;
        assert_eq!(s, 401, "{nama}: {j}");
        assert_eq!(j["kode"], "TIDAK_SAH", "{nama}");
    }
    // tanpa header Authorization / skema salah
    assert_eq!(saya(&app, &[("x-household-id", HH)]).await.0, 401);
    assert_eq!(
        saya(
            &app,
            &[("authorization", "Basic abc"), ("x-household-id", HH)]
        )
        .await
        .0,
        401
    );
}

#[sqlx::test]
async fn household_header_wajib_dan_harus_sama_dengan_token(db: PgPool) {
    let (app, _) = siapkan(&db).await;
    let a = login(&app).await;
    let auth = format!("Bearer {}", a["tokenAkses"].as_str().unwrap());
    let (s, j) = saya(&app, &[("authorization", &auth)]).await;
    assert_eq!((s, j["field"].as_str()), (400, Some("X-Household-Id")));
    let (s, j) = saya(
        &app,
        &[
            ("authorization", &auth),
            ("x-household-id", "hh_ORANG_LAIN"),
        ],
    )
    .await;
    assert_eq!(
        (s, j["kode"].as_str()),
        (401, Some("HOUSEHOLD_TIDAK_COCOK"))
    );
}
