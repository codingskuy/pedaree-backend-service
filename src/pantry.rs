//! Endpoint stok `/pantry/*`: item, batch, pakai (FEFO), opname, pindah, buang, kedaluwarsa.
//!
//! Aturan inti (AGENTS.md): FEFO di server; setiap perubahan `jumlah_aktif` menulis satu
//! `mutasi_stok` dalam transaksi yang sama; semua POST idempoten lewat `X-Request-Id`.

use std::collections::HashSet;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, Postgres, Transaction};

use crate::auth::Pengguna;
use crate::error::{AppError, JsonBaku, PathBaku, QueryBaku};
use crate::idempotensi::{self, IdPermintaan, Kunci};
use crate::model::{
    status_kedaluwarsa, status_stok, AlasanBuang, BatchRow, ItemRow, Kategori, Lokasi, MutasiRow,
    MutasiStok, PantryItem, StatusStok, StokBatch, Sumber, KOLOM_BATCH, KOLOM_ITEM, KOLOM_MUTASI,
};
use crate::satuan::{
    self, dapat_dikonversi, ke_simpan_x10, periksa_batas, satuan_sah_untuk, tampil, Satuan,
};
use crate::state::AppState;
use crate::waktu::hari_ini_wib;

type Tx = Transaction<'static, Postgres>;

/// Konteks satu permintaan bermutasi.
struct Konteks {
    household: String,
    aktor: String,
    sekarang: DateTime<Utc>,
    hari_ini: NaiveDate,
}

impl Konteks {
    fn baru(state: &AppState, p: &Pengguna) -> Self {
        let sekarang = (state.jam)();
        Self {
            household: p.household_id.clone(),
            aktor: p.id.clone(),
            sekarang,
            hari_ini: hari_ini_wib(sekarang),
        }
    }
}

fn id_baru(prefiks: &str) -> String {
    format!("{prefiks}_{}", ulid::Ulid::new())
}

/// Menyimpan hasil idempotensi, melakukan commit, lalu membalas.
async fn selesai<T: Serialize>(
    mut tx: Tx,
    kunci: &Kunci,
    status: StatusCode,
    hasil: &T,
) -> Result<Response, AppError> {
    let badan = serde_json::to_value(hasil)
        .map_err(|e| AppError::internal(format!("serialisasi respons: {e}")))?;
    idempotensi::simpan(&mut tx, kunci, status, &badan).await?;
    tx.commit().await?;
    Ok((status, Json(badan)).into_response())
}

/// Memulai transaksi idempoten. `Ok(Err(respons))` = hasil tersimpan, kembalikan apa adanya.
async fn mulai_idempoten(
    state: &AppState,
    kunci: &Kunci,
) -> Result<Result<Tx, Response>, AppError> {
    let mut tx = state.db.begin().await?;
    match idempotensi::mulai(&mut tx, kunci).await? {
        Some((status, badan)) => Ok(Err((status, Json(badan)).into_response())),
        None => Ok(Ok(tx)),
    }
}

// ---------------------------------------------------------------------------
// Akses data

async fn ambil_item(
    conn: &mut PgConnection,
    household: &str,
    id: &str,
    kunci: bool,
) -> Result<ItemRow, AppError> {
    let sql = format!(
        "SELECT {KOLOM_ITEM} FROM pantry_items WHERE id = $1 AND household_id = $2 AND NOT arsip{}",
        if kunci { " FOR UPDATE" } else { "" }
    );
    sqlx::query_as::<_, ItemRow>(&sql)
        .bind(id)
        .bind(household)
        .fetch_optional(conn)
        .await?
        .ok_or_else(|| AppError::tidak_ada("Item tidak ditemukan"))
}

/// Batch yang boleh dipakai, urutan FEFO: kedaluwarsa terdekat dulu; tanpa tanggal (null)
/// terakhir dan urut tertua (FIFO). Batch KEDALUWARSA terkunci dari FEFO. Baris dikunci.
async fn batch_fefo(
    conn: &mut PgConnection,
    item_id: &str,
    hari_ini: NaiveDate,
) -> Result<Vec<BatchRow>, AppError> {
    let sql = format!(
        "SELECT {KOLOM_BATCH} FROM stok_batch
          WHERE pantry_item_id = $1 AND status = 'AKTIF' AND jumlah_aktif_x10 > 0
            AND (tanggal_kedaluwarsa IS NULL OR tanggal_kedaluwarsa >= $2)
          ORDER BY tanggal_kedaluwarsa ASC NULLS LAST, tanggal_masuk ASC, dibuat_pada ASC, id ASC
          FOR UPDATE"
    );
    Ok(sqlx::query_as::<_, BatchRow>(&sql)
        .bind(item_id)
        .bind(hari_ini)
        .fetch_all(conn)
        .await?)
}

