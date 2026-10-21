//! Uji sinyal `pedaree.stok.v1` (PDR-BE-005): pemicu, dedup 24 jam, expiry-sweep, outbox,
//! pengiriman ke TitipO (mock), retry 5/30/300 detik lalu DLQ.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::{Json, Router};
use chrono::{Duration, TimeZone, Utc};
use pedaree_backend_service::config::Rahasia;
use pedaree_backend_service::pengirim::{self, Ringkasan, Tujuan};
use pedaree_backend_service::sinyal;
use serde_json::{json, Value};
use sqlx::PgPool;

mod dukung;
use dukung::{JamUji, Klien, HARI_INI};

// ---------------------------------------------------------------------------
// Bantuan

#[derive(Debug, Clone)]
struct Sinyal {
    id: String,
    jenis: String,
    status: String,
    percobaan: i32,
    payload: Value,
}

async fn sinyal_rows(db: &PgPool) -> Vec<Sinyal> {
    let rows: Vec<(String, String, String, i32, Value)> = sqlx::query_as(
        "SELECT id, jenis, status, percobaan, payload FROM sinyal_stok ORDER BY dibuat_pada, id",
    )
    .fetch_all(db)
    .await
    .unwrap();
    rows.into_iter()
        .map(|(id, jenis, status, percobaan, payload)| Sinyal {
            id,
            jenis,
            status,
            percobaan,
            payload,
        })
        .collect()
}

async fn pakai(
    k: &Klien,
    item: &str,
    jumlah: f64,
    satuan: &str,
    resep: Option<&str>,
) -> (u16, Value) {
    k.post(
        "/pantry/consume",
        json!({"pemakaian": [{"pantryItemId": item, "jumlah": jumlah, "satuan": satuan}], "sumberResepId": resep}),
    )
    .await
}

fn ambil<'a>(v: &'a [Sinyal], jenis: &str) -> Vec<&'a Sinyal> {
    v.iter().filter(|s| s.jenis == jenis).collect()
}

// ---------------------------------------------------------------------------
// Mock penerima TitipO

/// (path, X-Service-Key, badan JSON)
type Permintaan = (String, Option<String>, Value);

#[derive(Clone, Default)]
struct Mock {
    masuk: Arc<Mutex<Vec<Permintaan>>>,
    jawaban: Arc<Mutex<VecDeque<(u16, Value)>>>,
}

impl Mock {
    fn antre(&self, status: u16, badan: Value) {
        self.jawaban.lock().unwrap().push_back((status, badan));
    }

    fn permintaan(&self) -> Vec<Permintaan> {
        self.masuk.lock().unwrap().clone()
    }
}

async fn tangkap(
    State(m): State<Mock>,
    uri: Uri,
    headers: HeaderMap,
    badan: Bytes,
) -> (StatusCode, Json<Value>) {
    let kunci = headers
        .get("x-service-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let json: Value = serde_json::from_slice(&badan).unwrap_or(Value::Null);
    m.masuk
        .lock()
        .unwrap()
        .push((uri.path().to_owned(), kunci, json));
    let (status, badan) = m
        .jawaban
        .lock()
        .unwrap()
        .pop_front()
        .unwrap_or((201, json!({"status": "dicatat_sebagai_anjuran"})));
    (StatusCode::from_u16(status).unwrap(), Json(badan))
}

async fn jalankan_mock() -> (Tujuan, Mock) {
    let mock = Mock::default();
    let app = Router::new().fallback(tangkap).with_state(mock.clone());
    let pendengar = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let alamat = pendengar.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(pendengar, app).await.unwrap();
    });
    let tujuan = Tujuan {
        base_url: format!("http://{alamat}"),
        service_key: Rahasia::baru("kunci-layanan-uji"),
    };
    (tujuan, mock)
}

/// Satu item menipis -> satu sinyal ANTRE di outbox.
async fn buat_satu_sinyal(k: &Klien) -> Sinyal {
    let item = k.item("Cabai Merah", "gram", 100.0, 7).await;
    k.masuk(&item, 300.0, "gram", HARI_INI, Some("2026-10-26"))
        .await;
    let (s, _) = pakai(k, &item, 250.0, "gram", None).await;
    assert_eq!(s, 200);
    let v = sinyal_rows(&k.db).await;
    assert_eq!(v.len(), 1);
    v[0].clone()
}

