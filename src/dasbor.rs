//! `GET /web/dashboard` (PDR-BE-006): ringkasan dasbor web.
//!
//! Metrik diturunkan dari `Pedaree-TownHall/kelp/metrik/10-stockout-dan-waste.md`. ASUMSI (ditandai
//! karena FSD/FRD v1.0.0 masih templat):
//! - Tidak ada tabel riwayat stok harian; stok historis diturunkan dari buku mutasi: saldo item pada
//!   akhir hari WIB = jumlah seluruh `mutasi_stok` s.d. hari itu pada batch yang belum lewat
//!   tanggal kedaluwarsa pada hari itu. Item dianggap aktif sejak tanggal dibuat (arsip historis
//!   tidak diketahui, jadi hanya item yang kini tidak diarsip yang dihitung).
//! - `stockoutRate` harian = 100 x item HABIS / item aktif (0 bila tidak ada item aktif).
//! - Waste = jumlah `BUANG` dalam gram; satuan volume dihitung ml = g, satuan hitung memakai bobot
//!   acuan; baris yang tidak dapat dikonversi dilewati. Periode: bulan berjalan + 2 bulan sebelumnya.
//! - "Paling sering habis" = jumlah episode HABIS (hari HABIS yang didahului hari tidak-HABIS atau
//!   awal jendela) dalam 90 hari, lalu jumlah hari HABIS.
//! - `segeraKedaluwarsa3Hari` = batch berisi dengan kedaluwarsa 0..3 hari dari hari ini.

use std::collections::BTreeMap;

use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use serde::Serialize;

use crate::auth::Pengguna;
use crate::error::AppError;
use crate::model::{status_stok, StatusStok};
use crate::satuan::{dari_simpan, tampil, Jenis, Satuan};
use crate::state::AppState;
use crate::waktu::hari_ini_wib;

/// Jendela stockout dalam hari.
pub const HARI_JENDELA: i64 = 90;
const ITEM_TERATAS: usize = 10;

// ---------------------------------------------------------------------------
// Autentikasi: service key (SSR web) atau token pengguna.

/// Rumah tangga pemanggil: lewat `X-Service-Key` + `X-Household-Id` (SSR server-ke-server),
/// atau lewat Bearer JWT pengguna seperti endpoint lain.
pub struct RumahTangga(pub String);

fn sama_konstan(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl FromRequestParts<AppState> for RumahTangga {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let Some(kunci) = parts.headers.get("x-service-key") else {
            return Pengguna::from_request_parts(parts, state)
                .await
                .map(|p| Self(p.household_id));
        };
        let benar = state
            .config
            .service_key
            .as_ref()
            .is_some_and(|k| sama_konstan(k.nilai().as_bytes(), kunci.as_bytes()));
        if !benar {
            return Err(AppError::tidak_sah("Service key tidak sah"));
        }
        let household = parts
            .headers
            .get("x-household-id")
            .and_then(|v| v.to_str().ok())
            .filter(|h| h.starts_with("hh_") && h.len() > 3)
            .ok_or_else(|| {
                AppError::validasi("Header X-Household-Id wajib", Some("X-Household-Id"))
            })?;
        Ok(Self(household.to_owned()))
    }
}

