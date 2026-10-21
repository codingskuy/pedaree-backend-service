//! Uji `GET /web/dashboard` (PDR-BE-006). Hari ini (WIB) dibekukan di 2026-10-21 oleh
//! `dukung::jam_tetap`; riwayat dibuat dengan memundurkan `dibuat_pada` dan `terjadi_pada`.

use axum::http::Method;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sqlx::PgPool;

mod dukung;
use dukung::{kirim, Klien, HH};

fn waktu(teks: &str) -> DateTime<Utc> {
    teks.parse().unwrap()
}

async fn mundurkan_item(db: &PgPool, item: &str, dibuat: &str) {
    sqlx::query("UPDATE pantry_items SET dibuat_pada = $2 WHERE id = $1")
        .bind(item)
        .bind(waktu(dibuat))
        .execute(db)
        .await
        .unwrap();
}

async fn mundurkan_mutasi(db: &PgPool, item: &str, jenis: &str, kapan: &str) {
    sqlx::query(
        "UPDATE mutasi_stok m SET terjadi_pada = $3 FROM stok_batch b
          WHERE m.batch_id = b.id AND b.pantry_item_id = $1 AND m.jenis = $2",
    )
    .bind(item)
    .bind(jenis)
    .bind(waktu(kapan))
    .execute(db)
    .await
    .unwrap();
}

async fn dasbor(k: &Klien) -> Value {
    let (s, j) = k.get("/web/dashboard").await;
    assert_eq!(s, 200, "{j}");
    j
}

fn titik(j: &Value, tanggal: &str) -> f64 {
    j["stockoutRate90Hari"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["tanggal"] == tanggal)
        .unwrap_or_else(|| panic!("tanggal {tanggal} tidak ada"))["stockoutRate"]
        .as_f64()
        .unwrap()
}

#[sqlx::test]
async fn stockout_90_hari_diturunkan_dari_buku_mutasi_dan_top_item_habis(db: PgPool) {
    let k = Klien::baru(&db).await;
    // Beras: aktif sejak 07-01, stok baru masuk 10-10 -> habis 07-24..10-09 (78 hari, 1 episode).
    let beras = k.item("Beras", "gram", 2000.0, 180).await;
    mundurkan_item(&db, &beras, "2026-07-01T00:00:00Z").await;
    k.masuk(&beras, 5000.0, "gram", "2026-10-21", None).await;
    mundurkan_mutasi(&db, &beras, "MASUK", "2026-10-10T03:00:00Z").await;
    // Gula: aktif sejak 10-01, masuk 10-01, habis dipakai 10-18 -> habis 10-18..10-21 (4 hari).
    let gula = k.item("Gula", "gram", 500.0, 365).await;
    mundurkan_item(&db, &gula, "2026-10-01T00:00:00Z").await;
    k.masuk(&gula, 1000.0, "gram", "2026-10-21", None).await;
    mundurkan_mutasi(&db, &gula, "MASUK", "2026-10-01T03:00:00Z").await;
    let (s, j) = k
        .post(
            "/pantry/consume",
            json!({"pemakaian": [{"pantryItemId": gula, "jumlah": 1000, "satuan": "gram"}]}),
        )
        .await;
    assert_eq!(s, 200, "{j}");
    mundurkan_mutasi(&db, &gula, "PAKAI", "2026-10-18T03:00:00Z").await;

    let j = dasbor(&k).await;
    assert_eq!(j["stockoutRate90Hari"].as_array().unwrap().len(), 90);
    assert_eq!(j["stockoutRate90Hari"][0]["tanggal"], "2026-07-24");
    assert_eq!(titik(&j, "2026-09-01"), 100.0); // hanya Beras aktif, habis
    assert_eq!(titik(&j, "2026-10-05"), 50.0); // Beras habis, Gula ada
    assert_eq!(titik(&j, "2026-10-12"), 0.0); // keduanya ada
    assert_eq!(titik(&j, "2026-10-21"), 50.0); // Gula habis lagi
    assert_eq!(j["stockoutRateRata90Hari"], 83.9);
    assert_eq!(
        (
            j["totalItemAktif"].as_i64(),
            j["habis"].as_i64(),
            j["menipis"].as_i64()
        ),
        (Some(2), Some(1), Some(0))
    );
    let top = j["itemSeringHabis"].as_array().unwrap();
    assert_eq!(top.len(), 2);
    assert_eq!(
        (
            top[0]["nama"].as_str(),
            top[0]["kejadianHabis"].as_i64(),
            top[0]["hariHabis"].as_i64()
        ),
        (Some("Beras"), Some(1), Some(78))
    );
    assert_eq!(
        (
            top[1]["nama"].as_str(),
            top[1]["kejadianHabis"].as_i64(),
            top[1]["hariHabis"].as_i64()
        ),
        (Some("Gula"), Some(1), Some(4))
    );
    assert_eq!(top[0]["pantryItemId"], beras.as_str());
    assert_eq!(top[0]["stokMinimum"], 2000.0);
}