// ---------------------------------------------------------------------------
// Pemicu

#[sqlx::test]
async fn pakai_ke_menipis_menulis_satu_sinyal_dengan_payload_lengkap(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Cabai Merah", "gram", 100.0, 7).await;
    k.masuk(&item, 300.0, "gram", HARI_INI, Some("2026-10-26"))
        .await;
    let (s, _) = pakai(&k, &item, 250.0, "gram", Some("rcp_sambal")).await;
    assert_eq!(s, 200);

    let v = sinyal_rows(&db).await;
    assert_eq!(v.len(), 1);
    assert_eq!(
        (v[0].jenis.as_str(), v[0].status.as_str(), v[0].percobaan),
        ("stok.menipis", "ANTRE", 0)
    );
    let p = &v[0].payload;
    assert_eq!(p["jenis"], "stok.menipis");
    assert_eq!(p["item_id"], item.as_str());
    assert_eq!(p["tanggal"], HARI_INI);
    assert_eq!(p["item"], "Cabai Merah");
    assert_eq!(p["sisa"], 50.0);
    assert_eq!(p["satuan"], "gram");
    assert_eq!(p["kedaluwarsa"], "2026-10-26");
    assert_eq!(p["recipe_id"], "rcp_sambal");
    let hh: String = sqlx::query_scalar("SELECT household_id FROM sinyal_stok")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(hh, k.household);
}

#[sqlx::test]
async fn pakai_sampai_habis_menulis_stok_habis(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Tomat", "gram", 100.0, 5).await;
    k.masuk(&item, 300.0, "gram", HARI_INI, Some("2026-10-25"))
        .await;
    pakai(&k, &item, 300.0, "gram", None).await;
    let v = sinyal_rows(&db).await;
    assert_eq!(v.len(), 1, "aman -> habis langsung: hanya satu sinyal");
    assert_eq!(v[0].jenis, "stok.habis");
    assert_eq!(v[0].payload["sisa"], 0.0);
    assert_eq!(
        v[0].payload["kedaluwarsa"], "2026-10-25",
        "tanggal batch terakhir yang habis"
    );
    assert!(
        v[0].payload["recipe_id"].is_null(),
        "recipe_id boleh kosong"
    );
}

#[sqlx::test]
async fn maksimal_satu_sinyal_per_item_per_24_jam(db: PgPool) {
    let jam = JamUji::baru();
    let k = Klien::dengan_jam(&db, jam.jam()).await;
    let item = k.item("Tomat", "gram", 100.0, 5).await;
    k.masuk(&item, 300.0, "gram", HARI_INI, Some("2026-10-30"))
        .await;
    pakai(&k, &item, 250.0, "gram", None).await; // menipis
    assert_eq!(sinyal_rows(&db).await.len(), 1);

    // menipis -> habis dalam 24 jam: ditekan (satu sinyal per item per 24 jam)
    pakai(&k, &item, 50.0, "gram", None).await;
    assert_eq!(sinyal_rows(&db).await.len(), 1);

    // isi lagi lalu menipis lagi pada jam ke-23: tetap ditekan
    k.masuk(&item, 300.0, "gram", HARI_INI, Some("2026-10-30"))
        .await;
    jam.majukan(Duration::hours(23));
    pakai(&k, &item, 250.0, "gram", None).await;
    assert_eq!(sinyal_rows(&db).await.len(), 1);

    // lewat 24 jam sejak sinyal pertama: transisi berikutnya dikirim
    k.masuk(&item, 300.0, "gram", HARI_INI, Some("2026-10-30"))
        .await;
    jam.majukan(Duration::hours(2));
    pakai(&k, &item, 350.0, "gram", None).await;
    let v = sinyal_rows(&db).await;
    assert_eq!(v.len(), 2);
    assert_eq!(v[1].jenis, "stok.habis");
}

#[sqlx::test]
async fn dedup_per_item_bukan_per_rumah_tangga(db: PgPool) {
    let k = Klien::baru(&db).await;
    let a = k.item("Tomat", "gram", 100.0, 5).await;
    let b = k.item("Bayam", "gram", 100.0, 2).await;
    for item in [&a, &b] {
        k.masuk(item, 300.0, "gram", HARI_INI, Some("2026-10-25"))
            .await;
        pakai(&k, item, 250.0, "gram", None).await;
    }
    assert_eq!(sinyal_rows(&db).await.len(), 2);
}

