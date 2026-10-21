//! Uji skema FEFO (PDR-BE-002): konstrain, batas per rumah tangga, migrasi balik.
//! Memerlukan `DATABASE_URL` ke Postgres yang boleh membuat basis data uji.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use pedaree_backend_service::db::MIGRATOR;
use pedaree_backend_service::rute;
use pedaree_backend_service::state::AppState;
use sqlx::PgPool;
use tower::ServiceExt;

const HH: &str = "hh_01J9Z8QK2M4B";

async fn buat_item(db: &PgPool, id: &str, nama: &str, hh: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO pantry_items (id, household_id, nama, kategori, satuan_simpan, satuan_tampil,
            stok_minimum_x10, umur_simpan_hari_default, lokasi_default)
         VALUES ($1, $2, $3, 'BUMBU', 'gram', 'gram', 2500, 21, 'BASKOM_BAWANG')",
    )
    .bind(id)
    .bind(hh)
    .bind(nama)
    .execute(db)
    .await
    .map(|_| ())
}

async fn buat_batch(db: &PgPool, id: &str, item: &str, aktif: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO stok_batch (id, household_id, pantry_item_id, jumlah_awal_x10, jumlah_aktif_x10,
            tanggal_masuk, tanggal_kedaluwarsa, lokasi, sumber)
         VALUES ($1, $2, $3, 10000, $4, '2026-09-28', '2026-10-19', 'BASKOM_BAWANG', 'MASUK_BELANJA')",
    )
    .bind(id)
    .bind(HH)
    .bind(item)
    .bind(aktif)
    .execute(db)
    .await
    .map(|_| ())
}

fn kode_pg(e: &sqlx::Error) -> Option<String> {
    match e {
        sqlx::Error::Database(d) => d.code().map(|c| c.to_string()),
        _ => None,
    }
}

#[sqlx::test]
async fn kesehatan_menjawab_ok_dengan_db_pedaree(db: PgPool) {
    let app = rute(AppState::baru(db));
    let res = app
        .oneshot(Request::get("/kesehatan").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let json: serde_json::Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["db"], "pedaree");
}

#[sqlx::test]
async fn rute_tak_dikenal_menjawab_galat_baku(db: PgPool) {
    let app = rute(AppState::baru(db));
    let res = app
        .oneshot(Request::get("/tidak-ada").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let json: serde_json::Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(json["kode"], "RUTE_TIDAK_ADA");
}

#[sqlx::test]
async fn nama_item_aktif_unik_per_rumah_tangga_tanpa_pedulikan_huruf(db: PgPool) {
    buat_item(&db, "itm_A1", "Bawang Merah", HH).await.unwrap();
    let ganda = buat_item(&db, "itm_A2", "  bawang MERAH ", HH)
        .await
        .unwrap_err();
    assert_eq!(kode_pg(&ganda).as_deref(), Some("23505"));
    // rumah tangga lain boleh memakai nama yang sama
    buat_item(&db, "itm_A3", "Bawang Merah", "hh_LAIN")
        .await
        .unwrap();
}

#[sqlx::test]
async fn item_diarsipkan_membebaskan_nama(db: PgPool) {
    buat_item(&db, "itm_A1", "Tomat", HH).await.unwrap();
    sqlx::query("UPDATE pantry_items SET arsip = TRUE WHERE id = 'itm_A1'")
        .execute(&db)
        .await
        .unwrap();
    buat_item(&db, "itm_A2", "Tomat", HH).await.unwrap();
    // mengaktifkan kembali yang lama bertabrakan dengan yang aktif
    let e = sqlx::query("UPDATE pantry_items SET arsip = FALSE WHERE id = 'itm_A1'")
        .execute(&db)
        .await
        .unwrap_err();
    assert_eq!(kode_pg(&e).as_deref(), Some("23505"));
}

#[sqlx::test]
async fn maksimum_400_item_aktif_per_rumah_tangga(db: PgPool) {
    sqlx::query(
        "INSERT INTO pantry_items (id, household_id, nama, kategori, satuan_simpan, satuan_tampil,
            stok_minimum_x10, umur_simpan_hari_default, lokasi_default)
         SELECT 'itm_' || g, $1, 'Bahan ' || g, 'KERING', 'gram', 'gram', 0, 0, 'SUHU_RUANG'
           FROM generate_series(1, 400) g",
    )
    .bind(HH)
    .execute(&db)
    .await
    .unwrap();
    let e = buat_item(&db, "itm_X", "Bahan ke-401", HH)
        .await
        .unwrap_err();
    assert_eq!(kode_pg(&e).as_deref(), Some("PDI01"));
    // item yang diarsipkan tidak dihitung; rumah tangga lain tidak terpengaruh
    buat_item(&db, "itm_Y", "Bahan lain", "hh_LAIN")
        .await
        .unwrap();
    sqlx::query("UPDATE pantry_items SET arsip = TRUE WHERE id = 'itm_1'")
        .execute(&db)
        .await
        .unwrap();
    buat_item(&db, "itm_Z", "Bahan pengganti", HH)
        .await
        .unwrap();
}