// ---------------------------------------------------------------------------
// Bentuk respons

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TitikStockout {
    pub tanggal: NaiveDate,
    pub stockout_rate: f64,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WasteKategori {
    /// `YYYY-MM` (WIB).
    pub bulan: String,
    pub kategori: String,
    pub waste_gram: f64,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ItemSeringHabis {
    pub pantry_item_id: String,
    pub nama: String,
    pub kategori: String,
    pub kejadian_habis: i64,
    pub hari_habis: i64,
    pub stok_minimum: f64,
    pub satuan_tampil: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DasborWeb {
    pub total_item_aktif: i64,
    pub menipis: i64,
    pub habis: i64,
    pub segera_kedaluwarsa3_hari: i64,
    pub waste_bulan_gram: f64,
    pub sinyal_terkirim7_hari: i64,
    #[serde(rename = "stockoutRate90Hari")]
    pub stockout_rate_90_hari: Vec<TitikStockout>,
    #[serde(rename = "stockoutRateRata90Hari")]
    pub stockout_rate_rata_90_hari: f64,
    pub waste_per_kategori: Vec<WasteKategori>,
    pub item_sering_habis: Vec<ItemSeringHabis>,
}

// ---------------------------------------------------------------------------
// Perhitungan murni (diuji unit)

/// Satu baris (item, hari) untuk item yang aktif pada hari itu.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct BarisSeri {
    pub hari: NaiveDate,
    pub id: String,
    pub nama: String,
    pub kategori: String,
    pub satuan_simpan: String,
    pub satuan_tampil: String,
    pub stok_minimum_x10: i64,
    pub saldo_x10: i64,
}

fn bulat1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// Deret stockout harian dan peringkat item sering habis. `baris` harus terurut (id, hari).
pub fn ringkas_seri(
    baris: &[BarisSeri],
    awal: NaiveDate,
    akhir: NaiveDate,
) -> (Vec<TitikStockout>, Vec<ItemSeringHabis>) {
    let mut aktif: BTreeMap<NaiveDate, (i64, i64)> = BTreeMap::new(); // hari -> (aktif, habis)
    let mut d = awal;
    while d <= akhir {
        aktif.insert(d, (0, 0));
        d += Duration::days(1);
    }
    struct Akum {
        nama: String,
        kategori: String,
        simpan: String,
        tampil: String,
        minimum_x10: i64,
        kejadian: i64,
        hari: i64,
        sebelumnya_habis: bool,
    }
    let mut per_item: BTreeMap<String, Akum> = BTreeMap::new();
    for b in baris {
        let habis = b.saldo_x10 <= 0;
        if let Some(t) = aktif.get_mut(&b.hari) {
            t.0 += 1;
            t.1 += i64::from(habis);
        }
        let a = per_item.entry(b.id.clone()).or_insert_with(|| Akum {
            nama: b.nama.clone(),
            kategori: b.kategori.clone(),
            simpan: b.satuan_simpan.clone(),
            tampil: b.satuan_tampil.clone(),
            minimum_x10: b.stok_minimum_x10,
            kejadian: 0,
            hari: 0,
            sebelumnya_habis: false,
        });
        if habis {
            a.hari += 1;
            if !a.sebelumnya_habis {
                a.kejadian += 1;
            }
        }
        a.sebelumnya_habis = habis;
    }
    let deret = aktif
        .into_iter()
        .map(|(tanggal, (a, h))| TitikStockout {
            tanggal,
            stockout_rate: if a == 0 {
                0.0
            } else {
                bulat1(100.0 * h as f64 / a as f64)
            },
        })
        .collect();
    let mut item: Vec<(String, Akum)> = per_item
        .into_iter()
        .filter(|(_, a)| a.kejadian > 0)
        .collect();
    item.sort_by(|(_, x), (_, y)| {
        y.kejadian
            .cmp(&x.kejadian)
            .then(y.hari.cmp(&x.hari))
            .then(x.nama.to_lowercase().cmp(&y.nama.to_lowercase()))
    });
    let teratas = item
        .into_iter()
        .take(ITEM_TERATAS)
        .map(|(id, a)| {
            let (min, unit) = match (Satuan::parse(&a.simpan), Satuan::parse(&a.tampil)) {
                (Some(s), Some(t)) => tampil(&a.nama, s, a.minimum_x10, t),
                _ => (a.minimum_x10 as f64 / 10.0, Satuan::Gram),
            };
            ItemSeringHabis {
                pantry_item_id: id,
                nama: a.nama,
                kategori: a.kategori,
                kejadian_habis: a.kejadian,
                hari_habis: a.hari,
                stok_minimum: min,
                satuan_tampil: unit.as_str().to_owned(),
            }
        })
        .collect();
    (deret, teratas)
}

/// Satu mutasi BUANG (jumlah_x10 negatif) untuk dijumlahkan sebagai waste.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct BarisBuang {
    pub bulan: String,
    pub kategori: String,
    pub nama: String,
    pub satuan_simpan: String,
    pub jumlah_x10: i64,
}

/// Gram dari jumlah simpan (x10); ml dihitung 1:1; `None` bila tak dapat dikonversi.
pub fn gram_dari_simpan(nama: &str, satuan_simpan: &str, x10: i64) -> Option<f64> {
    let simpan = Satuan::parse(satuan_simpan)?;
    match simpan.kanonis().jenis() {
        Jenis::Volume => Some(x10 as f64 / 10.0),
        _ => dari_simpan(nama, simpan, x10, Satuan::Gram),
    }
}

/// Waste per (bulan, kategori) dalam gram, terurut bulan lalu kategori.
pub fn waste_per_kategori(baris: &[BarisBuang]) -> Vec<WasteKategori> {
    let mut jumlah: BTreeMap<(String, String), f64> = BTreeMap::new();
    for b in baris {
        if let Some(g) = gram_dari_simpan(&b.nama, &b.satuan_simpan, b.jumlah_x10.abs()) {
            *jumlah
                .entry((b.bulan.clone(), b.kategori.clone()))
                .or_default() += g;
        }
    }
    jumlah
        .into_iter()
        .map(|((bulan, kategori), g)| WasteKategori {
            bulan,
            kategori,
            waste_gram: bulat1(g),
        })
        .collect()
}

/// Tanggal 1 bulan, `mundur` bulan sebelum `hari`.
fn awal_bulan_mundur(hari: NaiveDate, mundur: u32) -> NaiveDate {
    let total = hari.year() * 12 + hari.month0() as i32 - mundur as i32;
    NaiveDate::from_ymd_opt(total.div_euclid(12), total.rem_euclid(12) as u32 + 1, 1).unwrap()
}

// ---------------------------------------------------------------------------
// Handler

#[derive(sqlx::FromRow)]
struct BarisStok {
    stok: i64,
    stok_minimum_x10: i64,
}

async fn dasbor(
    State(state): State<AppState>,
    RumahTangga(household): RumahTangga,
) -> Result<Json<DasborWeb>, AppError> {
    let sekarang: DateTime<Utc> = (state.jam)();
    let hari_ini = hari_ini_wib(sekarang);
    let awal_seri = hari_ini - Duration::days(HARI_JENDELA - 1);
    let awal_waste = awal_bulan_mundur(hari_ini, 2);
    let bulan_ini = format!("{:04}-{:02}", hari_ini.year(), hari_ini.month());

    // Status stok terkini: definisi sama dengan GET /pantry/items.
    let stok: Vec<BarisStok> = sqlx::query_as(
        "SELECT COALESCE(SUM(b.jumlah_aktif_x10), 0)::bigint AS stok, i.stok_minimum_x10
           FROM pantry_items i
           LEFT JOIN stok_batch b ON b.pantry_item_id = i.id AND b.status = 'AKTIF'
                AND b.jumlah_aktif_x10 > 0
                AND (b.tanggal_kedaluwarsa IS NULL OR b.tanggal_kedaluwarsa >= $2)
          WHERE i.household_id = $1 AND NOT i.arsip
          GROUP BY i.id",
    )
    .bind(&household)
    .bind(hari_ini)
    .fetch_all(&state.db)
    .await?;
    let (mut menipis, mut habis) = (0, 0);
    for s in &stok {
        match status_stok(s.stok, s.stok_minimum_x10) {
            StatusStok::Habis => habis += 1,
            StatusStok::Menipis => menipis += 1,
            StatusStok::Aman => {}
        }
    }

    let segera: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM stok_batch b JOIN pantry_items i ON i.id = b.pantry_item_id
          WHERE b.household_id = $1 AND NOT i.arsip AND b.status = 'AKTIF'
            AND b.jumlah_aktif_x10 > 0 AND b.tanggal_kedaluwarsa BETWEEN $2 AND $3",
    )
    .bind(&household)
    .bind(hari_ini)
    .bind(hari_ini + Duration::days(3))
    .fetch_one(&state.db)
    .await?;

    let sinyal: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sinyal_stok
          WHERE household_id = $1 AND status = 'TERKIRIM' AND dikirim_pada >= $2",
    )
    .bind(&household)
    .bind(sekarang - Duration::days(7))
    .fetch_one(&state.db)
    .await?;

    let buang: Vec<BarisBuang> = sqlx::query_as(
        "SELECT to_char(m.terjadi_pada AT TIME ZONE 'Asia/Jakarta', 'YYYY-MM') AS bulan,
                i.kategori, i.nama, i.satuan_simpan, m.jumlah_x10
           FROM mutasi_stok m
           JOIN stok_batch b ON b.id = m.batch_id
           JOIN pantry_items i ON i.id = b.pantry_item_id
          WHERE m.household_id = $1 AND m.jenis = 'BUANG'
            AND (m.terjadi_pada AT TIME ZONE 'Asia/Jakarta')::date >= $2",
    )
    .bind(&household)
    .bind(awal_waste)
    .fetch_all(&state.db)
    .await?;
    let waste = waste_per_kategori(&buang);
    let waste_bulan_gram = bulat1(
        waste
            .iter()
            .filter(|w| w.bulan == bulan_ini)
            .map(|w| w.waste_gram)
            .sum(),
    );

    let seri: Vec<BarisSeri> = sqlx::query_as(
        "WITH hari AS (
            SELECT generate_series($2::date, $3::date, '1 day')::date AS hari)
         SELECT h.hari, i.id, i.nama, i.kategori, i.satuan_simpan, i.satuan_tampil,
                i.stok_minimum_x10,
                COALESCE((SELECT SUM(m.jumlah_x10)
                            FROM mutasi_stok m JOIN stok_batch b ON b.id = m.batch_id
                           WHERE b.pantry_item_id = i.id
                             AND (b.tanggal_kedaluwarsa IS NULL OR b.tanggal_kedaluwarsa >= h.hari)
                             AND (m.terjadi_pada AT TIME ZONE 'Asia/Jakarta')::date <= h.hari),
                         0)::bigint AS saldo_x10
           FROM hari h
           JOIN pantry_items i ON i.household_id = $1 AND NOT i.arsip
                AND (i.dibuat_pada AT TIME ZONE 'Asia/Jakarta')::date <= h.hari
          ORDER BY i.id, h.hari",
    )
    .bind(&household)
    .bind(awal_seri)
    .bind(hari_ini)
    .fetch_all(&state.db)
    .await?;
    let (deret, sering) = ringkas_seri(&seri, awal_seri, hari_ini);
    let rata = if deret.is_empty() {
        0.0
    } else {
        bulat1(deret.iter().map(|t| t.stockout_rate).sum::<f64>() / deret.len() as f64)
    };

    Ok(Json(DasborWeb {
        total_item_aktif: stok.len() as i64,
        menipis,
        habis,
        segera_kedaluwarsa3_hari: segera,
        waste_bulan_gram,
        sinyal_terkirim7_hari: sinyal,
        stockout_rate_90_hari: deret,
        stockout_rate_rata_90_hari: rata,
        waste_per_kategori: waste,
        item_sering_habis: sering,
    }))
}