#[sqlx::test]
async fn waste_bulan_berjalan_dan_per_kategori(db: PgPool) {
    let k = Klien::baru(&db).await;
    let buat = |nama: &'static str, kategori: &'static str, satuan: &'static str| {
        let k = &k;
        async move {
            let (s, j) = k
                .post(
                    "/pantry/items",
                    json!({"nama": nama, "kategori": kategori, "satuanSimpan": satuan,
                        "satuanTampil": satuan, "stokMinimum": 100, "umurSimpanHariDefault": 5,
                        "lokasiDefault": "KULKAS_BAWAH"}),
                )
                .await;
            assert_eq!(s, 201, "{j}");
            j["id"].as_str().unwrap().to_owned()
        }
    };
    let bayam = buat("Bayam", "SAYUR", "gram").await;
    let minyak = buat("Minyak Goreng", "CAIR", "ml").await;
    let b_bayam = k
        .masuk(&bayam, 800.0, "gram", "2026-10-15", Some("2026-10-25"))
        .await;
    let b_minyak = k.masuk(&minyak, 1000.0, "ml", "2026-10-15", None).await;
    for (b, jumlah, satuan) in [(&b_bayam, 200, "gram"), (&b_minyak, 300, "ml")] {
        let (s, j) = k
            .post(
                &format!("/pantry/batches/{b}/buang"),
                json!({"jumlah": jumlah, "satuan": satuan, "alasan": "busuk"}),
            )
            .await;
        assert_eq!(s, 200, "{j}");
    }
    // Buang bayam bulan lalu (September) 150 g.
    let (s, j) = k
        .post(
            &format!("/pantry/batches/{b_bayam}/buang"),
            json!({"jumlah": 150, "satuan": "gram", "alasan": "busuk"}),
        )
        .await;
    assert_eq!(s, 200, "{j}");
    sqlx::query(
        "UPDATE mutasi_stok SET terjadi_pada = $1 WHERE jenis = 'BUANG' AND jumlah_x10 = -1500",
    )
    .bind(waktu("2026-09-10T03:00:00Z"))
    .execute(&db)
    .await
    .unwrap();

    let j = dasbor(&k).await;
    assert_eq!(j["wasteBulanGram"], 500.0);
    let rinci: Vec<(String, String, f64)> = j["wastePerKategori"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| {
            (
                w["bulan"].as_str().unwrap().to_owned(),
                w["kategori"].as_str().unwrap().to_owned(),
                w["wasteGram"].as_f64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        rinci,
        vec![
            ("2026-09".to_owned(), "SAYUR".to_owned(), 150.0),
            ("2026-10".to_owned(), "CAIR".to_owned(), 300.0),
            ("2026-10".to_owned(), "SAYUR".to_owned(), 200.0),
        ]
    );
}

#[sqlx::test]
async fn kartu_ringkas_segera_kedaluwarsa_dan_sinyal_7_hari(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Tahu", "pcs", 1.0, 3).await;
    k.masuk(&item, 2.0, "pcs", "2026-10-20", Some("2026-10-23"))
        .await; // +2 hari
    k.masuk(&item, 2.0, "pcs", "2026-10-20", Some("2026-10-24"))
        .await; // +3 hari
    k.masuk(&item, 2.0, "pcs", "2026-10-20", Some("2026-10-26"))
        .await; // +5 hari
    k.masuk(&item, 2.0, "pcs", "2026-10-15", Some("2026-10-20"))
        .await; // sudah lewat
    for (id, status, kapan) in [
        ("sig_a", "TERKIRIM", "2026-10-18T00:00:00Z"), // dalam 7 hari
        ("sig_b", "TERKIRIM", "2026-10-10T00:00:00Z"), // lebih lama
        ("sig_c", "ANTRE", "2026-10-20T00:00:00Z"),    // belum terkirim
    ] {
        sqlx::query(
            "INSERT INTO sinyal_stok (id, household_id, item_id, jenis, tanggal, payload, status, dikirim_pada)
             VALUES ($1, $2, $3, 'stok.menipis', $4, '{}'::jsonb, $5, $6)",
        )
        .bind(id)
        .bind(HH)
        .bind(&item)
        .bind(chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap() + chrono::Duration::days(id.len() as i64 + (id.as_bytes()[4] - b'a') as i64))
        .bind(status)
        .bind(waktu(kapan))
        .execute(&db)
        .await
        .unwrap();
    }
    let j = dasbor(&k).await;
    assert_eq!(j["segeraKedaluwarsa3Hari"], 2);
    assert_eq!(j["sinyalTerkirim7Hari"], 1);
}

#[sqlx::test]
async fn data_rumah_tangga_lain_tidak_ikut_terhitung(db: PgPool) {
    let a = Klien::baru(&db).await;
    let b = Klien::untuk(&db, "hh_LAIN", "siti").await;
    a.item("Beras", "gram", 100.0, 10).await;
    let j = dasbor(&b).await;
    assert_eq!(j["totalItemAktif"], 0);
    assert!(j["itemSeringHabis"].as_array().unwrap().is_empty());
    assert_eq!(j["stockoutRateRata90Hari"], 0.0);
}

#[sqlx::test]
async fn otentikasi_service_key_atau_token_pengguna(db: PgPool) {
    let k = Klien::baru(&db).await;
    k.item("Beras", "gram", 100.0, 10).await;
    let get = |h: &'static [(&'static str, &'static str)]| {
        let app = k.app.clone();
        async move { kirim(&app, Method::GET, "/web/dashboard", h, None).await }
    };
    let (s, j) = get(&[]).await;
    assert_eq!((s, j["kode"].as_str()), (401, Some("TIDAK_SAH")));
    let (s, j) = get(&[("x-service-key", "salah"), ("x-household-id", HH)]).await;
    assert_eq!((s, j["kode"].as_str()), (401, Some("TIDAK_SAH")));
    let (s, j) = get(&[("x-service-key", "kunci-layanan-uji")]).await;
    assert_eq!((s, j["field"].as_str()), (400, Some("X-Household-Id")));
    let (s, j) = get(&[
        ("x-service-key", "kunci-layanan-uji"),
        ("x-household-id", HH),
    ])
    .await;
    assert_eq!(s, 200, "{j}");
    assert_eq!(j["totalItemAktif"], 1);
}