#[sqlx::test]
async fn tidak_ada_sinyal_tanpa_transisi(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Beras", "gram", 100.0, 180).await;
    // masuk tidak memicu
    let b = k.masuk(&item, 1000.0, "gram", HARI_INI, None).await;
    // turun tetapi tetap di atas minimum
    pakai(&k, &item, 500.0, "gram", None).await;
    assert!(sinyal_rows(&db).await.is_empty());
    // buang batch kedaluwarsa tidak mengubah stok yang dapat dipakai
    let kdl = k
        .masuk(&item, 100.0, "gram", "2026-10-16", Some("2026-10-20"))
        .await;
    let (s, _) = k
        .post(
            &format!("/pantry/batches/{kdl}/buang"),
            json!({"jumlah": 100, "satuan": "gram", "alasan": "lewat tanggal"}),
        )
        .await;
    assert_eq!(s, 200);
    assert!(sinyal_rows(&db).await.is_empty());
    // pindah lokasi tidak memicu
    k.post(
        &format!("/pantry/batches/{b}/pindah"),
        json!({"lokasiTujuan": "KULKAS_ATAS"}),
    )
    .await;
    // minimum 0: turun sebagian tidak pernah menipis
    let nol = k.item("Garam", "gram", 0.0, 0).await;
    k.masuk(&nol, 100.0, "gram", HARI_INI, None).await;
    pakai(&k, &nol, 90.0, "gram", None).await;
    assert!(sinyal_rows(&db).await.is_empty());
    // sudah menipis lalu turun lagi (belum habis): bukan transisi baru
    let c = k.item("Telur Ayam", "pcs", 6.0, 14).await;
    k.masuk(&c, 5.0, "pcs", HARI_INI, None).await; // 5 <= 6: sudah menipis sejak masuk
    pakai(&k, &c, 2.0, "pcs", None).await;
    assert!(sinyal_rows(&db).await.is_empty());
}

#[sqlx::test]
async fn buang_dan_opname_memicu_sinyal(db: PgPool) {
    let k = Klien::baru(&db).await;
    let a = k.item("Tomat", "gram", 100.0, 5).await;
    let batch = k
        .masuk(&a, 300.0, "gram", HARI_INI, Some("2026-10-25"))
        .await;
    let (s, _) = k
        .post(
            &format!("/pantry/batches/{batch}/buang"),
            json!({"jumlah": 250, "satuan": "gram", "alasan": "busuk"}),
        )
        .await;
    assert_eq!(s, 200);
    let b = k.item("Bayam", "gram", 100.0, 2).await;
    k.masuk(&b, 300.0, "gram", HARI_INI, Some("2026-10-23"))
        .await;
    let (s, _) = k
        .post(
            "/pantry/opname",
            json!({"pantryItemId": b, "stokFisik": 0, "satuan": "gram", "alasan": "tumpah semua"}),
        )
        .await;
    assert_eq!(s, 200);
    let v = sinyal_rows(&db).await;
    assert_eq!(v.len(), 2);
    assert_eq!(ambil(&v, "stok.menipis").len(), 1);
    assert_eq!(ambil(&v, "stok.habis").len(), 1);
    assert_eq!(ambil(&v, "stok.habis")[0].payload["item"], "Bayam");
}

#[sqlx::test]
async fn sinyal_dibatalkan_bersama_mutasi_bila_permintaan_gagal(db: PgPool) {
    let k = Klien::baru(&db).await;
    let a = k.item("Tomat", "gram", 100.0, 5).await;
    let kosong = k.item("Gula Pasir", "gram", 0.0, 0).await;
    let batch = k
        .masuk(&a, 300.0, "gram", HARI_INI, Some("2026-10-25"))
        .await;
    let (s, _) = k
        .post(
            "/pantry/consume",
            json!({"pemakaian": [
                {"pantryItemId": a, "jumlah": 250, "satuan": "gram"},
                {"pantryItemId": kosong, "jumlah": 1, "satuan": "gram"},
            ]}),
        )
        .await;
    assert_eq!(s, 422);
    assert!(sinyal_rows(&db).await.is_empty(), "outbox ikut dibatalkan");
    assert_eq!(k.aktif_x10(&batch).await, 3000);
}

