//! Enum domain, baris basis data, dan bentuk respons JSON (camelCase) sesuai OpenAPI.

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;

use crate::error::AppError;
use crate::satuan::Satuan;

enum_teks!(
    Kategori {
        Sayur => "SAYUR",
        Bumbu => "BUMBU",
        Buah => "BUAH",
        Protein => "PROTEIN",
        Kering => "KERING",
        Cair => "CAIR",
        Beku => "BEKU",
        Sisa => "SISA",
    }
);

enum_teks!(
    Lokasi {
        KulkasBawah => "KULKAS_BAWAH",
        KulkasAtas => "KULKAS_ATAS",
        Freezer => "FREEZER",
        SuhuRuang => "SUHU_RUANG",
        BaskomBawang => "BASKOM_BAWANG",
    }
);

enum_teks!(
    Sumber {
        MasukBelanja => "MASUK_BELANJA",
        MasukPanen => "MASUK_PANEN",
        MasukTitipo => "MASUK_TITIPO",
        MasukKoreksi => "MASUK_KOREKSI",
    }
);

enum_teks!(
    AlasanBuang {
        Busuk => "busuk",
        BerlendirBerbau => "berlendir/berbau",
        LewatTanggal => "lewat tanggal",
        Tercemar => "tercemar",
    }
);

enum_teks!(
    StatusStok {
        Aman => "AMAN",
        Menipis => "MENIPIS",
        Habis => "HABIS",
    }
);

/// Menipis = stok <= minimum dan > 0; habis = 0 (`inventori/10` bab 3).
pub fn status_stok(stok_x10: i64, minimum_x10: i64) -> StatusStok {
    if stok_x10 <= 0 {
        StatusStok::Habis
    } else if stok_x10 <= minimum_x10 {
        StatusStok::Menipis
    } else {
        StatusStok::Aman
    }
}

