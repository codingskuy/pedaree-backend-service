//! Uji endpoint stok (PDR-BE-004): FEFO, konversi 13 satuan, idempotensi, mutasi 1:1.
//! Hari ini (WIB) dibekukan di 2026-10-21 oleh `dukung::jam_tetap`.

use axum::http::Method;
use serde_json::{json, Value};
use sqlx::PgPool;

mod dukung;
use dukung::{Klien, HARI_INI};

fn arr(j: &Value, kunci: &str) -> Vec<Value> {
    j[kunci]
        .as_array()
        .unwrap_or_else(|| panic!("{kunci} bukan array: {j}"))
        .clone()
}

/// Invarian buku kas: untuk tiap batch, jumlah seluruh mutasi = jumlahAktif sekarang.
async fn periksa_buku_kas(db: &PgPool) {
    let selisih: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM stok_batch b
          WHERE b.jumlah_aktif_x10 <> COALESCE((SELECT SUM(jumlah_x10) FROM mutasi_stok m WHERE m.batch_id = b.id), 0)",
    )
    .fetch_one(db)
    .await
    .unwrap();
    assert_eq!(
        selisih, 0,
        "ada batch yang jumlahnya tidak cocok dengan buku mutasi"
    );
}

// ---------------------------------------------------------------------------
// Item

#[sqlx::test]
async fn buat_item_mengembalikan_kartu_dan_menolak_nama_ganda(db: PgPool) {
    let k = Klien::baru(&db).await;
    let badan = json!({"nama": "Bawang Merah", "kategori": "BUMBU", "satuanSimpan": "gram",
        "satuanTampil": "gram", "stokMinimum": 250, "umurSimpanHariDefault": 21,
        "lokasiDefault": "BASKOM_BAWANG"});
    let (s, j) = k.post("/pantry/items", badan.clone()).await;
    assert_eq!(s, 201, "{j}");
    assert!(j["id"].as_str().unwrap().starts_with("itm_"));
    assert_eq!(j["householdId"], "hh_01J9Z8QK2M4B");
    assert_eq!(j["stokMinimum"], 250.0);
    assert_eq!(j["arsip"], false);

    let mut sama = badan.clone();
    sama["nama"] = json!("  bawang merah ");
    let (s, j) = k.post("/pantry/items", sama).await;
    assert_eq!((s, j["kode"].as_str()), (409, Some("KONFLIK")));
    assert_eq!(j["pesan"], "Item dengan nama ini sudah ada");
}

#[sqlx::test]
async fn buat_item_memvalidasi_isian_dan_satuan(db: PgPool) {
    let k = Klien::baru(&db).await;
    let dasar = json!({"nama": "Beras", "kategori": "KERING", "satuanSimpan": "gram",
        "satuanTampil": "gram", "stokMinimum": 2000, "umurSimpanHariDefault": 180,
        "lokasiDefault": "SUHU_RUANG"});
    let ubah = |f: &str, v: Value| {
        let mut b = dasar.clone();
        b[f] = v;
        b
    };
    let kasus = [
        (ubah("nama", json!("ab")), 400, "VALIDASI_GAGAL", "nama"),
        (
            ubah("umurSimpanHariDefault", json!(366)),
            400,
            "VALIDASI_GAGAL",
            "umurSimpanHariDefault",
        ),
        (
            ubah("stokMinimum", json!(-1)),
            400,
            "VALIDASI_GAGAL",
            "stokMinimum",
        ),
        (
            ubah("fotoUrl", json!("http://x/y.jpg")),
            400,
            "VALIDASI_GAGAL",
            "fotoUrl",
        ),
        (ubah("kategori", json!("SNACK")), 400, "VALIDASI_GAGAL", ""),
        (
            ubah("satuanSimpan", json!("ons")),
            400,
            "VALIDASI_GAGAL",
            "",
        ),
        (
            ubah("satuanSimpan", json!("ikat")),
            422,
            "SATUAN_TIDAK_BERLAKU",
            "satuanSimpan",
        ),
        (
            ubah("satuanTampil", json!("liter")),
            422,
            "SATUAN_TIDAK_BERLAKU",
            "satuanTampil",
        ),
    ];
    for (badan, status, kode, field) in kasus {
        let (s, j) = k.post("/pantry/items", badan.clone()).await;
        assert_eq!(
            (s, j["kode"].as_str()),
            (status, Some(kode)),
            "{badan} => {j}"
        );
        if !field.is_empty() {
            assert_eq!(j["field"], field);
        }
    }
}

#[sqlx::test]
async fn satuan_simpan_kanonis_dan_minimum_dikonversi(db: PgPool) {
    let k = Klien::baru(&db).await;
    let (s, j) = k
        .post(
            "/pantry/items",
            json!({"nama": "Beras", "kategori": "KERING", "satuanSimpan": "kg", "satuanTampil": "kg",
                   "stokMinimum": 2, "umurSimpanHariDefault": 180, "lokasiDefault": "SUHU_RUANG"}),
        )
        .await;
    assert_eq!(s, 201, "{j}");
    assert_eq!(j["satuanSimpan"], "gram", "kg disimpan sebagai gram");
    assert_eq!(j["satuanTampil"], "kg");
    assert_eq!(j["stokMinimum"], 2000.0);
}

#[sqlx::test]
async fn post_tanpa_x_request_id_ditolak_400(db: PgPool) {
    let k = Klien::baru(&db).await;
    let (s, j) = k
        .kirim(
            Method::POST,
            "/pantry/items",
            None,
            Some(json!({"nama": "Garam"})),
        )
        .await;
    assert_eq!((s, j["field"].as_str()), (400, Some("X-Request-Id")));
    let (s, j) = k
        .kirim(
            Method::POST,
            "/pantry/items",
            Some("bukan-uuid"),
            Some(json!({})),
        )
        .await;
    assert_eq!((s, j["field"].as_str()), (400, Some("X-Request-Id")));
}

#[sqlx::test]
async fn item_ke_401_ditolak_karena_batas_400_item_aktif(db: PgPool) {
    let k = Klien::baru(&db).await;
    sqlx::query(
        "INSERT INTO pantry_items (id, household_id, nama, kategori, satuan_simpan, satuan_tampil,
            stok_minimum_x10, umur_simpan_hari_default, lokasi_default)
         SELECT 'itm_' || g, $1, 'Bahan ' || g, 'KERING', 'gram', 'gram', 0, 0, 'SUHU_RUANG'
           FROM generate_series(1, 400) g",
    )
    .bind(&k.household)
    .execute(&db)
    .await
    .unwrap();
    let (s, j) = k
        .post(
            "/pantry/items",
            json!({"nama": "Bahan ke-401", "kategori": "KERING", "satuanSimpan": "gram",
                   "satuanTampil": "gram", "stokMinimum": 0, "umurSimpanHariDefault": 0,
                   "lokasiDefault": "SUHU_RUANG"}),
        )
        .await;
    assert_eq!((s, j["kode"].as_str()), (422, Some("BATAS_ITEM_AKTIF")));
}