#[sqlx::test]
async fn kirim_ulang_idempoten_tidak_menggandakan_sinyal(db: PgPool) {
    let k = Klien::baru(&db).await;
    let item = k.item("Tomat", "gram", 100.0, 5).await;
    k.masuk(&item, 300.0, "gram", HARI_INI, Some("2026-10-25"))
        .await;
    let rid = uuid::Uuid::new_v4().to_string();
    let badan = json!({"pemakaian": [{"pantryItemId": item, "jumlah": 250, "satuan": "gram"}]});
    k.post_rid("/pantry/consume", &rid, badan.clone()).await;
    k.post_rid("/pantry/consume", &rid, badan).await;
    assert_eq!(sinyal_rows(&db).await.len(), 1);
}

// ---------------------------------------------------------------------------
// expiry-sweep

#[sqlx::test]
async fn expiry_sweep_membuat_kedaluwarsa_dekat_sekali_per_item(db: PgPool) {
    let jam = JamUji::baru();
    let k = Klien::dengan_jam(&db, jam.jam()).await;
    // Tomat: dua batch dalam jendela 3 hari + satu jauh
    let tomat = k.item("Tomat", "gram", 10.0, 30).await;
    k.masuk(&tomat, 100.0, "gram", "2026-10-18", Some("2026-10-23"))
        .await; // 2 hari
    k.masuk(&tomat, 50.0, "gram", "2026-10-18", Some("2026-10-24"))
        .await; // 3 hari
    k.masuk(&tomat, 70.0, "gram", "2026-10-18", Some("2026-10-30"))
        .await; // jauh
                // Bayam kedaluwarsa hari ini
    let bayam = k.item("Bayam", "gram", 10.0, 2).await;
    k.masuk(&bayam, 80.0, "gram", "2026-10-19", Some(HARI_INI))
        .await;
    // Bukan kandidat: WASPADA (4 hari), sudah lewat, tanpa tanggal
    let wortel = k.item("Wortel", "gram", 10.0, 14).await;
    k.masuk(&wortel, 100.0, "gram", "2026-10-18", Some("2026-10-25"))
        .await;
    let susu = k.item("Susu", "ml", 10.0, 7).await;
    k.masuk(&susu, 100.0, "ml", "2026-10-16", Some("2026-10-20"))
        .await;
    let garam = k.item("Garam", "gram", 10.0, 0).await;
    k.masuk(&garam, 100.0, "gram", HARI_INI, None).await;
    // Telur sudah punya sinyal menipis (24 jam terakhir): ditekan
    let telur = k.item("Telur Ayam", "pcs", 6.0, 14).await;
    k.masuk(&telur, 10.0, "pcs", "2026-10-19", Some("2026-10-22"))
        .await;
    pakai(&k, &telur, 6.0, "pcs", None).await;
    assert_eq!(sinyal_rows(&db).await.len(), 1);

    let dibuat = sinyal::expiry_sweep(&db, jam.sekarang()).await.unwrap();
    assert_eq!(dibuat, 2);
    let v = sinyal_rows(&db).await;
    let dekat = ambil(&v, "kedaluwarsa.dekat");
    assert_eq!(dekat.len(), 2);
    let untuk = |id: &str| {
        dekat
            .iter()
            .find(|s| s.payload["item_id"] == id)
            .unwrap()
            .payload
            .clone()
    };
    let p = untuk(&tomat);
    assert_eq!(p["item"], "Tomat");
    assert_eq!(p["sisa"], 150.0, "jumlah batch dalam jendela 3 hari");
    assert_eq!(p["kedaluwarsa"], "2026-10-23", "batch terdekat");
    assert_eq!(p["satuan"], "gram");
    assert_eq!(p["tanggal"], HARI_INI);
    assert!(p["recipe_id"].is_null());
    assert_eq!(untuk(&bayam)["kedaluwarsa"], HARI_INI);
    assert!(dekat.iter().all(|s| s.payload["item_id"] != telur.as_str()));

    // eksekusi ulang: tidak ada sinyal ganda
    assert_eq!(sinyal::expiry_sweep(&db, jam.sekarang()).await.unwrap(), 0);
    assert_eq!(sinyal_rows(&db).await.len(), 3);
    // 25 jam kemudian (mundur satu hari batch): item yang masih dekat dikirim lagi
    jam.majukan(Duration::hours(25));
    assert!(sinyal::expiry_sweep(&db, jam.sekarang()).await.unwrap() >= 1);
}