/// Ambang SOP kedaluwarsa dari selisih hari kalender (`notifikasi/10` bab 2).
pub fn status_kedaluwarsa(sisa_hari: i64) -> &'static str {
    match sisa_hari {
        h if h > 7 => "AMAN",
        4..=7 => "WASPADA",
        1..=3 => "SEGERA",
        0 => "HARI_INI",
        _ => "KEDALUWARSA",
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ItemRow {
    pub id: String,
    pub household_id: String,
    pub nama: String,
    pub kategori: String,
    pub satuan_simpan: String,
    pub satuan_tampil: String,
    pub stok_minimum_x10: i64,
    pub umur_simpan_hari_default: i32,
    pub lokasi_default: String,
    pub foto_url: Option<String>,
    pub arsip: bool,
    pub dibuat_pada: DateTime<Utc>,
    pub diubah_pada: DateTime<Utc>,
}

pub const KOLOM_ITEM: &str = "id, household_id, nama, kategori, satuan_simpan, satuan_tampil, \
    stok_minimum_x10, umur_simpan_hari_default, lokasi_default, foto_url, arsip, dibuat_pada, diubah_pada";

fn satuan_dari(teks: &str) -> Result<Satuan, AppError> {
    Satuan::parse(teks)
        .ok_or_else(|| AppError::internal(format!("satuan tidak dikenal di DB: {teks}")))
}

impl ItemRow {
    pub fn simpan(&self) -> Result<Satuan, AppError> {
        satuan_dari(&self.satuan_simpan)
    }

    pub fn tampil(&self) -> Result<Satuan, AppError> {
        satuan_dari(&self.satuan_tampil)
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct BatchRow {
    pub id: String,
    pub household_id: String,
    pub pantry_item_id: String,
    pub jumlah_awal_x10: i64,
    pub jumlah_aktif_x10: i64,
    pub tanggal_masuk: NaiveDate,
    pub tanggal_kedaluwarsa: Option<NaiveDate>,
    pub lokasi: String,
    pub sumber: String,
    pub status: String,
    pub dibuat_pada: DateTime<Utc>,
}

pub const KOLOM_BATCH: &str =
    "id, household_id, pantry_item_id, jumlah_awal_x10, jumlah_aktif_x10, \
    tanggal_masuk, tanggal_kedaluwarsa, lokasi, sumber, status, dibuat_pada";

impl BatchRow {
    /// Status turunan: AKTIF yang sudah lewat tanggal kedaluwarsa menjadi KEDALUWARSA.
    pub fn status_efektif(&self, hari_ini: NaiveDate) -> &str {
        match (self.status.as_str(), self.tanggal_kedaluwarsa) {
            ("AKTIF", Some(t)) if self.jumlah_aktif_x10 > 0 && t < hari_ini => "KEDALUWARSA",
            (s, _) => s,
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MutasiRow {
    pub id: String,
    pub batch_id: String,
    pub jenis: String,
    pub jumlah_x10: i64,
    pub sisa_setelah_x10: i64,
    pub alasan: String,
    pub aktor: String,
    pub terjadi_pada: DateTime<Utc>,
}

pub const KOLOM_MUTASI: &str =
    "id, batch_id, jenis, jumlah_x10, sisa_setelah_x10, alasan, aktor, terjadi_pada";

fn x10(nilai: i64) -> f64 {
    nilai as f64 / 10.0
}

// ---------------------------------------------------------------------------
// Bentuk respons

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PantryItem {
    pub id: String,
    pub household_id: String,
    pub nama: String,
    pub kategori: String,
    pub satuan_simpan: String,
    pub satuan_tampil: String,
    pub stok_minimum: f64,
    pub umur_simpan_hari_default: i32,
    pub lokasi_default: String,
    pub foto_url: Option<String>,
    pub arsip: bool,
    pub dibuat_pada: DateTime<Utc>,
    pub diubah_pada: DateTime<Utc>,
}

impl From<&ItemRow> for PantryItem {
    fn from(r: &ItemRow) -> Self {
        Self {
            id: r.id.clone(),
            household_id: r.household_id.clone(),
            nama: r.nama.clone(),
            kategori: r.kategori.clone(),
            satuan_simpan: r.satuan_simpan.clone(),
            satuan_tampil: r.satuan_tampil.clone(),
            stok_minimum: x10(r.stok_minimum_x10),
            umur_simpan_hari_default: r.umur_simpan_hari_default,
            lokasi_default: r.lokasi_default.clone(),
            foto_url: r.foto_url.clone(),
            arsip: r.arsip,
            dibuat_pada: r.dibuat_pada,
            diubah_pada: r.diubah_pada,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StokBatch {
    pub id: String,
    pub pantry_item_id: String,
    pub jumlah_awal: f64,
    pub jumlah_aktif: f64,
    pub tanggal_masuk: NaiveDate,
    pub tanggal_kedaluwarsa: Option<NaiveDate>,
    pub lokasi: String,
    pub sumber: String,
    pub status: String,
    pub dibuat_pada: DateTime<Utc>,
}

impl StokBatch {
    pub fn dari(r: &BatchRow, hari_ini: NaiveDate) -> Self {
        Self {
            id: r.id.clone(),
            pantry_item_id: r.pantry_item_id.clone(),
            jumlah_awal: x10(r.jumlah_awal_x10),
            jumlah_aktif: x10(r.jumlah_aktif_x10),
            tanggal_masuk: r.tanggal_masuk,
            tanggal_kedaluwarsa: r.tanggal_kedaluwarsa,
            lokasi: r.lokasi.clone(),
            sumber: r.sumber.clone(),
            status: r.status_efektif(hari_ini).to_owned(),
            dibuat_pada: r.dibuat_pada,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MutasiStok {
    pub id: String,
    pub batch_id: String,
    pub jenis: String,
    pub jumlah: f64,
    pub sisa_setelah: f64,
    pub alasan: String,
    pub aktor: String,
    pub terjadi_pada: DateTime<Utc>,
}

impl From<&MutasiRow> for MutasiStok {
    fn from(r: &MutasiRow) -> Self {
        Self {
            id: r.id.clone(),
            batch_id: r.batch_id.clone(),
            jenis: r.jenis.clone(),
            jumlah: x10(r.jumlah_x10),
            sisa_setelah: x10(r.sisa_setelah_x10),
            alasan: r.alasan.clone(),
            aktor: r.aktor.clone(),
            terjadi_pada: r.terjadi_pada,
        }
    }
}

#[cfg(test)]
mod uji {
    use super::*;

    #[test]
    fn ambang_sop_kedaluwarsa() {
        let kasus = [
            (30, "AMAN"),
            (8, "AMAN"),
            (7, "WASPADA"),
            (4, "WASPADA"),
            (3, "SEGERA"),
            (1, "SEGERA"),
            (0, "HARI_INI"),
            (-1, "KEDALUWARSA"),
            (-30, "KEDALUWARSA"),
        ];
        for (hari, status) in kasus {
            assert_eq!(status_kedaluwarsa(hari), status, "{hari} hari");
        }
    }

    #[test]
    fn status_stok_menipis_habis_aman() {
        assert_eq!(status_stok(0, 2500), StatusStok::Habis);
        assert_eq!(status_stok(2500, 2500), StatusStok::Menipis); // <= minimum
        assert_eq!(status_stok(2501, 2500), StatusStok::Aman);
        assert_eq!(status_stok(10, 0), StatusStok::Aman); // minimum 0 tidak pernah menipis
    }
}