// ---------------------------------------------------------------------------
// Catat masuk

#[sqlx::test]
async fn catat_masuk_membuat_batch_dan_satu_mutasi_masuk(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Beras", "gram", 2000.0, 180).await;
    let (s, j) = k
        .post(
            "/pantry/batches",
            json!({"pantryItemId": item, "jumlah": 1.5, "satuan": "kg", "tanggalMasuk": "2026-10-20",
                   "lokasi": "SUHU_RUANG", "sumber": "MASUK_BELANJA"}),
        )
        .await;
    assert_eq!(s, 201, "{j}");
    assert_eq!(j["jumlahAwal"], 1500.0);
    assert_eq!(j["jumlahAktif"], 1500.0);
    assert_eq!(j["status"], "AKTIF");
    // default kedaluwarsa = masuk + umur simpan (180 hari)
    assert_eq!(j["tanggalKedaluwarsa"], "2027-04-18");
    let (jenis, jumlah, sisa, aktor): (String, i64, i64, String) = sqlx::query_as(
        "SELECT jenis, jumlah_x10, sisa_setelah_x10, aktor FROM mutasi_stok WHERE batch_id = $1",
    )
    .bind(j["id"].as_str().unwrap())
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!((jenis.as_str(), jumlah, sisa), ("MASUK", 15_000, 15_000));
    assert!(aktor.starts_with("usr_"));
    periksa_buku_kas(&db).await;
}

#[sqlx::test]
async fn umur_simpan_nol_menghasilkan_kedaluwarsa_null(db: PgPool) {
    let k = Klien::baru(&db).await;
    let garam = k.item("Garam", "gram", 250.0, 0).await;
    let b = k.masuk(&garam, 500.0, "gram", HARI_INI, None).await;
    let (_, j) = k.get(&format!("/pantry/items/{garam}")).await;
    assert_eq!(j["batches"][0]["id"], b.as_str());
    assert!(j["batches"][0]["tanggalKedaluwarsa"].is_null());
}

#[sqlx::test]
async fn catat_masuk_menolak_data_tidak_valid(db: PgPool) {
    let k = Klien::baru(&db).await;
    let beras = k.item("Beras", "gram", 0.0, 180).await;
    let telur = k.item("Telur Ayam", "pcs", 6.0, 14).await;
    let basis = |item: &str, jumlah: Value, satuan: &str, masuk: &str, kdl: Value| {
        json!({"pantryItemId": item, "jumlah": jumlah, "satuan": satuan, "tanggalMasuk": masuk,
               "tanggalKedaluwarsa": kdl, "lokasi": "SUHU_RUANG", "sumber": "MASUK_BELANJA"})
    };
    let kasus = [
        // jumlah <= 0 -> 400
        (
            basis(&beras, json!(0), "gram", HARI_INI, Value::Null),
            400,
            "VALIDASI_GAGAL",
        ),
        (
            basis(&beras, json!(-5), "gram", HARI_INI, Value::Null),
            400,
            "VALIDASI_GAGAL",
        ),
        // > 50.000 g / ml per batch -> 422
        (
            basis(&beras, json!(50_000.1), "gram", HARI_INI, Value::Null),
            422,
            "JUMLAH_MELEBIHI_BATAS",
        ),
        (
            basis(&beras, json!(51), "kg", HARI_INI, Value::Null),
            422,
            "JUMLAH_MELEBIHI_BATAS",
        ),
        // satuan hitung pecahan -> 422
        (
            basis(&telur, json!(1.5), "pcs", HARI_INI, Value::Null),
            422,
            "JUMLAH_PECAHAN",
        ),
        // satuan tak berlaku -> 422
        (
            basis(&beras, json!(1), "ikat", HARI_INI, Value::Null),
            422,
            "SATUAN_TIDAK_BERLAKU",
        ),
        (
            basis(&beras, json!(1), "liter", HARI_INI, Value::Null),
            422,
            "SATUAN_TIDAK_BERLAKU",
        ),
        (
            basis(&telur, json!(100), "gram", HARI_INI, Value::Null),
            422,
            "SATUAN_TIDAK_BERLAKU",
        ),
        // tanggal
        (
            basis(&beras, json!(1), "gram", "2026-10-22", Value::Null),
            422,
            "TANGGAL_MASUK_TIDAK_VALID",
        ),
        (
            basis(&beras, json!(1), "gram", "2026-10-13", Value::Null),
            422,
            "TANGGAL_MASUK_TIDAK_VALID",
        ),
        (
            basis(&beras, json!(1), "gram", "2026-10-20", json!("2026-10-19")),
            422,
            "TANGGAL_KEDALUWARSA_TIDAK_VALID",
        ),
        // item tak ada
        (
            basis("itm_TIDAK_ADA", json!(1), "gram", HARI_INI, Value::Null),
            404,
            "TIDAK_ADA",
        ),
    ];
    for (badan, status, kode) in kasus {
        let (s, j) = k.post("/pantry/batches", badan.clone()).await;
        assert_eq!(
            (s, j["kode"].as_str()),
            (status, Some(kode)),
            "{badan} => {j}"
        );
    }
    // batas: tepat 50.000 g, mundur tepat 7 hari, tanpa mutasi dari yang ditolak
    assert_eq!(k.jumlah_mutasi().await, 0);
    k.masuk(&beras, 50.0, "kg", "2026-10-14", None).await;
    assert_eq!(k.jumlah_mutasi().await, 1);
}

// ---------------------------------------------------------------------------
// Catat pakai (FEFO)

#[sqlx::test]
async fn pakai_memotong_batch_kedaluwarsa_terdekat_dulu(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Tomat", "gram", 100.0, 5).await;
    let a = k
        .masuk(&item, 300.0, "gram", "2026-10-20", Some("2026-10-30"))
        .await;
    let b = k
        .masuk(&item, 200.0, "gram", "2026-10-21", Some("2026-10-25"))
        .await;
    let c = k
        .masuk(&item, 100.0, "gram", "2026-10-19", Some("2026-11-10"))
        .await;

    let (s, j) = k
        .post(
            "/pantry/consume",
            json!({"pemakaian": [{"pantryItemId": item, "jumlah": 250, "satuan": "gram"}]}),
        )
        .await;
    assert_eq!(s, 200, "{j}");
    let t = arr(&j, "terpotong");
    assert_eq!(t.len(), 2);
    assert_eq!(
        (t[0]["batchId"].as_str(), t[0]["jumlah"].as_f64()),
        (Some(b.as_str()), Some(200.0))
    );
    assert_eq!(
        (t[1]["batchId"].as_str(), t[1]["jumlah"].as_f64()),
        (Some(a.as_str()), Some(50.0))
    );
    assert!(arr(&j, "kurang").is_empty());
    assert_eq!(k.aktif_x10(&b).await, 0);
    assert_eq!(k.aktif_x10(&a).await, 2500);
    assert_eq!(k.aktif_x10(&c).await, 1000);
    let status_b: String = sqlx::query_scalar("SELECT status FROM stok_batch WHERE id = $1")
        .bind(&b)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(status_b, "HABIS");

    // satu mutasi PAKAI per batch tersentuh, negatif, sisaSetelah benar
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT batch_id, jumlah_x10, sisa_setelah_x10 FROM mutasi_stok WHERE jenis = 'PAKAI' ORDER BY terjadi_pada, id",
    )
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.contains(&(b.clone(), -2000, 0)));
    assert!(rows.contains(&(a.clone(), -500, 2500)));
    periksa_buku_kas(&db).await;
}

