//! Sisi PRODUSEN kontrak bilateral `pedaree.stok.v1` ke TitipO.
//!
//! Pemicu: transisi ke menipis/habis setelah mutasi stok, dan `kedaluwarsa.dekat` dari job
//! `expiry-sweep` harian. Sinyal ditulis ke outbox `sinyal_stok` pada transaksi yang sama
//! dengan mutasi; pengiriman dilakukan `pengirim`. Maksimal 1 sinyal per item PER JENIS per
//! 24 jam (BR-301, ADR-0010 bagian pembaruan); kunci idempoten konsumen `item_id + jenis + tanggal`. Sinyal hanyalah ANJURAN restock: tidak ada kode di repo ini yang membuat
//! pesanan TitipO.

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde_json::json;
use sqlx::{PgConnection, PgPool};

use crate::error::AppError;
use crate::model::{ItemRow, KOLOM_ITEM};
use crate::waktu::hari_ini_wib;

enum_teks!(
    Jenis {
        Menipis => "stok.menipis",
        Habis => "stok.habis",
        KedaluwarsaDekat => "kedaluwarsa.dekat",
    }
);

/// "Kedaluwarsa dekat" = batch yang tersisa 0 sampai 3 hari (SOP: SEGERA dan HARI_INI).
/// ASUMSI: kontrak bilateral tidak menyebut ambang; dipilih tingkat yang butuh tindakan segera.
pub const AMBANG_DEKAT_HARI: i64 = 3;

/// Transisi stok yang memicu sinyal (skala x10, stok yang dapat dipakai).
///
/// - ke habis: sebelumnya > 0 dan sekarang 0;
/// - ke menipis: sebelumnya di atas minimum dan sekarang <= minimum tetapi > 0.
pub fn jenis_transisi(sebelum_x10: i64, sesudah_x10: i64, minimum_x10: i64) -> Option<Jenis> {
    if sebelum_x10 > 0 && sesudah_x10 <= 0 {
        Some(Jenis::Habis)
    } else if sebelum_x10 > minimum_x10 && sesudah_x10 > 0 && sesudah_x10 <= minimum_x10 {
        Some(Jenis::Menipis)
    } else {
        None
    }
}

async fn kedaluwarsa_untuk_payload(
    conn: &mut PgConnection,
    item_id: &str,
    hari_ini: NaiveDate,
) -> Result<NaiveDate, AppError> {
    let terdekat: Option<NaiveDate> = sqlx::query_scalar(
        "SELECT MIN(tanggal_kedaluwarsa) FROM stok_batch
          WHERE pantry_item_id = $1 AND status = 'AKTIF' AND jumlah_aktif_x10 > 0
            AND tanggal_kedaluwarsa >= $2",
    )
    .bind(item_id)
    .bind(hari_ini)
    .fetch_one(&mut *conn)
    .await?;
    if let Some(t) = terdekat {
        return Ok(t);
    }
    // Stok habis atau tanpa tanggal: pakai tanggal kedaluwarsa batch terakhir; kalau tidak ada
    // sama sekali (mis. garam), pakai tanggal sinyal. ASUMSI: payload mewajibkan `kedaluwarsa`.
    let terakhir: Option<NaiveDate> = sqlx::query_scalar(
        "SELECT MAX(tanggal_kedaluwarsa) FROM stok_batch WHERE pantry_item_id = $1",
    )
    .bind(item_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(terakhir.unwrap_or(hari_ini))
}

/// Menulis sinyal ke outbox kecuali sudah ada sinyal item ini dengan JENIS yang sama dalam
/// 24 jam terakhir (menipis dan habis dedup sendiri-sendiri).
/// Mengembalikan `true` bila sinyal dibuat. Harus dipanggil pada transaksi yang sudah
/// menahan kunci baris item (mutasi) atau mengambilnya sendiri (sweep).
pub async fn antrekan(
    conn: &mut PgConnection,
    item: &ItemRow,
    jenis: Jenis,
    sisa_x10: i64,
    kedaluwarsa: NaiveDate,
    recipe_id: Option<&str>,
    sekarang: DateTime<Utc>,
) -> Result<bool, AppError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(format!("sinyal:{}", item.id))
        .execute(&mut *conn)
        .await?;
    let ada: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM sinyal_stok
                         WHERE item_id = $1 AND jenis = $3
                           AND dibuat_pada > $2::timestamptz - interval '24 hours')",
    )
    .bind(&item.id)
    .bind(sekarang)
    .bind(jenis.as_str())
    .fetch_one(&mut *conn)
    .await?;
    if ada {
        return Ok(false);
    }
    let tanggal = hari_ini_wib(sekarang);
    let payload = json!({
        "jenis": jenis.as_str(),
        "item_id": item.id,
        "tanggal": tanggal,
        "item": item.nama,
        "sisa": sisa_x10 as f64 / 10.0,
        "satuan": item.satuan_simpan,
        "kedaluwarsa": kedaluwarsa,
        "recipe_id": recipe_id,
    });
    sqlx::query(
        "INSERT INTO sinyal_stok (id, household_id, item_id, jenis, tanggal, payload, coba_lagi_pada, dibuat_pada)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $7)",
    )
    .bind(format!("sig_{}", ulid::Ulid::new()))
    .bind(&item.household_id)
    .bind(&item.id)
    .bind(jenis.as_str())
    .bind(tanggal)
    .bind(&payload)
    .bind(sekarang)
    .execute(&mut *conn)
    .await?;
    Ok(true)
}