async fn ambil_batch(
    conn: &mut PgConnection,
    household: &str,
    id: &str,
    kunci: bool,
) -> Result<BatchRow, AppError> {
    let sql = format!(
        "SELECT {KOLOM_BATCH} FROM stok_batch WHERE id = $1 AND household_id = $2{}",
        if kunci { " FOR UPDATE" } else { "" }
    );
    sqlx::query_as::<_, BatchRow>(&sql)
        .bind(id)
        .bind(household)
        .fetch_optional(conn)
        .await?
        .ok_or_else(|| AppError::tidak_ada("Batch tidak ditemukan"))
}

#[allow(clippy::too_many_arguments)]
async fn tulis_mutasi(
    conn: &mut PgConnection,
    ctx: &Konteks,
    batch_id: &str,
    jenis: &str,
    jumlah_x10: i64,
    sisa_x10: i64,
    alasan: &str,
    resep: Option<&str>,
) -> Result<MutasiRow, AppError> {
    let sql = format!(
        "INSERT INTO mutasi_stok (id, household_id, batch_id, jenis, jumlah_x10, sisa_setelah_x10,
                                  alasan, aktor, sumber_resep_id, terjadi_pada)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         RETURNING {KOLOM_MUTASI}"
    );
    Ok(sqlx::query_as::<_, MutasiRow>(&sql)
        .bind(id_baru("mut"))
        .bind(&ctx.household)
        .bind(batch_id)
        .bind(jenis)
        .bind(jumlah_x10)
        .bind(sisa_x10)
        .bind(alasan)
        .bind(&ctx.aktor)
        .bind(resep)
        .bind(ctx.sekarang)
        .fetch_one(conn)
        .await?)
}

/// Mengurangi `ambil_x10` dari satu batch dan menulis satu mutasi (jenis PAKAI/KOREKSI/BUANG).
#[allow(clippy::too_many_arguments)]
async fn kurangi_batch(
    conn: &mut PgConnection,
    ctx: &Konteks,
    batch: &BatchRow,
    ambil_x10: i64,
    jenis: &str,
    alasan: &str,
    resep: Option<&str>,
    status_nol: &str,
) -> Result<MutasiRow, AppError> {
    let sisa = batch.jumlah_aktif_x10 - ambil_x10;
    debug_assert!(sisa >= 0 && ambil_x10 > 0);
    let status = if sisa == 0 { status_nol } else { "AKTIF" };
    sqlx::query("UPDATE stok_batch SET jumlah_aktif_x10 = $2, status = $3 WHERE id = $1")
        .bind(&batch.id)
        .bind(sisa)
        .bind(status)
        .execute(&mut *conn)
        .await?;
    tulis_mutasi(conn, ctx, &batch.id, jenis, -ambil_x10, sisa, alasan, resep).await
}

#[allow(clippy::too_many_arguments)]
async fn buat_batch(
    conn: &mut PgConnection,
    ctx: &Konteks,
    item: &ItemRow,
    jumlah_x10: i64,
    tanggal_masuk: NaiveDate,
    tanggal_kedaluwarsa: Option<NaiveDate>,
    lokasi: &str,
    sumber: &str,
) -> Result<BatchRow, AppError> {
    let sql = format!(
        "INSERT INTO stok_batch (id, household_id, pantry_item_id, jumlah_awal_x10, jumlah_aktif_x10,
                                 tanggal_masuk, tanggal_kedaluwarsa, lokasi, sumber, status, dibuat_pada)
         VALUES ($1, $2, $3, $4, $4, $5, $6, $7, $8, 'AKTIF', $9)
         RETURNING {KOLOM_BATCH}"
    );
    Ok(sqlx::query_as::<_, BatchRow>(&sql)
        .bind(id_baru("btc"))
        .bind(&ctx.household)
        .bind(&item.id)
        .bind(jumlah_x10)
        .bind(tanggal_masuk)
        .bind(tanggal_kedaluwarsa)
        .bind(lokasi)
        .bind(sumber)
        .bind(ctx.sekarang)
        .fetch_one(conn)
        .await?)
}

/// Stok yang dapat dipakai (AKTIF dan belum kedaluwarsa), skala x10.
async fn stok_dapat_dipakai(
    conn: &mut PgConnection,
    item_id: &str,
    hari_ini: NaiveDate,
) -> Result<i64, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT COALESCE(SUM(jumlah_aktif_x10), 0)::bigint FROM stok_batch
          WHERE pantry_item_id = $1 AND status = 'AKTIF' AND jumlah_aktif_x10 > 0
            AND (tanggal_kedaluwarsa IS NULL OR tanggal_kedaluwarsa >= $2)",
    )
    .bind(item_id)
    .bind(hari_ini)
    .fetch_one(conn)
    .await?)
}

fn alasan_valid(teks: &str, field: &str) -> Result<String, AppError> {
    let t = teks.trim();
    let n = t.chars().count();
    if !(3..=140).contains(&n) {
        return Err(AppError::validasi(
            "alasan harus 3 sampai 140 karakter",
            Some(field),
        ));
    }
    Ok(t.to_owned())
}

// ---------------------------------------------------------------------------
// POST /pantry/items

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PantryItemBuat {
    nama: String,
    kategori: Kategori,
    satuan_simpan: Satuan,
    satuan_tampil: Satuan,
    stok_minimum: f64,
    umur_simpan_hari_default: i32,
    lokasi_default: Lokasi,
    foto_url: Option<String>,
}