#[sqlx::test]
async fn pakai_kurang_menghabiskan_stok_dan_melaporkan_kekurangan(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Tomat", "gram", 100.0, 5).await;
    let a = k
        .masuk(&item, 300.0, "gram", "2026-10-20", Some("2026-10-30"))
        .await;
    let c = k
        .masuk(&item, 100.0, "gram", "2026-10-19", Some("2026-11-10"))
        .await;
    let (s, j) = k
        .post(
            "/pantry/consume",
            json!({"pemakaian": [{"pantryItemId": item, "jumlah": 1000, "satuan": "gram"}]}),
        )
        .await;
    assert_eq!(s, 200, "{j}");
    let t = arr(&j, "terpotong");
    assert_eq!(t.len(), 2);
    assert_eq!(t[0]["batchId"], a.as_str());
    assert_eq!(t[1]["batchId"], c.as_str());
    let kurang = arr(&j, "kurang");
    assert_eq!(kurang.len(), 1);
    assert_eq!(kurang[0]["pantryItemId"], item.as_str());
    assert_eq!(kurang[0]["kurangJumlah"], 600.0);
    assert_eq!(kurang[0]["satuanSimpan"], "gram");
    periksa_buku_kas(&db).await;
}

#[sqlx::test]
async fn pakai_tanpa_tanggal_kedaluwarsa_mengikuti_fifo(db: PgPool) {
    let k = Klien::baru(&db).await;
    let garam = k.item("Garam", "gram", 250.0, 0).await;
    let baru = k.masuk(&garam, 100.0, "gram", "2026-10-18", None).await;
    let lama = k.masuk(&garam, 100.0, "gram", "2026-10-15", None).await;
    let (s, j) = k
        .post(
            "/pantry/consume",
            json!({"pemakaian": [{"pantryItemId": garam, "jumlah": 150, "satuan": "gram"}]}),
        )
        .await;
    assert_eq!(s, 200, "{j}");
    let t = arr(&j, "terpotong");
    assert_eq!(t[0]["batchId"], lama.as_str(), "batch tertua keluar dulu");
    assert_eq!(t[1]["batchId"], baru.as_str());
    assert_eq!(k.aktif_x10(&lama).await, 0);
    assert_eq!(k.aktif_x10(&baru).await, 500);
}

#[sqlx::test]
async fn batch_kedaluwarsa_terkunci_dari_fefo_tetapi_hari_ini_masih_boleh(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Bayam", "gram", 100.0, 2).await;
    // kedaluwarsa kemarin (terkunci), hari ini (boleh)
    let basi = k
        .masuk(&item, 500.0, "gram", "2026-10-16", Some("2026-10-20"))
        .await;
    let hari_ini = k
        .masuk(&item, 100.0, "gram", "2026-10-19", Some(HARI_INI))
        .await;
    let (s, j) = k
        .post(
            "/pantry/consume",
            json!({"pemakaian": [{"pantryItemId": item, "jumlah": 200, "satuan": "gram"}]}),
        )
        .await;
    assert_eq!(s, 200, "{j}");
    let t = arr(&j, "terpotong");
    assert_eq!(t.len(), 1);
    assert_eq!(t[0]["batchId"], hari_ini.as_str());
    assert_eq!(arr(&j, "kurang")[0]["kurangJumlah"], 100.0);
    assert_eq!(
        k.aktif_x10(&basi).await,
        5000,
        "batch kedaluwarsa tidak disentuh"
    );

    // hanya tersisa batch kedaluwarsa => stok habis (422), tidak ada mutasi baru
    let sebelum = k.jumlah_mutasi().await;
    let (s, j) = k
        .post(
            "/pantry/consume",
            json!({"pemakaian": [{"pantryItemId": item, "jumlah": 1, "satuan": "gram"}]}),
        )
        .await;
    assert_eq!((s, j["kode"].as_str()), (422, Some("STOK_HABIS")));
    assert_eq!(k.jumlah_mutasi().await, sebelum);
}

#[sqlx::test]
async fn pakai_mengonversi_satuan_sah(db: PgPool) {
    let k = Klien::baru(&db).await;
    let beras = k.item("Beras", "gram", 0.0, 180).await;
    let minyak = k.item("Minyak Goreng", "ml", 0.0, 365).await;
    let bayam = k.item("Bayam", "gram", 0.0, 2).await;
    let b1 = k.masuk(&beras, 5.0, "kg", HARI_INI, None).await;
    let b2 = k.masuk(&minyak, 2.0, "liter", HARI_INI, None).await;
    let b3 = k.masuk(&bayam, 4.0, "ikat", HARI_INI, None).await; // 4 x 200 g
    assert_eq!(k.aktif_x10(&b3).await, 8000);

    let (s, j) = k
        .post(
            "/pantry/consume",
            json!({"pemakaian": [
                {"pantryItemId": beras, "jumlah": 1, "satuan": "kg"},
                {"pantryItemId": minyak, "jumlah": 3, "satuan": "sdm"},
                {"pantryItemId": minyak, "jumlah": 4, "satuan": "sdt"},
                {"pantryItemId": bayam, "jumlah": 2, "satuan": "ikat"},
            ]}),
        )
        .await;
    assert_eq!(s, 200, "{j}");
    assert_eq!(k.aktif_x10(&b1).await, 40_000); // 5000 - 1000 g
    assert_eq!(k.aktif_x10(&b2).await, 20_000 - 450 - 200); // 3 sdm = 45 ml, 4 sdt = 20 ml
    assert_eq!(k.aktif_x10(&b3).await, 4000); // 2 ikat = 400 g
    periksa_buku_kas(&db).await;
}

