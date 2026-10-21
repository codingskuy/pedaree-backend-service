//! Uji CLI pembuat akun (ADR-0011) dan jaminan tidak ada pendaftaran publik.

use axum::http::Method;
use pedaree_backend_service::cli::{buat_akun, PermintaanAkun};
use pedaree_backend_service::rute;
use serde_json::json;
use sqlx::PgPool;

mod dukung;
use dukung::{argon_murah, kirim, post};

fn minta(id: &str, hh: Option<&str>) -> PermintaanAkun {
    PermintaanAkun {
        id_pengguna: id.to_owned(),
        household_id: hh.map(str::to_owned),
    }
}

#[sqlx::test]
async fn akun_baru_membuat_household_dan_bisa_login(db: PgPool) {
    let a = buat_akun(
        &db,
        &minta("budi", None),
        "rahasia-panjang-1",
        &argon_murah(),
    )
    .await
    .unwrap();
    assert!(a.household_baru);
    assert!(a.household_id.starts_with("hh_"));
    assert!(a.pengguna_id.starts_with("usr_"));

    let app = rute(dukung::state(db));
    let (s, j) = post(
        &app,
        "/auth/login",
        json!({"idPengguna": "budi", "kataSandi": "rahasia-panjang-1", "idPerangkat": "d1"}),
    )
    .await;
    assert_eq!(s, 200, "{j}");
}

#[sqlx::test]
async fn household_yang_ada_dipakai_ulang_untuk_impor(db: PgPool) {
    let a = buat_akun(
        &db,
        &minta("ibu", Some("hh_PAWONEE01")),
        "rahasia-panjang-1",
        &argon_murah(),
    )
    .await
    .unwrap();
    let b = buat_akun(
        &db,
        &minta("ayah", Some("hh_PAWONEE01")),
        "rahasia-panjang-2",
        &argon_murah(),
    )
    .await
    .unwrap();
    assert_eq!(a.household_id, "hh_PAWONEE01");
    assert_eq!(b.household_id, "hh_PAWONEE01");
    assert!(!b.household_baru);
}

#[sqlx::test]
async fn masukan_tidak_valid_dan_duplikat_ditolak(db: PgPool) {
    let argon = argon_murah();
    assert!(buat_akun(&db, &minta("budi", None), "pendek", &argon)
        .await
        .is_err());
    assert!(
        buat_akun(&db, &minta("ab", None), "rahasia-panjang-1", &argon)
            .await
            .is_err()
    );
    assert!(buat_akun(
        &db,
        &minta("budi", Some("usr_x")),
        "rahasia-panjang-1",
        &argon
    )
    .await
    .is_err());
    buat_akun(&db, &minta("budi", None), "rahasia-panjang-1", &argon)
        .await
        .unwrap();
    // id pengguna unik tanpa membedakan huruf besar/kecil
    assert!(
        buat_akun(&db, &minta("BUDI", None), "rahasia-panjang-1", &argon)
            .await
            .is_err()
    );
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM pengguna")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[sqlx::test]
async fn tidak_ada_endpoint_pendaftaran_publik(db: PgPool) {
    let app = rute(dukung::state(db.clone()));
    let badan = json!({"idPengguna": "intruder", "kataSandi": "rahasia-panjang-1"});
    for path in [
        "/auth/daftar",
        "/auth/register",
        "/auth/signup",
        "/daftar",
        "/households",
        "/pengguna",
        "/users",
    ] {
        for metode in [Method::POST, Method::PUT] {
            let (s, _) = kirim(&app, metode, path, &[], Some(badan.clone())).await;
            assert!(s == 404 || s == 405, "{path} menjawab {s}");
        }
    }
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM pengguna")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(n, 0);
}