/// Dipanggil setelah mutasi yang mengubah stok yang dapat dipakai (pakai, opname, buang).
pub async fn setelah_mutasi(
    conn: &mut PgConnection,
    item: &ItemRow,
    sebelum_x10: i64,
    sesudah_x10: i64,
    recipe_id: Option<&str>,
    sekarang: DateTime<Utc>,
) -> Result<(), AppError> {
    let Some(jenis) = jenis_transisi(sebelum_x10, sesudah_x10, item.stok_minimum_x10) else {
        return Ok(());
    };
    let kedaluwarsa = kedaluwarsa_untuk_payload(conn, &item.id, hari_ini_wib(sekarang)).await?;
    antrekan(
        conn,
        item,
        jenis,
        sesudah_x10,
        kedaluwarsa,
        recipe_id,
        sekarang,
    )
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// expiry-sweep

#[derive(Debug, sqlx::FromRow)]
struct Kandidat {
    id: String,
    terdekat: NaiveDate,
    sisa: i64,
}

/// Mengevaluasi seluruh item: yang punya batch kedaluwarsa 0..=3 hari menghasilkan
/// `kedaluwarsa.dekat` (bila belum ada sinyal jenis itu untuk item tersebut dalam 24 jam). Mengembalikan
/// jumlah sinyal yang dibuat. `sisa` = jumlah batch yang kedaluwarsa dalam jendela itu.
pub async fn expiry_sweep(db: &PgPool, sekarang: DateTime<Utc>) -> Result<usize, AppError> {
    let hari_ini = hari_ini_wib(sekarang);
    let batas = hari_ini + Duration::days(AMBANG_DEKAT_HARI);
    let kandidat: Vec<Kandidat> = sqlx::query_as(
        "SELECT b.pantry_item_id AS id, MIN(b.tanggal_kedaluwarsa) AS terdekat,
                SUM(b.jumlah_aktif_x10)::bigint AS sisa
           FROM stok_batch b JOIN pantry_items i ON i.id = b.pantry_item_id
          WHERE NOT i.arsip AND b.status = 'AKTIF' AND b.jumlah_aktif_x10 > 0
            AND b.tanggal_kedaluwarsa BETWEEN $1 AND $2
          GROUP BY b.pantry_item_id
          ORDER BY b.pantry_item_id",
    )
    .bind(hari_ini)
    .bind(batas)
    .fetch_all(db)
    .await?;

    let mut dibuat = 0;
    for k in kandidat {
        let mut tx = db.begin().await?;
        let sql =
            format!("SELECT {KOLOM_ITEM} FROM pantry_items WHERE id = $1 AND NOT arsip FOR UPDATE");
        let Some(item) = sqlx::query_as::<_, ItemRow>(&sql)
            .bind(&k.id)
            .fetch_optional(&mut *tx)
            .await?
        else {
            continue;
        };
        if antrekan(
            &mut tx,
            &item,
            Jenis::KedaluwarsaDekat,
            k.sisa,
            k.terdekat,
            None,
            sekarang,
        )
        .await?
        {
            dibuat += 1;
        }
        tx.commit().await?;
    }
    Ok(dibuat)
}

/// Mengklaim sweep hari WIB `sekarang` lalu menjalankannya. `None` bila hari itu sudah
/// diklaim (instans lain atau eksekusi ulang): aman dipanggil berulang.
pub async fn jalankan_sweep_harian(
    db: &PgPool,
    sekarang: DateTime<Utc>,
) -> Result<Option<usize>, AppError> {
    let hari = hari_ini_wib(sekarang);
    let klaim = sqlx::query(
        "INSERT INTO expiry_sweep_log (tanggal, dimulai_pada) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(hari)
    .bind(sekarang)
    .execute(db)
    .await?;
    if klaim.rows_affected() == 0 {
        return Ok(None);
    }
    let dibuat = expiry_sweep(db, sekarang).await?;
    sqlx::query(
        "UPDATE expiry_sweep_log SET selesai_pada = now(), sinyal_dibuat = $2 WHERE tanggal = $1",
    )
    .bind(hari)
    .bind(dibuat as i32)
    .execute(db)
    .await?;
    Ok(Some(dibuat))
}

/// Jam sweep: 06:00 WIB = 23:00 UTC hari sebelumnya.
const JAM_SWEEP_WIB: i64 = 6;

/// Detik dari `sekarang` sampai 06:00 WIB berikutnya (tepat 06:00 berarti besok).
pub fn detik_sampai_sweep(sekarang: DateTime<Utc>) -> u64 {
    let wib = sekarang + Duration::hours(7);
    let hari_ini_pukul_enam = wib
        .date_naive()
        .and_hms_opt(JAM_SWEEP_WIB as u32, 0, 0)
        .expect("jam valid")
        .and_utc();
    let target = if wib < hari_ini_pukul_enam {
        hari_ini_pukul_enam
    } else {
        hari_ini_pukul_enam + Duration::days(1)
    };
    (target - wib).num_seconds().max(1) as u64
}

/// Apakah sudah lewat 06:00 WIB pada hari `sekarang` (untuk menyusul sweep yang terlewat).
pub fn sudah_lewat_jam_sweep(sekarang: DateTime<Utc>) -> bool {
    let wib = sekarang + Duration::hours(7);
    wib.time() >= chrono::NaiveTime::from_hms_opt(JAM_SWEEP_WIB as u32, 0, 0).expect("jam valid")
}

#[cfg(test)]
mod uji {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn transisi_menipis_dan_habis() {
        // aman -> menipis
        assert_eq!(jenis_transisi(5000, 2500, 2500), Some(Jenis::Menipis));
        assert_eq!(jenis_transisi(5000, 2400, 2500), Some(Jenis::Menipis));
        // aman -> habis langsung: hanya habis
        assert_eq!(jenis_transisi(5000, 0, 2500), Some(Jenis::Habis));
        // menipis -> habis
        assert_eq!(jenis_transisi(2000, 0, 2500), Some(Jenis::Habis));
        // sudah menipis dan tetap menipis: bukan transisi baru
        assert_eq!(jenis_transisi(2400, 1000, 2500), None);
        // tetap aman
        assert_eq!(jenis_transisi(5000, 2600, 2500), None);
        // naik tidak memicu
        assert_eq!(jenis_transisi(0, 3000, 2500), None);
        assert_eq!(jenis_transisi(1000, 6000, 2500), None);
        // minimum 0: tidak pernah menipis, hanya habis
        assert_eq!(jenis_transisi(5000, 10, 0), None);
        assert_eq!(jenis_transisi(5000, 0, 0), Some(Jenis::Habis));
        // sudah nol dan tetap nol
        assert_eq!(jenis_transisi(0, 0, 2500), None);
    }

    #[test]
    fn tiga_jenis_terkunci_sesuai_kontrak() {
        assert_eq!(Jenis::Menipis.as_str(), "stok.menipis");
        assert_eq!(Jenis::Habis.as_str(), "stok.habis");
        assert_eq!(Jenis::KedaluwarsaDekat.as_str(), "kedaluwarsa.dekat");
        assert!(Jenis::parse("stok.melimpah").is_none());
    }

    #[test]
    fn jadwal_sweep_pukul_0600_wib() {
        // 05:59 WIB (22:59 UTC hari sebelumnya) -> 60 detik lagi
        let t = Utc.with_ymd_and_hms(2026, 10, 20, 22, 59, 0).unwrap();
        assert_eq!(detik_sampai_sweep(t), 60);
        assert!(!sudah_lewat_jam_sweep(t));
        // tepat 06:00 WIB (23:00 UTC) -> besok (86.400 detik)
        let t = Utc.with_ymd_and_hms(2026, 10, 20, 23, 0, 0).unwrap();
        assert_eq!(detik_sampai_sweep(t), 86_400);
        assert!(sudah_lewat_jam_sweep(t));
        // 12:00 WIB (05:00 UTC) -> 18 jam lagi
        let t = Utc.with_ymd_and_hms(2026, 10, 21, 5, 0, 0).unwrap();
        assert_eq!(detik_sampai_sweep(t), 18 * 3600);
        // 00:30 WIB (17:30 UTC) -> 5,5 jam lagi
        let t = Utc.with_ymd_and_hms(2026, 10, 20, 17, 30, 0).unwrap();
        assert_eq!(detik_sampai_sweep(t), 5 * 3600 + 1800);
    }
}