#[sqlx::test]
async fn maksimum_2000_batch_berisi_per_rumah_tangga(db: PgPool) {
    buat_item(&db, "itm_A1", "Beras", HH).await.unwrap();
    sqlx::query(
        "INSERT INTO stok_batch (id, household_id, pantry_item_id, jumlah_awal_x10, jumlah_aktif_x10,
            tanggal_masuk, tanggal_kedaluwarsa, lokasi, sumber)
         SELECT 'btc_' || g, $1, 'itm_A1', 10000, 10000, '2026-09-28', '2026-10-19',
                'SUHU_RUANG', 'MASUK_BELANJA'
           FROM generate_series(1, 2000) g",
    )
    .bind(HH)
    .execute(&db)
    .await
    .unwrap();
    let e = buat_batch(&db, "btc_X", "itm_A1", 10000).await.unwrap_err();
    assert_eq!(kode_pg(&e).as_deref(), Some("PDB01"));
    // batch kosong (riwayat) tidak dihitung terhadap batas
    sqlx::query("UPDATE stok_batch SET jumlah_aktif_x10 = 0, status = 'HABIS' WHERE id = 'btc_1'")
        .execute(&db)
        .await
        .unwrap();
    buat_batch(&db, "btc_Y", "itm_A1", 10000).await.unwrap();
}

#[sqlx::test]
async fn mutasi_hanya_menerima_lima_jenis_dengan_tanda_yang_benar(db: PgPool) {
    buat_item(&db, "itm_A1", "Garam", HH).await.unwrap();
    buat_batch(&db, "btc_A1", "itm_A1", 10000).await.unwrap();
    let masukkan = |id: &'static str, jenis: &'static str, jumlah: i64| {
        let db = db.clone();
        async move {
            sqlx::query(
                "INSERT INTO mutasi_stok (id, household_id, batch_id, jenis, jumlah_x10,
                    sisa_setelah_x10, alasan, aktor)
                 VALUES ($1, $2, 'btc_A1', $3, $4, 100, 'uji mutasi', 'usr_1')",
            )
            .bind(id)
            .bind(HH)
            .bind(jenis)
            .bind(jumlah)
            .execute(&db)
            .await
        }
    };
    for (id, jenis, jumlah) in [
        ("mut_1", "MASUK", 500),
        ("mut_2", "PAKAI", -500),
        ("mut_3", "KOREKSI", -100),
        ("mut_4", "PINDAH", 0),
        ("mut_5", "BUANG", -50),
    ] {
        masukkan(id, jenis, jumlah)
            .await
            .unwrap_or_else(|e| panic!("{jenis}: {e}"));
    }
    assert!(
        masukkan("mut_6", "KELUAR", -5).await.is_err(),
        "jenis lama ditolak"
    );
    assert!(
        masukkan("mut_7", "PAKAI", 500).await.is_err(),
        "PAKAI harus negatif"
    );
    assert!(
        masukkan("mut_8", "MASUK", -500).await.is_err(),
        "MASUK harus positif"
    );
    assert!(
        masukkan("mut_9", "PINDAH", 5).await.is_err(),
        "PINDAH harus nol"
    );
}

#[sqlx::test]
async fn satuan_di_luar_13_baku_dan_nama_pendek_ditolak(db: PgPool) {
    let e = sqlx::query(
        "INSERT INTO pantry_items (id, household_id, nama, kategori, satuan_simpan, satuan_tampil,
            stok_minimum_x10, umur_simpan_hari_default, lokasi_default)
         VALUES ('itm_S', $1, 'Gula Pasir', 'KERING', 'ons', 'gram', 0, 0, 'SUHU_RUANG')",
    )
    .bind(HH)
    .execute(&db)
    .await
    .unwrap_err();
    assert_eq!(kode_pg(&e).as_deref(), Some("23514"));
    assert!(buat_item(&db, "itm_P", "ab", HH).await.is_err());
}

#[sqlx::test]
async fn migrasi_balik_memulihkan_dan_dapat_dijalankan_ulang(db: PgPool) {
    // sqlx::test sudah menjalankan semua migrasi maju.
    MIGRATOR.undo(&db, 1).await.unwrap();
    let ada: bool = sqlx::query_scalar("SELECT to_regclass('public.stok_batch') IS NOT NULL")
        .fetch_one(&db)
        .await
        .unwrap();
    assert!(!ada, "stok_batch harus hilang setelah undo ke 001");
    let lama: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns
          WHERE table_name = 'pantry_items' AND column_name = 'satuan')",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert!(lama, "skema datar 001 dipulihkan");

    MIGRATOR.run(&db).await.unwrap();
    buat_item(&db, "itm_A1", "Kangkung", HH).await.unwrap();

    MIGRATOR.undo(&db, 0).await.unwrap();
    let sisa: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.tables
          WHERE table_schema = 'public' AND table_name IN ('pantry_items', 'stok_batch', 'mutasi_stok')",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(sisa, 0);
}