async fn buat_item(
    State(state): State<AppState>,
    pengguna: Pengguna,
    rid: IdPermintaan,
    JsonBaku(req): JsonBaku<PantryItemBuat>,
) -> Result<Response, AppError> {
    let ctx = Konteks::baru(&state, &pengguna);
    let nama = req.nama.trim().to_owned();
    if !(3..=60).contains(&nama.chars().count()) {
        return Err(AppError::validasi(
            "nama harus 3 sampai 60 karakter",
            Some("nama"),
        ));
    }
    if !(0..=365).contains(&req.umur_simpan_hari_default) {
        return Err(AppError::validasi(
            "umurSimpanHariDefault harus 0 sampai 365",
            Some("umurSimpanHariDefault"),
        ));
    }
    if let Some(url) = &req.foto_url {
        if !url.starts_with("https://") || url.len() > 500 {
            return Err(AppError::validasi(
                "fotoUrl harus URL https",
                Some("fotoUrl"),
            ));
        }
    }
    if !req.stok_minimum.is_finite() || req.stok_minimum < 0.0 {
        return Err(AppError::validasi(
            "stokMinimum tidak boleh negatif",
            Some("stokMinimum"),
        ));
    }
    if !satuan_sah_untuk(&nama, req.satuan_simpan) {
        return Err(satuan::GalatSatuan::TidakBerlaku.ke_app("satuanSimpan"));
    }
    let simpan = req.satuan_simpan.kanonis();
    if !satuan_sah_untuk(&nama, req.satuan_tampil)
        || !dapat_dikonversi(&nama, simpan, req.satuan_tampil)
    {
        return Err(satuan::GalatSatuan::TidakBerlaku.ke_app("satuanTampil"));
    }
    // stokMinimum diberikan dalam satuanSimpan sebagaimana diminta, disimpan dalam kanonis.
    let minimum_x10 = if req.stok_minimum == 0.0 {
        0
    } else {
        ke_simpan_x10(&nama, simpan, req.stok_minimum, req.satuan_simpan)
            .map_err(|e| e.ke_app("stokMinimum"))?
    };

    let kunci = Kunci::baru(&ctx.household, rid, "POST /pantry/items");
    let mut tx = match mulai_idempoten(&state, &kunci).await? {
        Ok(tx) => tx,
        Err(tersimpan) => return Ok(tersimpan),
    };
    let sql = format!(
        "INSERT INTO pantry_items (id, household_id, nama, kategori, satuan_simpan, satuan_tampil,
            stok_minimum_x10, umur_simpan_hari_default, lokasi_default, foto_url, dibuat_pada, diubah_pada)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11)
         RETURNING {KOLOM_ITEM}"
    );
    let baris = sqlx::query_as::<_, ItemRow>(&sql)
        .bind(id_baru("itm"))
        .bind(&ctx.household)
        .bind(&nama)
        .bind(req.kategori.as_str())
        .bind(simpan.as_str())
        .bind(req.satuan_tampil.as_str())
        .bind(minimum_x10)
        .bind(req.umur_simpan_hari_default)
        .bind(req.lokasi_default.as_str())
        .bind(&req.foto_url)
        .bind(ctx.sekarang)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| match AppError::from(e) {
            AppError::Konflik(_) => AppError::konflik("Item dengan nama ini sudah ada"),
            lain => lain,
        })?;
    selesai(tx, &kunci, StatusCode::CREATED, &PantryItem::from(&baris)).await
}