#[sqlx::test]
async fn pakai_menolak_satuan_dan_jumlah_tidak_sah_dengan_422(db: PgPool) {
    let k = Klien::baru(&db).await;
    let beras = k.item("Beras", "gram", 0.0, 180).await;
    let telur = k.item("Telur Ayam", "pcs", 0.0, 14).await;
    k.masuk(&beras, 10.0, "kg", HARI_INI, None).await;
    k.masuk(&telur, 12.0, "pcs", HARI_INI, None).await;
    let kasus = [
        (
            json!({"pantryItemId": beras, "jumlah": 50.1, "satuan": "kg"}),
            422,
            "JUMLAH_MELEBIHI_BATAS",
        ),
        (
            json!({"pantryItemId": telur, "jumlah": 1.5, "satuan": "pcs"}),
            422,
            "JUMLAH_PECAHAN",
        ),
        (
            json!({"pantryItemId": beras, "jumlah": 1, "satuan": "siung"}),
            422,
            "SATUAN_TIDAK_BERLAKU",
        ),
        (
            json!({"pantryItemId": beras, "jumlah": 1, "satuan": "ml"}),
            422,
            "SATUAN_TIDAK_BERLAKU",
        ),
        (
            json!({"pantryItemId": beras, "jumlah": 0, "satuan": "gram"}),
            400,
            "VALIDASI_GAGAL",
        ),
        (
            json!({"pantryItemId": "itm_TIDAK_ADA", "jumlah": 1, "satuan": "gram"}),
            404,
            "TIDAK_ADA",
        ),
    ];
    for (baris, status, kode) in kasus {
        let (s, j) = k
            .post("/pantry/consume", json!({"pemakaian": [baris.clone()]}))
            .await;
        assert_eq!(
            (s, j["kode"].as_str()),
            (status, Some(kode)),
            "{baris} => {j}"
        );
    }
    let (s, j) = k.post("/pantry/consume", json!({"pemakaian": []})).await;
    assert_eq!((s, j["kode"].as_str()), (400, Some("VALIDASI_GAGAL")));
    assert_eq!(k.jumlah_mutasi().await, 2, "hanya dua mutasi MASUK awal");
}

#[sqlx::test]
async fn pakai_atomik_satu_baris_gagal_membatalkan_semuanya(db: PgPool) {
    let k = Klien::baru(&db).await;
    let beras = k.item("Beras", "gram", 0.0, 180).await;
    let kosong = k.item("Gula Pasir", "gram", 0.0, 0).await;
    let b = k.masuk(&beras, 1000.0, "gram", HARI_INI, None).await;
    let (s, j) = k
        .post(
            "/pantry/consume",
            json!({"pemakaian": [
                {"pantryItemId": beras, "jumlah": 400, "satuan": "gram"},
                {"pantryItemId": kosong, "jumlah": 10, "satuan": "gram"},
            ]}),
        )
        .await;
    assert_eq!((s, j["kode"].as_str()), (422, Some("STOK_HABIS")));
    assert_eq!(j["field"], "pemakaian[1].pantryItemId");
    assert_eq!(
        k.aktif_x10(&b).await,
        10_000,
        "pemotongan baris pertama dibatalkan"
    );
    assert_eq!(k.jumlah_mutasi().await, 1);
}

#[sqlx::test]
async fn pakai_mencatat_resep_pawonee_pada_mutasi(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Beras", "gram", 0.0, 180).await;
    k.masuk(&item, 1000.0, "gram", HARI_INI, None).await;
    let (s, _) = k
        .post(
            "/pantry/consume",
            json!({"pemakaian": [{"pantryItemId": item, "jumlah": 100, "satuan": "gram"}],
                   "sumberResepId": "rcp_nasi_goreng"}),
        )
        .await;
    assert_eq!(s, 200);
    let resep: Option<String> =
        sqlx::query_scalar("SELECT sumber_resep_id FROM mutasi_stok WHERE jenis = 'PAKAI'")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(resep.as_deref(), Some("rcp_nasi_goreng"));
}

// ---------------------------------------------------------------------------
// Idempotensi

#[sqlx::test]
async fn kirim_ulang_x_request_id_tidak_menggandakan_mutasi(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Beras", "gram", 0.0, 180).await;
    let b = k.masuk(&item, 1000.0, "gram", HARI_INI, None).await;
    let rid = uuid::Uuid::new_v4().to_string();
    let badan = json!({"pemakaian": [{"pantryItemId": item, "jumlah": 300, "satuan": "gram"}]});
    let (s1, j1) = k.post_rid("/pantry/consume", &rid, badan.clone()).await;
    let mutasi_setelah_pertama = k.jumlah_mutasi().await;
    let (s2, j2) = k.post_rid("/pantry/consume", &rid, badan.clone()).await;
    let (s3, j3) = k.post_rid("/pantry/consume", &rid, badan).await;
    assert_eq!((s1, s2, s3), (200, 200, 200));
    assert_eq!(j1, j2, "hasil asli dikembalikan");
    assert_eq!(j1, j3);
    assert_eq!(k.jumlah_mutasi().await, mutasi_setelah_pertama);
    assert_eq!(k.aktif_x10(&b).await, 7000, "hanya terpotong sekali");
    periksa_buku_kas(&db).await;
}