#[sqlx::test]
async fn sweep_harian_diklaim_sekali_per_hari_wib(db: PgPool) {
    let jam = JamUji::baru();
    let k = Klien::dengan_jam(&db, jam.jam()).await;
    let item = k.item("Bayam", "gram", 10.0, 2).await;
    k.masuk(&item, 80.0, "gram", "2026-10-19", Some(HARI_INI))
        .await;

    assert_eq!(
        sinyal::jalankan_sweep_harian(&db, jam.sekarang())
            .await
            .unwrap(),
        Some(1)
    );
    assert_eq!(
        sinyal::jalankan_sweep_harian(&db, jam.sekarang())
            .await
            .unwrap(),
        None
    );
    let (selesai, dibuat): (Option<chrono::DateTime<Utc>>, Option<i32>) =
        sqlx::query_as("SELECT selesai_pada, sinyal_dibuat FROM expiry_sweep_log")
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(selesai.is_some());
    assert_eq!(dibuat, Some(1));
    // hari WIB berikutnya diklaim lagi
    jam.majukan(Duration::hours(24));
    assert!(sinyal::jalankan_sweep_harian(&db, jam.sekarang())
        .await
        .unwrap()
        .is_some());
}

// ---------------------------------------------------------------------------
// Pengiriman ke TitipO