// ---------------------------------------------------------------------------
// GET /pantry/items

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueryDaftar {
    q: Option<String>,
    kategori: Option<Kategori>,
    status_stok: Option<StatusStok>,
    limit: Option<i64>,
    cursor: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct BarisRingkas {
    id: String,
    nama: String,
    nama_lower: String,
    kategori: String,
    satuan_simpan: String,
    satuan_tampil: String,
    lokasi_default: String,
    stok_minimum_x10: i64,
    stok: i64,
    terdekat: Option<NaiveDate>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ItemRingkas {
    id: String,
    nama: String,
    kategori: String,
    stok_tampil: f64,
    satuan_tampil: String,
    status_stok: String,
    batch_terdekat_kedaluwarsa: Option<NaiveDate>,
    lokasi_default: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HalamanItem {
    data: Vec<ItemRingkas>,
    next_cursor: Option<String>,
}

fn kode_kursor(nama_lower: &str, id: &str) -> String {
    URL_SAFE_NO_PAD.encode(format!("{nama_lower}\u{1f}{id}"))
}

fn baca_kursor(kursor: &str) -> Result<(String, String), AppError> {
    let salah = || AppError::validasi("cursor tidak valid", Some("cursor"));
    let mentah = URL_SAFE_NO_PAD.decode(kursor).map_err(|_| salah())?;
    let teks = String::from_utf8(mentah).map_err(|_| salah())?;
    let (nama, id) = teks.split_once('\u{1f}').ok_or_else(salah)?;
    Ok((nama.to_owned(), id.to_owned()))
}

async fn daftar_item(
    State(state): State<AppState>,
    pengguna: Pengguna,
    QueryBaku(q): QueryBaku<QueryDaftar>,
) -> Result<Json<HalamanItem>, AppError> {
    let limit = q.limit.unwrap_or(20);
    if !(1..=100).contains(&limit) {
        return Err(AppError::validasi(
            "limit harus 1 sampai 100",
            Some("limit"),
        ));
    }
    let (kursor_nama, kursor_id) = match q.cursor.as_deref() {
        Some(c) => {
            let (n, i) = baca_kursor(c)?;
            (Some(n), Some(i))
        }
        None => (None, None),
    };
    let hari_ini = hari_ini_wib((state.jam)());
    let baris: Vec<BarisRingkas> = sqlx::query_as(
        "WITH agg AS (
            SELECT i.id, i.nama, lower(i.nama) AS nama_lower, i.kategori, i.satuan_simpan,
                   i.satuan_tampil, i.lokasi_default, i.stok_minimum_x10,
                   COALESCE(SUM(b.jumlah_aktif_x10), 0)::bigint AS stok,
                   MIN(b.tanggal_kedaluwarsa) AS terdekat
              FROM pantry_items i
              LEFT JOIN stok_batch b ON b.pantry_item_id = i.id AND b.status = 'AKTIF'
                   AND b.jumlah_aktif_x10 > 0
                   AND (b.tanggal_kedaluwarsa IS NULL OR b.tanggal_kedaluwarsa >= $2)
             WHERE i.household_id = $1 AND NOT i.arsip
               AND ($3::text IS NULL OR strpos(lower(i.nama), lower($3)) > 0)
               AND ($4::text IS NULL OR i.kategori = $4)
             GROUP BY i.id)
         SELECT * FROM agg
          WHERE ($5::text IS NULL OR CASE WHEN stok <= 0 THEN 'HABIS'
                                          WHEN stok <= stok_minimum_x10 THEN 'MENIPIS'
                                          ELSE 'AMAN' END = $5)
            AND ($6::text IS NULL OR (nama_lower, id) > ($6::text, $7::text))
          ORDER BY nama_lower, id
          LIMIT $8",
    )
    .bind(&pengguna.household_id)
    .bind(hari_ini)
    .bind(&q.q)
    .bind(q.kategori.map(|k| k.as_str()))
    .bind(q.status_stok.map(|s| s.as_str()))
    .bind(&kursor_nama)
    .bind(&kursor_id)
    .bind(limit + 1)
    .fetch_all(&state.db)
    .await?;

    let ada_lagi = baris.len() as i64 > limit;
    let halaman: Vec<&BarisRingkas> = baris.iter().take(limit as usize).collect();
    let next_cursor = if ada_lagi {
        halaman.last().map(|b| kode_kursor(&b.nama_lower, &b.id))
    } else {
        None
    };
    let mut data = Vec::with_capacity(halaman.len());
    for b in halaman {
        let simpan = parse_satuan(&b.satuan_simpan)?;
        let satuan_tampil = parse_satuan(&b.satuan_tampil)?;
        let (jumlah, unit) = tampil(&b.nama, simpan, b.stok, satuan_tampil);
        data.push(ItemRingkas {
            id: b.id.clone(),
            nama: b.nama.clone(),
            kategori: b.kategori.clone(),
            stok_tampil: jumlah,
            satuan_tampil: unit.as_str().to_owned(),
            status_stok: status_stok(b.stok, b.stok_minimum_x10).as_str().to_owned(),
            batch_terdekat_kedaluwarsa: b.terdekat,
            lokasi_default: b.lokasi_default.clone(),
        });
    }
    Ok(Json(HalamanItem { data, next_cursor }))
}

fn parse_satuan(teks: &str) -> Result<Satuan, AppError> {
    Satuan::parse(teks).ok_or_else(|| AppError::internal(format!("satuan tidak dikenal: {teks}")))
}

// ---------------------------------------------------------------------------
// GET /pantry/items/{id}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ItemDetail {
    #[serde(flatten)]
    item: PantryItem,
    stok_tampil: f64,
    /// Satuan `stokTampil` (satuan tampil bila dapat dikonversi, jika tidak satuan simpan).
    satuan_stok: String,
    batches: Vec<StokBatch>,
}

async fn detail_item(
    State(state): State<AppState>,
    pengguna: Pengguna,
    PathBaku(id): PathBaku<String>,
) -> Result<Json<ItemDetail>, AppError> {
    let hari_ini = hari_ini_wib((state.jam)());
    let mut conn = state.db.acquire().await?;
    let item = ambil_item(&mut conn, &pengguna.household_id, &id, false).await?;
    let sql = format!(
        "SELECT {KOLOM_BATCH} FROM stok_batch
          WHERE pantry_item_id = $1 AND jumlah_aktif_x10 > 0
          ORDER BY tanggal_kedaluwarsa ASC NULLS LAST, tanggal_masuk ASC, dibuat_pada ASC, id ASC"
    );
    let batches: Vec<BatchRow> = sqlx::query_as(&sql)
        .bind(&item.id)
        .fetch_all(&mut *conn)
        .await?;
    let stok = stok_dapat_dipakai(&mut conn, &item.id, hari_ini).await?;
    let (jumlah, unit) = tampil(&item.nama, item.simpan()?, stok, item.tampil()?);
    Ok(Json(ItemDetail {
        item: PantryItem::from(&item),
        stok_tampil: jumlah,
        satuan_stok: unit.as_str().to_owned(),
        batches: batches
            .iter()
            .map(|b| StokBatch::dari(b, hari_ini))
            .collect(),
    }))
}

// ---------------------------------------------------------------------------
// POST /pantry/batches (Catat masuk)

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BatchBuat {
    pantry_item_id: String,
    jumlah: f64,
    satuan: Satuan,
    tanggal_masuk: NaiveDate,
    tanggal_kedaluwarsa: Option<NaiveDate>,
    lokasi: Lokasi,
    sumber: Sumber,
}

async fn catat_masuk(
    State(state): State<AppState>,
    pengguna: Pengguna,
    rid: IdPermintaan,
    JsonBaku(req): JsonBaku<BatchBuat>,
) -> Result<Response, AppError> {
    let ctx = Konteks::baru(&state, &pengguna);
    let kunci = Kunci::baru(&ctx.household, rid, "POST /pantry/batches");
    let mut tx = match mulai_idempoten(&state, &kunci).await? {
        Ok(tx) => tx,
        Err(tersimpan) => return Ok(tersimpan),
    };
    let item = ambil_item(&mut tx, &ctx.household, &req.pantry_item_id, true).await?;
    let simpan = item.simpan()?;
    let jumlah_x10 = ke_simpan_x10(&item.nama, simpan, req.jumlah, req.satuan)
        .map_err(|e| e.ke_app("jumlah"))?;
    periksa_batas(simpan, jumlah_x10).map_err(|e| e.ke_app("jumlah"))?;

    if req.tanggal_masuk > ctx.hari_ini || req.tanggal_masuk < ctx.hari_ini - Duration::days(7) {
        return Err(AppError::tak_terproses(
            "TANGGAL_MASUK_TIDAK_VALID",
            "tanggalMasuk tidak boleh di masa depan dan maksimal 7 hari ke belakang",
            Some("tanggalMasuk"),
        ));
    }
    // Default: tanggal masuk + umur simpan; null hanya bila umur simpan 0.
    let kedaluwarsa = match req.tanggal_kedaluwarsa {
        Some(t) => Some(t),
        None if item.umur_simpan_hari_default == 0 => None,
        None => Some(req.tanggal_masuk + Duration::days(i64::from(item.umur_simpan_hari_default))),
    };
    if kedaluwarsa.is_some_and(|t| t < req.tanggal_masuk) {
        return Err(AppError::tak_terproses(
            "TANGGAL_KEDALUWARSA_TIDAK_VALID",
            "tanggalKedaluwarsa tidak boleh sebelum tanggalMasuk",
            Some("tanggalKedaluwarsa"),
        ));
    }

    let batch = buat_batch(
        &mut tx,
        &ctx,
        &item,
        jumlah_x10,
        req.tanggal_masuk,
        kedaluwarsa,
        req.lokasi.as_str(),
        req.sumber.as_str(),
    )
    .await?;
    tulis_mutasi(
        &mut tx,
        &ctx,
        &batch.id,
        "MASUK",
        jumlah_x10,
        jumlah_x10,
        &format!("Catat masuk ({})", req.sumber.as_str()),
        None,
    )
    .await?;
    selesai(
        tx,
        &kunci,
        StatusCode::CREATED,
        &StokBatch::dari(&batch, ctx.hari_ini),
    )
    .await
}

// ---------------------------------------------------------------------------
// POST /pantry/consume (Catat pakai, FEFO)

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PemakaianBaris {
    pantry_item_id: String,
    jumlah: f64,
    satuan: Satuan,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PemakaianRequest {
    pemakaian: Vec<PemakaianBaris>,
    sumber_resep_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Terpotong {
    batch_id: String,
    jumlah: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Kurang {
    pantry_item_id: String,
    kurang_jumlah: f64,
    satuan_simpan: String,
}

#[derive(Debug, Serialize)]
struct PemakaianHasil {
    terpotong: Vec<Terpotong>,
    kurang: Vec<Kurang>,
}

async fn catat_pakai(
    State(state): State<AppState>,
    pengguna: Pengguna,
    rid: IdPermintaan,
    JsonBaku(req): JsonBaku<PemakaianRequest>,
) -> Result<Response, AppError> {
    let ctx = Konteks::baru(&state, &pengguna);
    if req.pemakaian.is_empty() || req.pemakaian.len() > 100 {
        return Err(AppError::validasi(
            "pemakaian harus berisi 1 sampai 100 baris",
            Some("pemakaian"),
        ));
    }
    if req
        .sumber_resep_id
        .as_deref()
        .is_some_and(|s| s.is_empty() || s.len() > 64)
    {
        return Err(AppError::validasi(
            "sumberResepId 1 sampai 64 karakter",
            Some("sumberResepId"),
        ));
    }
    let kunci = Kunci::baru(&ctx.household, rid, "POST /pantry/consume");
    let mut tx = match mulai_idempoten(&state, &kunci).await? {
        Ok(tx) => tx,
        Err(tersimpan) => return Ok(tersimpan),
    };

    // Kunci kartu item dalam urutan ID agar dua permintaan serentak tidak saling buntu.
    let mut urut: Vec<&str> = req
        .pemakaian
        .iter()
        .map(|b| b.pantry_item_id.as_str())
        .collect();
    urut.sort_unstable();
    urut.dedup();
    let mut kartu = std::collections::HashMap::new();
    for id in urut {
        let item = ambil_item(&mut tx, &ctx.household, id, true).await?;
        kartu.insert(id.to_owned(), item);
    }

    let alasan = match &req.sumber_resep_id {
        Some(r) => format!("Pakai FEFO (resep {r})"),
        None => "Pakai FEFO".to_owned(),
    };
    let mut disentuh = HashSet::new();
    let mut terpotong = Vec::new();
    let mut kurang = Vec::new();
    for (i, baris) in req.pemakaian.iter().enumerate() {
        let field = format!("pemakaian[{i}].jumlah");
        let item = &kartu[&baris.pantry_item_id];
        let simpan = item.simpan()?;
        let mut sisa_x10 = ke_simpan_x10(&item.nama, simpan, baris.jumlah, baris.satuan)
            .map_err(|e| e.ke_app(&field))?;
        periksa_batas(simpan, sisa_x10).map_err(|e| e.ke_app(&field))?;

        let batches = batch_fefo(&mut tx, &item.id, ctx.hari_ini).await?;
        if batches.is_empty() && disentuh.insert(item.id.clone()) {
            // Stok nol (atau hanya batch kedaluwarsa): tolak seluruh permintaan.
            return Err(AppError::tak_terproses(
                "STOK_HABIS",
                format!("Stok {} habis", item.nama),
                Some(&format!("pemakaian[{i}].pantryItemId")),
            ));
        }
        disentuh.insert(item.id.clone());
        for batch in &batches {
            if sisa_x10 == 0 {
                break;
            }
            let ambil = sisa_x10.min(batch.jumlah_aktif_x10);
            kurangi_batch(
                &mut tx,
                &ctx,
                batch,
                ambil,
                "PAKAI",
                &alasan,
                req.sumber_resep_id.as_deref(),
                "HABIS",
            )
            .await?;
            terpotong.push(Terpotong {
                batch_id: batch.id.clone(),
                jumlah: ambil as f64 / 10.0,
            });
            sisa_x10 -= ambil;
        }
        if sisa_x10 > 0 {
            kurang.push(Kurang {
                pantry_item_id: item.id.clone(),
                kurang_jumlah: sisa_x10 as f64 / 10.0,
                satuan_simpan: simpan.as_str().to_owned(),
            });
        }
    }
    selesai(
        tx,
        &kunci,
        StatusCode::OK,
        &PemakaianHasil { terpotong, kurang },
    )
    .await
}

// ---------------------------------------------------------------------------
// POST /pantry/opname

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpnameRequest {
    pantry_item_id: String,
    stok_fisik: f64,
    satuan: Satuan,
    alasan: String,
    /// Opsional: stok catatan yang dilihat pengguna; berbeda dengan server = `409`.
    stok_catat_terakhir: Option<f64>,
}

#[derive(Debug, Serialize)]
struct HasilMutasiList {
    mutasi: Vec<MutasiStok>,
}

async fn opname(
    State(state): State<AppState>,
    pengguna: Pengguna,
    rid: IdPermintaan,
    JsonBaku(req): JsonBaku<OpnameRequest>,
) -> Result<Response, AppError> {
    let ctx = Konteks::baru(&state, &pengguna);
    let alasan = alasan_valid(&req.alasan, "alasan")?;
    if !req.stok_fisik.is_finite() || req.stok_fisik < 0.0 {
        return Err(AppError::validasi(
            "stokFisik tidak boleh negatif",
            Some("stokFisik"),
        ));
    }
    let kunci = Kunci::baru(&ctx.household, rid, "POST /pantry/opname");
    let mut tx = match mulai_idempoten(&state, &kunci).await? {
        Ok(tx) => tx,
        Err(tersimpan) => return Ok(tersimpan),
    };
    let item = ambil_item(&mut tx, &ctx.household, &req.pantry_item_id, true).await?;
    let simpan = item.simpan()?;
    let ke_x10 = |nilai: f64, field: &str| -> Result<i64, AppError> {
        if nilai == 0.0 {
            Ok(0)
        } else {
            ke_simpan_x10(&item.nama, simpan, nilai, req.satuan).map_err(|e| e.ke_app(field))
        }
    };
    let fisik_x10 = ke_x10(req.stok_fisik, "stokFisik")?;
    let catat_x10 = stok_dapat_dipakai(&mut tx, &item.id, ctx.hari_ini).await?;
    if let Some(terakhir) = req.stok_catat_terakhir {
        if terakhir < 0.0 || ke_x10(terakhir, "stokCatatTerakhir")? != catat_x10 {
            return Err(AppError::Konflik(crate::error::Detail::baru(
                "KONFLIK_OPNAME",
                "Stok catatan sudah berubah sejak dilihat; muat ulang lalu ulangi opname",
                Some("stokCatatTerakhir"),
            )));
        }
    }

    let selisih = fisik_x10 - catat_x10;
    let mut hasil = Vec::new();
    if selisih < 0 {
        // Koreksi negatif mengambil dari batch FEFO.
        let mut sisa = -selisih;
        for batch in batch_fefo(&mut tx, &item.id, ctx.hari_ini).await? {
            if sisa == 0 {
                break;
            }
            let ambil = sisa.min(batch.jumlah_aktif_x10);
            hasil.push(
                kurangi_batch(
                    &mut tx, &ctx, &batch, ambil, "KOREKSI", &alasan, None, "HABIS",
                )
                .await?,
            );
            sisa -= ambil;
        }
    } else if selisih > 0 {
        periksa_batas(simpan, selisih).map_err(|e| e.ke_app("stokFisik"))?;
        let kedaluwarsa = (item.umur_simpan_hari_default > 0)
            .then(|| ctx.hari_ini + Duration::days(i64::from(item.umur_simpan_hari_default)));
        let batch = buat_batch(
            &mut tx,
            &ctx,
            &item,
            selisih,
            ctx.hari_ini,
            kedaluwarsa,
            &item.lokasi_default,
            "MASUK_KOREKSI",
        )
        .await?;
        hasil.push(
            tulis_mutasi(
                &mut tx, &ctx, &batch.id, "KOREKSI", selisih, selisih, &alasan, None,
            )
            .await?,
        );
    }
    let badan = HasilMutasiList {
        mutasi: hasil.iter().map(MutasiStok::from).collect(),
    };
    selesai(tx, &kunci, StatusCode::OK, &badan).await
}

// ---------------------------------------------------------------------------
// POST /pantry/batches/{id}/pindah

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PindahRequest {
    lokasi_tujuan: Lokasi,
}

async fn pindah_lokasi(
    State(state): State<AppState>,
    pengguna: Pengguna,
    rid: IdPermintaan,
    PathBaku(id): PathBaku<String>,
    JsonBaku(req): JsonBaku<PindahRequest>,
) -> Result<Response, AppError> {
    let ctx = Konteks::baru(&state, &pengguna);
    let kunci = Kunci::baru(
        &ctx.household,
        rid,
        format!("POST /pantry/batches/{id}/pindah"),
    );
    let mut tx = match mulai_idempoten(&state, &kunci).await? {
        Ok(tx) => tx,
        Err(tersimpan) => return Ok(tersimpan),
    };
    let batch = ambil_batch(&mut tx, &ctx.household, &id, true).await?;
    if batch.status != "AKTIF" || batch.jumlah_aktif_x10 == 0 {
        return Err(AppError::tak_terproses(
            "BATCH_TIDAK_AKTIF",
            "Batch kosong atau sudah dibuang tidak bisa dipindah",
            None,
        ));
    }
    if batch.lokasi == req.lokasi_tujuan.as_str() {
        return Err(AppError::tak_terproses(
            "LOKASI_SAMA",
            "Batch sudah berada di lokasi tujuan",
            Some("lokasiTujuan"),
        ));
    }
    let baru: BatchRow = sqlx::query_as(&format!(
        "UPDATE stok_batch SET lokasi = $2 WHERE id = $1 RETURNING {KOLOM_BATCH}"
    ))
    .bind(&batch.id)
    .bind(req.lokasi_tujuan.as_str())
    .fetch_one(&mut *tx)
    .await?;
    tulis_mutasi(
        &mut tx,
        &ctx,
        &batch.id,
        "PINDAH",
        0,
        batch.jumlah_aktif_x10,
        &format!("{} -> {}", batch.lokasi, req.lokasi_tujuan.as_str()),
        None,
    )
    .await?;
    selesai(
        tx,
        &kunci,
        StatusCode::OK,
        &StokBatch::dari(&baru, ctx.hari_ini),
    )
    .await
}

// ---------------------------------------------------------------------------
// POST /pantry/batches/{id}/buang

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BuangRequest {
    jumlah: f64,
    satuan: Satuan,
    alasan: AlasanBuang,
}

async fn buang_batch(
    State(state): State<AppState>,
    pengguna: Pengguna,
    rid: IdPermintaan,
    PathBaku(id): PathBaku<String>,
    JsonBaku(req): JsonBaku<BuangRequest>,
) -> Result<Response, AppError> {
    let ctx = Konteks::baru(&state, &pengguna);
    let kunci = Kunci::baru(
        &ctx.household,
        rid,
        format!("POST /pantry/batches/{id}/buang"),
    );
    let mut tx = match mulai_idempoten(&state, &kunci).await? {
        Ok(tx) => tx,
        Err(tersimpan) => return Ok(tersimpan),
    };
    // Urutan kunci sama dengan alur lain (kartu item dulu, lalu batch) agar tidak buntu.
    let awal = ambil_batch(&mut tx, &ctx.household, &id, false).await?;
    let item = ambil_item(&mut tx, &ctx.household, &awal.pantry_item_id, true).await?;
    let batch = ambil_batch(&mut tx, &ctx.household, &id, true).await?;
    let simpan = item.simpan()?;
    let jumlah_x10 = ke_simpan_x10(&item.nama, simpan, req.jumlah, req.satuan)
        .map_err(|e| e.ke_app("jumlah"))?;
    if jumlah_x10 > batch.jumlah_aktif_x10 {
        return Err(AppError::tak_terproses(
            "JUMLAH_MELEBIHI_SISA",
            "Jumlah buang melebihi sisa batch",
            Some("jumlah"),
        ));
    }
    let mutasi = kurangi_batch(
        &mut tx,
        &ctx,
        &batch,
        jumlah_x10,
        "BUANG",
        req.alasan.as_str(),
        None,
        "DIBUANG",
    )
    .await?;
    selesai(tx, &kunci, StatusCode::OK, &MutasiStok::from(&mutasi)).await
}

// ---------------------------------------------------------------------------
// GET /pantry/expiring

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueryKedaluwarsa {
    dalam_hari: Option<i64>,
}

#[derive(Debug, sqlx::FromRow)]
struct BarisKedaluwarsa {
    batch_id: String,
    pantry_item_id: String,
    nama: String,
    satuan_simpan: String,
    satuan_tampil: String,
    jumlah_aktif_x10: i64,
    tanggal_kedaluwarsa: NaiveDate,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchKedaluwarsa {
    batch_id: String,
    pantry_item_id: String,
    nama: String,
    sisa: f64,
    satuan_tampil: String,
    tanggal_kedaluwarsa: NaiveDate,
    status: &'static str,
}

#[derive(Debug, Serialize)]
struct DaftarKedaluwarsa {
    data: Vec<BatchKedaluwarsa>,
}

async fn daftar_kedaluwarsa(
    State(state): State<AppState>,
    pengguna: Pengguna,
    QueryBaku(q): QueryBaku<QueryKedaluwarsa>,
) -> Result<Json<DaftarKedaluwarsa>, AppError> {
    let dalam = q.dalam_hari.unwrap_or(7);
    if !(0..=3650).contains(&dalam) {
        return Err(AppError::validasi(
            "dalamHari harus 0 sampai 3650",
            Some("dalamHari"),
        ));
    }
    let hari_ini = hari_ini_wib((state.jam)());
    // Termasuk yang sudah lewat (KEDALUWARSA): menunggu aksi Buang.
    let baris: Vec<BarisKedaluwarsa> = sqlx::query_as(
        "SELECT b.id AS batch_id, b.pantry_item_id, i.nama, i.satuan_simpan, i.satuan_tampil,
                b.jumlah_aktif_x10, b.tanggal_kedaluwarsa
           FROM stok_batch b JOIN pantry_items i ON i.id = b.pantry_item_id
          WHERE b.household_id = $1 AND NOT i.arsip AND b.status = 'AKTIF'
            AND b.jumlah_aktif_x10 > 0 AND b.tanggal_kedaluwarsa IS NOT NULL
            AND b.tanggal_kedaluwarsa <= $2
          ORDER BY b.tanggal_kedaluwarsa, b.tanggal_masuk, b.dibuat_pada, b.id
          LIMIT 500",
    )
    .bind(&pengguna.household_id)
    .bind(hari_ini + Duration::days(dalam))
    .fetch_all(&state.db)
    .await?;
    let mut data = Vec::with_capacity(baris.len());
    for b in baris {
        let (sisa, unit) = tampil(
            &b.nama,
            parse_satuan(&b.satuan_simpan)?,
            b.jumlah_aktif_x10,
            parse_satuan(&b.satuan_tampil)?,
        );
        data.push(BatchKedaluwarsa {
            batch_id: b.batch_id,
            pantry_item_id: b.pantry_item_id,
            nama: b.nama,
            sisa,
            satuan_tampil: unit.as_str().to_owned(),
            tanggal_kedaluwarsa: b.tanggal_kedaluwarsa,
            status: status_kedaluwarsa((b.tanggal_kedaluwarsa - hari_ini).num_days()),
        });
    }
    Ok(Json(DaftarKedaluwarsa { data }))
}

pub fn rute() -> Router<AppState> {
    Router::new()
        .route("/pantry/items", get(daftar_item).post(buat_item))
        .route("/pantry/items/{id}", get(detail_item))
        .route("/pantry/batches", post(catat_masuk))
        .route("/pantry/batches/{id}/pindah", post(pindah_lokasi))
        .route("/pantry/batches/{id}/buang", post(buang_batch))
        .route("/pantry/consume", post(catat_pakai))
        .route("/pantry/opname", post(opname))
        .route("/pantry/expiring", get(daftar_kedaluwarsa))
}