pub fn rute() -> Router<AppState> {
    Router::new().route("/web/dashboard", get(dasbor))
}

#[cfg(test)]
mod uji {
    use super::*;

    fn d(hari: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, hari).unwrap()
    }

    fn baris(id: &str, nama: &str, hari: u32, saldo: i64) -> BarisSeri {
        BarisSeri {
            hari: d(hari),
            id: id.into(),
            nama: nama.into(),
            kategori: "KERING".into(),
            satuan_simpan: "gram".into(),
            satuan_tampil: "gram".into(),
            stok_minimum_x10: 2500,
            saldo_x10: saldo,
        }
    }

    #[test]
    fn deret_stockout_dan_episode_habis() {
        // Beras: habis hari 1-2, ada hari 3, habis lagi hari 4 (2 episode, 3 hari).
        // Gula: ada terus. Gula baru aktif mulai hari 2.
        let data = vec![
            baris("itm_a", "Beras", 1, 0),
            baris("itm_a", "Beras", 2, 0),
            baris("itm_a", "Beras", 3, 500),
            baris("itm_a", "Beras", 4, 0),
            baris("itm_b", "Gula", 2, 900),
            baris("itm_b", "Gula", 3, 900),
            baris("itm_b", "Gula", 4, 900),
        ];
        let (deret, top) = ringkas_seri(&data, d(1), d(4));
        let rate: Vec<f64> = deret.iter().map(|t| t.stockout_rate).collect();
        assert_eq!(rate, vec![100.0, 50.0, 0.0, 50.0]);
        assert_eq!(top.len(), 1, "Gula tidak pernah habis");
        assert_eq!(
            (
                top[0].nama.as_str(),
                top[0].kejadian_habis,
                top[0].hari_habis
            ),
            ("Beras", 2, 3)
        );
        assert_eq!(top[0].stok_minimum, 250.0);
    }

    #[test]
    fn tanpa_item_aktif_rate_nol_dan_top_10_dibatasi() {
        let (deret, top) = ringkas_seri(&[], d(1), d(3));
        assert_eq!(deret.len(), 3);
        assert!(deret.iter().all(|t| t.stockout_rate == 0.0) && top.is_empty());
        let banyak: Vec<BarisSeri> = (0..15)
            .map(|i| baris(&format!("itm_{i:02}"), &format!("Bahan {i:02}"), 1, 0))
            .collect();
        let (_, top) = ringkas_seri(&banyak, d(1), d(1));
        assert_eq!(top.len(), 10);
    }

    #[test]
    fn waste_dikonversi_ke_gram_dan_dikelompokkan() {
        let b = |bulan: &str, kat: &str, nama: &str, satuan: &str, x10: i64| BarisBuang {
            bulan: bulan.into(),
            kategori: kat.into(),
            nama: nama.into(),
            satuan_simpan: satuan.into(),
            jumlah_x10: x10,
        };
        let hasil = waste_per_kategori(&[
            b("2026-10", "SAYUR", "Bayam", "gram", -2000),
            b("2026-10", "SAYUR", "Kangkung", "gram", -1500),
            b("2026-10", "CAIR", "Minyak Goreng", "ml", -5000),
            b("2026-09", "SAYUR", "Bayam", "gram", -1000),
        ]);
        let ringkas: Vec<(&str, &str, f64)> = hasil
            .iter()
            .map(|w| (w.bulan.as_str(), w.kategori.as_str(), w.waste_gram))
            .collect();
        assert_eq!(
            ringkas,
            vec![
                ("2026-09", "SAYUR", 100.0),
                ("2026-10", "CAIR", 500.0),
                ("2026-10", "SAYUR", 350.0),
            ]
        );
    }

    #[test]
    fn awal_bulan_mundur_melewati_tahun() {
        let h = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        assert_eq!(
            awal_bulan_mundur(h, 2),
            NaiveDate::from_ymd_opt(2025, 11, 1).unwrap()
        );
        assert_eq!(
            awal_bulan_mundur(h, 0),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
        );
    }

    #[test]
    fn banding_konstan() {
        assert!(sama_konstan(b"abc", b"abc"));
        assert!(!sama_konstan(b"abc", b"abd"));
        assert!(!sama_konstan(b"abc", b"ab"));
    }
}