#[sqlx::test]
async fn idempotensi_berlaku_untuk_semua_post_dan_menyimpan_status_asli(db: PgPool) {
    let k = Klien::baru(&db).await;
    let rid = uuid::Uuid::new_v4().to_string();
    let badan = json!({"nama": "Garam", "kategori": "KERING", "satuanSimpan": "gram", "satuanTampil": "gram",
        "stokMinimum": 250, "umurSimpanHariDefault": 0, "lokasiDefault": "SUHU_RUANG"});
    let (s1, j1) = k.post_rid("/pantry/items", &rid, badan.clone()).await;
    let (s2, j2) = k.post_rid("/pantry/items", &rid, badan).await;
    assert_eq!(
        (s1, s2),
        (201, 201),
        "kirim ulang mengembalikan 201 asli, bukan 409 nama ganda"
    );
    assert_eq!(j1, j2);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM pantry_items")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(n, 1);

    // POST /pantry/batches
    let item = j1["id"].as_str().unwrap().to_owned();
    let rid2 = uuid::Uuid::new_v4().to_string();
    let masuk = json!({"pantryItemId": item, "jumlah": 100, "satuan": "gram", "tanggalMasuk": HARI_INI,
        "lokasi": "SUHU_RUANG", "sumber": "MASUK_BELANJA"});
    let (_, b1) = k.post_rid("/pantry/batches", &rid2, masuk.clone()).await;
    let (_, b2) = k.post_rid("/pantry/batches", &rid2, masuk).await;
    assert_eq!(b1, b2);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM stok_batch")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[sqlx::test]
async fn x_request_id_yang_sama_untuk_operasi_lain_bentrok_409(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Beras", "gram", 0.0, 180).await;
    k.masuk(&item, 1000.0, "gram", HARI_INI, None).await;
    let rid = uuid::Uuid::new_v4().to_string();
    let (s, _) = k
        .post_rid(
            "/pantry/consume",
            &rid,
            json!({"pemakaian": [{"pantryItemId": item, "jumlah": 1, "satuan": "gram"}]}),
        )
        .await;
    assert_eq!(s, 200);
    let (s, j) = k
        .post_rid(
            "/pantry/opname",
            &rid,
            json!({"pantryItemId": item, "stokFisik": 10, "satuan": "gram", "alasan": "uji"}),
        )
        .await;
    assert_eq!((s, j["kode"].as_str()), (409, Some("IDEMPOTENSI_BENTROK")));
}

#[sqlx::test]
async fn hasil_idempotensi_kedaluwarsa_setelah_24_jam(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Beras", "gram", 0.0, 180).await;
    let b = k.masuk(&item, 1000.0, "gram", HARI_INI, None).await;
    let rid = uuid::Uuid::new_v4().to_string();
    let badan = json!({"pemakaian": [{"pantryItemId": item, "jumlah": 100, "satuan": "gram"}]});
    k.post_rid("/pantry/consume", &rid, badan.clone()).await;
    // masih dalam 24 jam: tidak diproses ulang
    sqlx::query("UPDATE idempotensi SET dibuat_pada = now() - interval '23 hours'")
        .execute(&db)
        .await
        .unwrap();
    k.post_rid("/pantry/consume", &rid, badan.clone()).await;
    assert_eq!(k.aktif_x10(&b).await, 9000);
    // lewat 24 jam: dianggap permintaan baru
    sqlx::query("UPDATE idempotensi SET dibuat_pada = now() - interval '25 hours'")
        .execute(&db)
        .await
        .unwrap();
    let (s, _) = k.post_rid("/pantry/consume", &rid, badan).await;
    assert_eq!(s, 200);
    assert_eq!(k.aktif_x10(&b).await, 8000);
    // pembersihan berkala menghapus yang lama
    sqlx::query("UPDATE idempotensi SET dibuat_pada = now() - interval '30 hours'")
        .execute(&db)
        .await
        .unwrap();
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM idempotensi")
        .fetch_one(&db)
        .await
        .unwrap();
    let dihapus = pedaree_backend_service::idempotensi::bersihkan(&db)
        .await
        .unwrap();
    assert!(total > 0);
    assert_eq!(dihapus as i64, total);
}

#[sqlx::test]
async fn permintaan_serentak_dengan_id_sama_hanya_diproses_sekali(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Beras", "gram", 0.0, 180).await;
    let b = k.masuk(&item, 1000.0, "gram", HARI_INI, None).await;
    let rid = uuid::Uuid::new_v4().to_string();
    let badan = json!({"pemakaian": [{"pantryItemId": item, "jumlah": 100, "satuan": "gram"}]});
    let (r1, r2, r3) = tokio::join!(
        k.post_rid("/pantry/consume", &rid, badan.clone()),
        k.post_rid("/pantry/consume", &rid, badan.clone()),
        k.post_rid("/pantry/consume", &rid, badan.clone()),
    );
    assert_eq!((r1.0, r2.0, r3.0), (200, 200, 200));
    assert_eq!(r1.1, r2.1);
    assert_eq!(r1.1, r3.1);
    assert_eq!(k.aktif_x10(&b).await, 9000);
}

#[sqlx::test]
async fn galat_tidak_disimpan_sehingga_klien_boleh_mengulang(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Beras", "gram", 0.0, 180).await;
    let rid = uuid::Uuid::new_v4().to_string();
    let badan = json!({"pemakaian": [{"pantryItemId": item, "jumlah": 100, "satuan": "gram"}]});
    let (s, _) = k.post_rid("/pantry/consume", &rid, badan.clone()).await;
    assert_eq!(s, 422, "stok masih kosong");
    k.masuk(&item, 1000.0, "gram", HARI_INI, None).await;
    let (s, _) = k.post_rid("/pantry/consume", &rid, badan).await;
    assert_eq!(s, 200, "ID yang sama boleh dipakai ulang setelah galat");
}

// ---------------------------------------------------------------------------
// Opname

#[sqlx::test]
async fn opname_negatif_mengambil_dari_batch_fefo_sebagai_koreksi(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Bayam", "gram", 100.0, 2).await;
    let dekat = k
        .masuk(&item, 200.0, "gram", "2026-10-20", Some("2026-10-22"))
        .await;
    let jauh = k
        .masuk(&item, 300.0, "gram", "2026-10-20", Some("2026-10-28"))
        .await;
    let (s, j) = k
        .post("/pantry/opname", json!({"pantryItemId": item, "stokFisik": 250, "satuan": "gram", "alasan": "susut bayam"}))
        .await;
    assert_eq!(s, 200, "{j}");
    let m = arr(&j, "mutasi");
    assert_eq!(m.len(), 2);
    assert_eq!(
        (
            m[0]["batchId"].as_str(),
            m[0]["jenis"].as_str(),
            m[0]["jumlah"].as_f64()
        ),
        (Some(dekat.as_str()), Some("KOREKSI"), Some(-200.0))
    );
    assert_eq!(
        (
            m[1]["batchId"].as_str(),
            m[1]["jumlah"].as_f64(),
            m[1]["sisaSetelah"].as_f64()
        ),
        (Some(jauh.as_str()), Some(-50.0), Some(250.0))
    );
    assert_eq!(m[0]["alasan"], "susut bayam");
    assert_eq!(k.aktif_x10(&dekat).await, 0);
    assert_eq!(k.aktif_x10(&jauh).await, 2500);
    periksa_buku_kas(&db).await;
}

#[sqlx::test]
async fn opname_positif_membuat_batch_masuk_koreksi(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Bayam", "gram", 100.0, 2).await;
    k.masuk(&item, 200.0, "gram", HARI_INI, Some("2026-10-23"))
        .await;
    let (s, j) = k
        .post("/pantry/opname", json!({"pantryItemId": item, "stokFisik": 1, "satuan": "kg", "alasan": "salah takar kemarin"}))
        .await;
    assert_eq!(s, 200, "{j}");
    let m = arr(&j, "mutasi");
    assert_eq!(m.len(), 1);
    assert_eq!(
        (m[0]["jenis"].as_str(), m[0]["jumlah"].as_f64()),
        (Some("KOREKSI"), Some(800.0))
    );
    let (sumber, kdl, awal): (String, Option<chrono::NaiveDate>, i64) = sqlx::query_as(
        "SELECT sumber, tanggal_kedaluwarsa, jumlah_awal_x10 FROM stok_batch WHERE id = $1",
    )
    .bind(m[0]["batchId"].as_str().unwrap())
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(sumber, "MASUK_KOREKSI");
    assert_eq!(
        kdl.unwrap().to_string(),
        "2026-10-23",
        "hari ini + umur default 2 hari"
    );
    assert_eq!(awal, 8000);
    periksa_buku_kas(&db).await;
}

#[sqlx::test]
async fn opname_tanpa_selisih_tidak_menulis_mutasi_dan_validasi_alasan(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Beras", "gram", 0.0, 180).await;
    k.masuk(&item, 1000.0, "gram", HARI_INI, None).await;
    let sebelum = k.jumlah_mutasi().await;
    let (s, j) = k
        .post("/pantry/opname", json!({"pantryItemId": item, "stokFisik": 1000, "satuan": "gram", "alasan": "cek rutin"}))
        .await;
    assert_eq!(s, 200);
    assert!(arr(&j, "mutasi").is_empty());
    assert_eq!(k.jumlah_mutasi().await, sebelum);
    let panjang = "x".repeat(141);
    for alasan in ["", "ab", panjang.as_str()] {
        let (s, j) = k
            .post(
                "/pantry/opname",
                json!({"pantryItemId": item, "stokFisik": 900, "satuan": "gram", "alasan": alasan}),
            )
            .await;
        assert_eq!((s, j["field"].as_str()), (400, Some("alasan")), "{alasan}");
    }
}

#[sqlx::test]
async fn opname_konflik_bila_stok_catatan_berubah(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Beras", "gram", 0.0, 180).await;
    k.masuk(&item, 1000.0, "gram", HARI_INI, None).await;
    let (s, j) = k
        .post(
            "/pantry/opname",
            json!({"pantryItemId": item, "stokFisik": 900, "satuan": "gram", "alasan": "timbang ulang", "stokCatatTerakhir": 800}),
        )
        .await;
    assert_eq!((s, j["kode"].as_str()), (409, Some("KONFLIK_OPNAME")));
    let (s, _) = k
        .post(
            "/pantry/opname",
            json!({"pantryItemId": item, "stokFisik": 900, "satuan": "gram", "alasan": "timbang ulang", "stokCatatTerakhir": 1000}),
        )
        .await;
    assert_eq!(s, 200);
}

#[sqlx::test]
async fn opname_ke_nol_menghabiskan_semua_batch(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Beras", "gram", 0.0, 180).await;
    let b = k.masuk(&item, 1000.0, "gram", HARI_INI, None).await;
    let (s, _) = k
        .post("/pantry/opname", json!({"pantryItemId": item, "stokFisik": 0, "satuan": "gram", "alasan": "tumpah semua"}))
        .await;
    assert_eq!(s, 200);
    assert_eq!(k.aktif_x10(&b).await, 0);
    periksa_buku_kas(&db).await;
}

// ---------------------------------------------------------------------------
// Pindah

#[sqlx::test]
async fn pindah_menulis_mutasi_nol_dan_memperbarui_lokasi(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Cabai Merah", "gram", 100.0, 7).await;
    let b = k.masuk(&item, 250.0, "gram", HARI_INI, None).await;
    let (s, j) = k
        .post(
            &format!("/pantry/batches/{b}/pindah"),
            json!({"lokasiTujuan": "KULKAS_BAWAH"}),
        )
        .await;
    assert_eq!(s, 200, "{j}");
    assert_eq!(j["lokasi"], "KULKAS_BAWAH");
    assert_eq!(j["jumlahAktif"], 250.0);
    let (jenis, jumlah, sisa, alasan): (String, i64, i64, String) = sqlx::query_as(
        "SELECT jenis, jumlah_x10, sisa_setelah_x10, alasan FROM mutasi_stok WHERE jenis = 'PINDAH'",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!((jenis.as_str(), jumlah, sisa), ("PINDAH", 0, 2500));
    assert_eq!(alasan, "SUHU_RUANG -> KULKAS_BAWAH");

    let (s, j) = k
        .post(
            &format!("/pantry/batches/{b}/pindah"),
            json!({"lokasiTujuan": "KULKAS_BAWAH"}),
        )
        .await;
    assert_eq!((s, j["kode"].as_str()), (422, Some("LOKASI_SAMA")));
    let (s, j) = k
        .post(
            "/pantry/batches/btc_TIDAK_ADA/pindah",
            json!({"lokasiTujuan": "FREEZER"}),
        )
        .await;
    assert_eq!((s, j["kode"].as_str()), (404, Some("TIDAK_ADA")));
    let (s, _) = k
        .post(
            &format!("/pantry/batches/{b}/pindah"),
            json!({"lokasiTujuan": "GARASI"}),
        )
        .await;
    assert_eq!(s, 400);
}

// ---------------------------------------------------------------------------
// Buang

#[sqlx::test]
async fn buang_sebagian_lalu_habis_menjadi_dibuang(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Tomat", "gram", 100.0, 5).await;
    let b = k
        .masuk(&item, 500.0, "gram", "2026-10-16", Some("2026-10-20"))
        .await; // sudah kedaluwarsa
    let (s, j) = k
        .post(
            &format!("/pantry/batches/{b}/buang"),
            json!({"jumlah": 150, "satuan": "gram", "alasan": "lewat tanggal"}),
        )
        .await;
    assert_eq!(s, 200, "{j}");
    assert_eq!(
        (
            j["jenis"].as_str(),
            j["jumlah"].as_f64(),
            j["sisaSetelah"].as_f64()
        ),
        (Some("BUANG"), Some(-150.0), Some(350.0))
    );
    assert_eq!(j["alasan"], "lewat tanggal");
    let (_, d) = k.get(&format!("/pantry/items/{item}")).await;
    assert_eq!(
        d["batches"][0]["status"], "KEDALUWARSA",
        "sisa tetap tercatat untuk metrik waste"
    );

    let (s, j) = k
        .post(
            &format!("/pantry/batches/{b}/buang"),
            json!({"jumlah": 350.1, "satuan": "gram", "alasan": "busuk"}),
        )
        .await;
    assert_eq!((s, j["kode"].as_str()), (422, Some("JUMLAH_MELEBIHI_SISA")));
    let (s, _) = k
        .post(
            &format!("/pantry/batches/{b}/buang"),
            json!({"jumlah": 0.35, "satuan": "kg", "alasan": "busuk"}),
        )
        .await;
    assert_eq!(s, 200);
    let status: String = sqlx::query_scalar("SELECT status FROM stok_batch WHERE id = $1")
        .bind(&b)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(status, "DIBUANG");
    periksa_buku_kas(&db).await;
}

#[sqlx::test]
async fn buang_menolak_alasan_di_luar_daftar_baku(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Tomat", "gram", 100.0, 5).await;
    let b = k.masuk(&item, 500.0, "gram", HARI_INI, None).await;
    for alasan in ["bosan", "Busuk", ""] {
        let (s, j) = k
            .post(
                &format!("/pantry/batches/{b}/buang"),
                json!({"jumlah": 10, "satuan": "gram", "alasan": alasan}),
            )
            .await;
        assert_eq!(
            (s, j["kode"].as_str()),
            (400, Some("VALIDASI_GAGAL")),
            "{alasan}"
        );
    }
    for alasan in ["busuk", "berlendir/berbau", "lewat tanggal", "tercemar"] {
        let (s, _) = k
            .post(
                &format!("/pantry/batches/{b}/buang"),
                json!({"jumlah": 10, "satuan": "gram", "alasan": alasan}),
            )
            .await;
        assert_eq!(s, 200, "{alasan}");
    }
}

// ---------------------------------------------------------------------------
// Baca

#[sqlx::test]
async fn daftar_item_menghitung_stok_status_filter_dan_paginasi(db: PgPool) {
    let k = Klien::baru(&db).await;
    let beras = k.item("Beras", "gram", 2000.0, 180).await; // 1500 <= 2000 => MENIPIS
    let gula = k.item("Gula Pasir", "gram", 500.0, 0).await; // 0 => HABIS
    let telur = k.item("Telur Ayam", "pcs", 6.0, 14).await; // 12 > 6 => AMAN
    let _ = k.item("Cabai", "gram", 100.0, 7).await;
    k.masuk(&beras, 1.5, "kg", HARI_INI, None).await;
    k.masuk(&telur, 12.0, "pcs", HARI_INI, None).await;
    // batch kedaluwarsa tidak dihitung ke stok tampil
    k.masuk(&beras, 300.0, "gram", "2026-10-16", Some("2026-10-20"))
        .await;

    let (s, j) = k.get("/pantry/items").await;
    assert_eq!(s, 200, "{j}");
    let data = arr(&j, "data");
    let nama: Vec<&str> = data.iter().map(|d| d["nama"].as_str().unwrap()).collect();
    assert_eq!(
        nama,
        ["Beras", "Cabai", "Gula Pasir", "Telur Ayam"],
        "urut nama"
    );
    assert!(j["nextCursor"].is_null());
    let per = |n: &str| data.iter().find(|d| d["nama"] == n).unwrap().clone();
    assert_eq!(per("Beras")["stokTampil"], 1500.0);
    assert_eq!(per("Beras")["statusStok"], "MENIPIS");
    assert_eq!(per("Gula Pasir")["statusStok"], "HABIS");
    assert_eq!(per("Telur Ayam")["statusStok"], "AMAN");
    assert_eq!(per("Telur Ayam")["stokTampil"], 12.0);
    assert!(per("Beras")["batchTerdekatKedaluwarsa"].is_string());
    let _ = gula;

    let (_, j) = k.get("/pantry/items?statusStok=MENIPIS").await;
    assert_eq!(arr(&j, "data").len(), 1);
    let (_, j) = k.get("/pantry/items?q=pasir").await;
    assert_eq!(arr(&j, "data")[0]["nama"], "Gula Pasir");
    let (_, j) = k.get("/pantry/items?kategori=BUAH").await;
    assert!(arr(&j, "data").is_empty());
    let (s, _) = k.get("/pantry/items?statusStok=BASI").await;
    assert_eq!(s, 400);
    let (s, _) = k.get("/pantry/items?limit=101").await;
    assert_eq!(s, 400);

    // paginasi kursor
    let (_, h1) = k.get("/pantry/items?limit=3").await;
    assert_eq!(arr(&h1, "data").len(), 3);
    let kursor = h1["nextCursor"].as_str().unwrap();
    let (_, h2) = k
        .get(&format!("/pantry/items?limit=3&cursor={kursor}"))
        .await;
    let sisa = arr(&h2, "data");
    assert_eq!(sisa.len(), 1);
    assert_eq!(sisa[0]["nama"], "Telur Ayam");
    assert!(h2["nextCursor"].is_null());
    let (s, _) = k.get("/pantry/items?cursor=%25%25").await;
    assert_eq!(s, 400);
}

#[sqlx::test]
async fn daftar_item_kursor_kosong_dianggap_tidak_ada(db: PgPool) {
    // Temuan Schemathesis (PDR-BE-TEST-SCHEMATHESIS): klien web lazim mengirim `cursor=` kosong
    // pada halaman pertama; itu harus sama dengan tanpa kursor, bukan 400.
    let k = Klien::baru(&db).await;
    let (s, j) = k
        .post(
            "/pantry/items",
            json!({"nama": "Beras", "kategori": "KERING", "satuanSimpan": "gram",
                "satuanTampil": "gram", "stokMinimum": 2000, "umurSimpanHariDefault": 180,
                "lokasiDefault": "SUHU_RUANG"}),
        )
        .await;
    assert_eq!(s, 201, "{j}");
    let (s, j) = k.get("/pantry/items?cursor=").await;
    assert_eq!(s, 200, "{j}");
    assert_eq!(arr(&j, "data").len(), 1);
}

#[sqlx::test]
async fn daftar_item_menampilkan_stok_dalam_satuan_tampil(db: PgPool) {
    let k = Klien::baru(&db).await;
    let (_, j) = k
        .post(
            "/pantry/items",
            json!({"nama": "Bawang Merah", "kategori": "BUMBU", "satuanSimpan": "gram", "satuanTampil": "siung",
                   "stokMinimum": 250, "umurSimpanHariDefault": 21, "lokasiDefault": "BASKOM_BAWANG"}),
        )
        .await;
    let item = j["id"].as_str().unwrap().to_owned();
    k.masuk(&item, 500.0, "gram", HARI_INI, None).await;
    let (_, j) = k.get("/pantry/items").await;
    let d = &arr(&j, "data")[0];
    assert_eq!(
        (d["stokTampil"].as_f64(), d["satuanTampil"].as_str()),
        (Some(50.0), Some("siung"))
    );
    // 505 g = 50,5 siung (pecahan): kembali ke gram
    k.masuk(&item, 5.0, "gram", HARI_INI, None).await;
    let (_, j) = k.get("/pantry/items").await;
    let d = &arr(&j, "data")[0];
    assert_eq!(
        (d["stokTampil"].as_f64(), d["satuanTampil"].as_str()),
        (Some(505.0), Some("gram"))
    );
}

#[sqlx::test]
async fn detail_item_memuat_batch_fefo_dan_status_turunan(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Tomat", "gram", 100.0, 5).await;
    let basi = k
        .masuk(&item, 100.0, "gram", "2026-10-16", Some("2026-10-20"))
        .await;
    let baik = k
        .masuk(&item, 200.0, "gram", HARI_INI, Some("2026-10-26"))
        .await;
    let (s, j) = k.get(&format!("/pantry/items/{item}")).await;
    assert_eq!(s, 200, "{j}");
    assert_eq!(j["id"], item.as_str());
    assert_eq!(j["stokTampil"], 200.0, "batch kedaluwarsa tidak dihitung");
    let b = arr(&j, "batches");
    assert_eq!(b[0]["id"], basi.as_str());
    assert_eq!(b[0]["status"], "KEDALUWARSA");
    assert_eq!(b[1]["id"], baik.as_str());
    assert_eq!(b[1]["status"], "AKTIF");
    let (s, j) = k.get("/pantry/items/itm_TIDAK_ADA").await;
    assert_eq!((s, j["kode"].as_str()), (404, Some("TIDAK_ADA")));
}

#[sqlx::test]
async fn kedaluwarsa_diurutkan_dengan_status_sop(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Tomat", "gram", 100.0, 30).await;
    let lewat = k
        .masuk(&item, 10.0, "gram", "2026-10-16", Some("2026-10-19"))
        .await;
    let hari_ini = k
        .masuk(&item, 20.0, "gram", "2026-10-18", Some(HARI_INI))
        .await;
    let segera = k
        .masuk(&item, 30.0, "gram", "2026-10-18", Some("2026-10-23"))
        .await; // 2 hari
    let waspada = k
        .masuk(&item, 40.0, "gram", "2026-10-18", Some("2026-10-27"))
        .await; // 6 hari
    let _aman = k
        .masuk(&item, 50.0, "gram", "2026-10-18", Some("2026-11-20"))
        .await;
    let _tanpa = k
        .masuk(&item, 60.0, "gram", "2026-10-18", Some("2026-10-21"))
        .await; // juga hari ini

    let (s, j) = k.get("/pantry/expiring").await;
    assert_eq!(s, 200, "{j}");
    let d = arr(&j, "data");
    let urut: Vec<(&str, &str)> = d
        .iter()
        .map(|x| {
            (
                x["batchId"].as_str().unwrap(),
                x["status"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(urut[0], (lewat.as_str(), "KEDALUWARSA"));
    assert_eq!(urut[1].1, "HARI_INI");
    assert_eq!(urut[2].1, "HARI_INI");
    assert!(urut.iter().any(|(b, s)| *b == hari_ini && *s == "HARI_INI"));
    assert_eq!(urut[3], (segera.as_str(), "SEGERA"));
    assert_eq!(urut[4], (waspada.as_str(), "WASPADA"));
    assert_eq!(d.len(), 5, "yang AMAN (lebih dari 7 hari) tidak masuk");
    assert_eq!(d[3]["sisa"], 30.0);

    let (_, j) = k.get("/pantry/expiring?dalamHari=2").await;
    assert_eq!(
        arr(&j, "data").len(),
        4,
        "kedaluwarsa, dua batch hari-ini, dan SEGERA (2 hari lagi)"
    );
}

// ---------------------------------------------------------------------------
// Isolasi rumah tangga dan buku kas

#[sqlx::test]
async fn data_rumah_tangga_lain_tidak_terlihat_dan_tidak_bisa_diubah(db: PgPool) {
    let a = Klien::baru(&db).await;
    let b = Klien::untuk(&db, "hh_LAIN", "sari").await;
    let item = a.item("Beras", "gram", 0.0, 180).await;
    let batch = a.masuk(&item, 1000.0, "gram", HARI_INI, None).await;

    let (s, _) = b.get(&format!("/pantry/items/{item}")).await;
    assert_eq!(s, 404);
    let (_, j) = b.get("/pantry/items").await;
    assert!(arr(&j, "data").is_empty());
    let (s, _) = b
        .post(
            "/pantry/consume",
            json!({"pemakaian": [{"pantryItemId": item, "jumlah": 1, "satuan": "gram"}]}),
        )
        .await;
    assert_eq!(s, 404);
    let (s, _) = b
        .post(
            &format!("/pantry/batches/{batch}/pindah"),
            json!({"lokasiTujuan": "FREEZER"}),
        )
        .await;
    assert_eq!(s, 404);
    let (s, _) = b
        .post(
            &format!("/pantry/batches/{batch}/buang"),
            json!({"jumlah": 1, "satuan": "gram", "alasan": "busuk"}),
        )
        .await;
    assert_eq!(s, 404);
    let (s, _) = b
        .post(
            "/pantry/batches",
            json!({"pantryItemId": item, "jumlah": 1, "satuan": "gram", "tanggalMasuk": HARI_INI,
            "lokasi": "SUHU_RUANG", "sumber": "MASUK_BELANJA"}),
        )
        .await;
    assert_eq!(s, 404);
    assert_eq!(a.aktif_x10(&batch).await, 10_000);

    // nama item yang sama boleh dipakai rumah tangga lain
    let (s, _) = b
        .post(
            "/pantry/items",
            json!({"nama": "Beras", "kategori": "KERING", "satuanSimpan": "gram", "satuanTampil": "gram",
                   "stokMinimum": 0, "umurSimpanHariDefault": 180, "lokasiDefault": "SUHU_RUANG"}),
        )
        .await;
    assert_eq!(s, 201);
}

#[sqlx::test]
async fn endpoint_stok_menolak_tanpa_token(db: PgPool) {
    let k = Klien::baru(&db).await;
    for (metode, path) in [
        (Method::GET, "/pantry/items"),
        (Method::GET, "/pantry/expiring"),
        (Method::POST, "/pantry/consume"),
    ] {
        let (s, j) = dukung::kirim(
            &k.app,
            metode,
            path,
            &[("x-household-id", "hh_01J9Z8QK2M4B")],
            None,
        )
        .await;
        assert_eq!((s, j["kode"].as_str()), (401, Some("TIDAK_SAH")), "{path}");
    }
}

#[sqlx::test]
async fn buku_kas_konsisten_setelah_urutan_operasi_campuran(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Beras", "gram", 100.0, 180).await;
    let b1 = k.masuk(&item, 5.0, "kg", HARI_INI, None).await;
    k.masuk(&item, 2000.0, "gram", HARI_INI, Some("2026-12-01"))
        .await;
    k.post(
        "/pantry/consume",
        json!({"pemakaian": [{"pantryItemId": item, "jumlah": 1200, "satuan": "gram"}]}),
    )
    .await;
    k.post(
        "/pantry/opname",
        json!({"pantryItemId": item, "stokFisik": 7, "satuan": "kg", "alasan": "timbang ulang"}),
    )
    .await;
    k.post(
        &format!("/pantry/batches/{b1}/pindah"),
        json!({"lokasiTujuan": "KULKAS_ATAS"}),
    )
    .await;
    k.post(
        &format!("/pantry/batches/{b1}/buang"),
        json!({"jumlah": 100, "satuan": "gram", "alasan": "tercemar"}),
    )
    .await;
    k.post(
        "/pantry/opname",
        json!({"pantryItemId": item, "stokFisik": 3, "satuan": "kg", "alasan": "dipakai tetangga"}),
    )
    .await;
    periksa_buku_kas(&db).await;
    let (_, d) = k.get(&format!("/pantry/items/{item}")).await;
    assert_eq!(d["stokTampil"], 3000.0);
}