#[sqlx::test]
async fn kirim_201_menandai_terkirim_dengan_header_service_key_dan_payload_asli(db: PgPool) {
    let k = Klien::baru(&db).await;
    let s = buat_satu_sinyal(&k).await;
    let (tujuan, mock) = jalankan_mock().await;
    let klien = pengirim::klien_http().unwrap();
    let t0 = Utc.with_ymd_and_hms(2026, 10, 21, 5, 0, 0).unwrap();

    let r = pengirim::kirim_antrean(&db, &klien, &tujuan, t0)
        .await
        .unwrap();
    assert_eq!(
        r,
        Ringkasan {
            terkirim: 1,
            ..Default::default()
        }
    );
    let masuk = mock.permintaan();
    assert_eq!(masuk.len(), 1);
    assert_eq!(
        masuk[0].0, "/sinyal/pedaree-stok",
        "hanya endpoint sinyal; tidak ada pembuatan pesanan"
    );
    assert_eq!(
        masuk[0].1.as_deref(),
        Some("kunci-layanan-uji"),
        "header X-Service-Key"
    );
    assert_eq!(masuk[0].2, s.payload);
    for wajib in [
        "item",
        "sisa",
        "satuan",
        "kedaluwarsa",
        "recipe_id",
        "item_id",
        "tanggal",
        "jenis",
    ] {
        assert!(
            masuk[0].2.get(wajib).is_some(),
            "payload wajib memuat {wajib}"
        );
    }
    let (status, jawaban, percobaan): (String, Option<String>, i32) =
        sqlx::query_as("SELECT status, jawaban, percobaan FROM sinyal_stok WHERE id = $1")
            .bind(&s.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(
        (status.as_str(), jawaban.as_deref(), percobaan),
        ("TERKIRIM", Some("dicatat_sebagai_anjuran"), 1)
    );

    // tidak dikirim dua kali
    let r = pengirim::kirim_antrean(&db, &klien, &tujuan, t0 + Duration::hours(1))
        .await
        .unwrap();
    assert_eq!(r, Ringkasan::default());
    assert_eq!(mock.permintaan().len(), 1);
}

#[sqlx::test]
async fn jawaban_200_duplikat_diabaikan_bukan_galat(db: PgPool) {
    let k = Klien::baru(&db).await;
    let s = buat_satu_sinyal(&k).await;
    let (tujuan, mock) = jalankan_mock().await;
    mock.antre(200, json!({"status": "duplikat_diabaikan"}));
    let klien = pengirim::klien_http().unwrap();
    let r = pengirim::kirim_antrean(&db, &klien, &tujuan, Utc::now())
        .await
        .unwrap();
    assert_eq!(
        r,
        Ringkasan {
            duplikat: 1,
            ..Default::default()
        }
    );
    let (status, jawaban, galat): (String, Option<String>, Option<String>) =
        sqlx::query_as("SELECT status, jawaban, galat_terakhir FROM sinyal_stok WHERE id = $1")
            .bind(&s.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(
        (status.as_str(), jawaban.as_deref(), galat),
        ("TERKIRIM", Some("duplikat_diabaikan"), None)
    );
}

#[sqlx::test]
async fn kegagalan_sementara_retry_5_30_300_detik_lalu_dlq(db: PgPool) {
    let k = Klien::baru(&db).await;
    let s = buat_satu_sinyal(&k).await;
    let (tujuan, mock) = jalankan_mock().await;
    for _ in 0..4 {
        mock.antre(500, json!({"kode": "GALAT_SERVER"}));
    }
    let klien = pengirim::klien_http().unwrap();
    let t0 = Utc.with_ymd_and_hms(2026, 10, 21, 5, 0, 0).unwrap();
    let keadaan = || async {
        let r: (String, i32, chrono::DateTime<Utc>, Option<String>) = sqlx::query_as(
            "SELECT status, percobaan, coba_lagi_pada, galat_terakhir FROM sinyal_stok WHERE id = $1",
        )
        .bind(&s.id)
        .fetch_one(&db)
        .await
        .unwrap();
        r
    };

    // percobaan 1 gagal -> ulang 5 detik lagi
    let r = pengirim::kirim_antrean(&db, &klien, &tujuan, t0)
        .await
        .unwrap();
    assert_eq!(
        r,
        Ringkasan {
            diulang: 1,
            ..Default::default()
        }
    );
    let (st, n, lagi, galat) = keadaan().await;
    assert_eq!(
        (st.as_str(), n, lagi),
        ("ANTRE", 1, t0 + Duration::seconds(5))
    );
    assert!(galat.unwrap().contains("500"));

    // belum jatuh tempo: tidak dikirim
    pengirim::kirim_antrean(&db, &klien, &tujuan, t0 + Duration::seconds(4))
        .await
        .unwrap();
    assert_eq!(mock.permintaan().len(), 1);

    // retry 1 (t+5) gagal -> 30 detik
    let t1 = t0 + Duration::seconds(5);
    pengirim::kirim_antrean(&db, &klien, &tujuan, t1)
        .await
        .unwrap();
    let (st, n, lagi, _) = keadaan().await;
    assert_eq!(
        (st.as_str(), n, lagi),
        ("ANTRE", 2, t1 + Duration::seconds(30))
    );

    // retry 2 gagal -> 300 detik
    let t2 = t1 + Duration::seconds(30);
    pengirim::kirim_antrean(&db, &klien, &tujuan, t2)
        .await
        .unwrap();
    let (st, n, lagi, _) = keadaan().await;
    assert_eq!(
        (st.as_str(), n, lagi),
        ("ANTRE", 3, t2 + Duration::seconds(300))
    );

    // retry 3 gagal -> DLQ
    let t3 = t2 + Duration::seconds(300);
    let r = pengirim::kirim_antrean(&db, &klien, &tujuan, t3)
        .await
        .unwrap();
    assert_eq!(
        r,
        Ringkasan {
            dlq: 1,
            ..Default::default()
        }
    );
    let (st, n, _, galat) = keadaan().await;
    assert_eq!((st.as_str(), n), ("DLQ", 4));
    assert!(galat.is_some());
    assert_eq!(mock.permintaan().len(), 4, "1 percobaan awal + 3 retry");

    // DLQ tidak dicoba lagi
    pengirim::kirim_antrean(&db, &klien, &tujuan, t3 + Duration::days(1))
        .await
        .unwrap();
    assert_eq!(mock.permintaan().len(), 4);
    assert!(mock
        .permintaan()
        .iter()
        .all(|(p, _, _)| p == "/sinyal/pedaree-stok"));
}

#[sqlx::test]
async fn berhasil_setelah_retry_membersihkan_galat(db: PgPool) {
    let k = Klien::baru(&db).await;
    let s = buat_satu_sinyal(&k).await;
    let (tujuan, mock) = jalankan_mock().await;
    mock.antre(503, json!({}));
    let klien = pengirim::klien_http().unwrap();
    let t0 = Utc.with_ymd_and_hms(2026, 10, 21, 5, 0, 0).unwrap();
    pengirim::kirim_antrean(&db, &klien, &tujuan, t0)
        .await
        .unwrap();
    let r = pengirim::kirim_antrean(&db, &klien, &tujuan, t0 + Duration::seconds(5))
        .await
        .unwrap();
    assert_eq!(r.terkirim, 1);
    let (st, n, galat): (String, i32, Option<String>) =
        sqlx::query_as("SELECT status, percobaan, galat_terakhir FROM sinyal_stok WHERE id = $1")
            .bind(&s.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!((st.as_str(), n, galat), ("TERKIRIM", 2, None));
}

#[sqlx::test]
async fn jaringan_putus_dan_429_diulang_sedangkan_422_langsung_dlq(db: PgPool) {
    let k = Klien::baru(&db).await;
    buat_satu_sinyal(&k).await;
    let klien = pengirim::klien_http().unwrap();
    let t0 = Utc.with_ymd_and_hms(2026, 10, 21, 5, 0, 0).unwrap();

    // port tertutup -> kegagalan sementara
    let mati = Tujuan {
        base_url: "http://127.0.0.1:1".to_owned(),
        service_key: Rahasia::baru("x"),
    };
    let r = pengirim::kirim_antrean(&db, &klien, &mati, t0)
        .await
        .unwrap();
    assert_eq!(r.diulang, 1);

    let (tujuan, mock) = jalankan_mock().await;
    mock.antre(429, json!({}));
    let r = pengirim::kirim_antrean(&db, &klien, &tujuan, t0 + Duration::seconds(5))
        .await
        .unwrap();
    assert_eq!(r.diulang, 1, "429 sementara");

    mock.antre(
        422,
        json!({"kode": "JENIS_SINYAL_TIDAK_DIKENAL", "pesan": "jenis tidak dikenal"}),
    );
    let r = pengirim::kirim_antrean(&db, &klien, &tujuan, t0 + Duration::seconds(35))
        .await
        .unwrap();
    assert_eq!(
        r,
        Ringkasan {
            dlq: 1,
            ..Default::default()
        },
        "422 permanen langsung DLQ"
    );
    let (st, galat): (String, Option<String>) =
        sqlx::query_as("SELECT status, galat_terakhir FROM sinyal_stok")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(st, "DLQ");
    assert!(galat.unwrap().contains("JENIS_SINYAL_TIDAK_DIKENAL"));
}

#[sqlx::test]
async fn pengiriman_gagal_tidak_mengganggu_mutasi_stok(db: PgPool) {
    // Outbox: stok tetap berhasil dicatat meskipun TitipO mati; sinyal menunggu di ANTRE.
    let k = Klien::baru(&db).await;
    let s = buat_satu_sinyal(&k).await;
    assert_eq!(s.status, "ANTRE");
    let ada: i64 = sqlx::query_scalar("SELECT count(*) FROM mutasi_stok WHERE jenis = 'PAKAI'")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(ada, 1);
}

#[test]
fn tujuan_hanya_ada_bila_url_dan_service_key_dikonfigurasi() {
    let mut c = dukung::config_uji();
    c.titipo_base_url = None;
    assert!(Tujuan::dari_config(&c).is_none());
    c.titipo_base_url = Some("https://api.titipo.chefgenie.id/".to_owned());
    c.service_key = None;
    assert!(Tujuan::dari_config(&c).is_none());
    c.service_key = Some(Rahasia::baru("rahasia"));
    let t = Tujuan::dari_config(&c).unwrap();
    assert_eq!(
        t.base_url, "https://api.titipo.chefgenie.id",
        "garis miring akhir dibuang"
    );
}

#[test]
fn tidak_ada_kode_yang_memanggil_endpoint_pesanan_titipo() {
    // Sinyal hanya ANJURAN: satu-satunya jalur keluar ke TitipO adalah /sinyal/pedaree-stok.
    fn periksa(dir: &std::path::Path) {
        for entri in std::fs::read_dir(dir).unwrap() {
            let jalur = entri.unwrap().path();
            if jalur.is_dir() {
                periksa(&jalur);
            } else if jalur.extension().is_some_and(|e| e == "rs") {
                let isi = std::fs::read_to_string(&jalur).unwrap();
                assert!(
                    !isi.contains("\"/pesanan"),
                    "{} memuat jalur pesanan",
                    jalur.display()
                );
                assert!(
                    !isi.contains("/pesanan/"),
                    "{} memuat jalur pesanan",
                    jalur.display()
                );
            }
        }
    }
    periksa(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"));
    assert_eq!(pengirim::PATH_PENERIMA, "/sinyal/pedaree-stok");
}
