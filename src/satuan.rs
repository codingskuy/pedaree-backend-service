//! 13 satuan baku, konversi, bobot acuan, dan validasi jumlah.
//!
//! Sumber sah: `inventori/20-satuan-hortikultura.md` (Pedaree-TownHall). Jumlah dipakai sebagai
//! bilangan bulat x10 (ADR-0002): ketelitian 0,1 g/ml, pembulatan setengah ke atas.

use crate::error::AppError;

enum_teks!(
    Satuan {
        Gram => "gram",
        Kg => "kg",
        Ml => "ml",
        Liter => "liter",
        Sdm => "sdm",
        Sdt => "sdt",
        Pcs => "pcs",
        Siung => "siung",
        Buah => "buah",
        Ikat => "ikat",
        Batang => "batang",
        Bonggol => "bonggol",
        Lembar => "lembar",
    }
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Jenis {
    Berat,
    Volume,
    Hitung,
}

/// Batas per batch untuk gram/ml (50.000), dalam skala x10.
pub const BATAS_BATCH_X10: i64 = 50_000 * 10;

impl Satuan {
    pub fn jenis(self) -> Jenis {
        match self {
            Self::Gram | Self::Kg => Jenis::Berat,
            Self::Ml | Self::Liter | Self::Sdm | Self::Sdt => Jenis::Volume,
            _ => Jenis::Hitung,
        }
    }

    /// Satuan penyimpanan kanonis: kg disimpan gram; liter/sdm/sdt disimpan ml.
    pub fn kanonis(self) -> Self {
        match self {
            Self::Kg => Self::Gram,
            Self::Liter | Self::Sdm | Self::Sdt => Self::Ml,
            lain => lain,
        }
    }

    /// Pengali ke satuan kanonis (kg x1000, liter x1000, sdm x15, sdt x5).
    pub fn faktor(self) -> f64 {
        match self {
            Self::Kg | Self::Liter => 1000.0,
            Self::Sdm => 15.0,
            Self::Sdt => 5.0,
            _ => 1.0,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum GalatSatuan {
    /// Jumlah kurang dari atau sama dengan nol.
    TidakPositif,
    /// Satuan hitung tidak boleh pecahan.
    Pecahan,
    /// Konversi/satuan tidak berlaku untuk komoditas ini.
    TidakBerlaku,
    /// Di atas 50.000 g atau 50.000 ml per batch.
    MelebihiBatas,
}

impl GalatSatuan {
    pub fn ke_app(self, field: &str) -> AppError {
        match self {
            Self::TidakPositif => AppError::validasi("jumlah harus lebih dari 0", Some(field)),
            Self::Pecahan => AppError::tak_terproses(
                "JUMLAH_PECAHAN",
                "satuan hitung tidak boleh pecahan; masukkan bilangan bulat atau gunakan gram",
                Some(field),
            ),
            Self::TidakBerlaku => AppError::tak_terproses(
                "SATUAN_TIDAK_BERLAKU",
                "satuan tidak berlaku untuk komoditas ini",
                Some(field),
            ),
            Self::MelebihiBatas => AppError::tak_terproses(
                "JUMLAH_MELEBIHI_BATAS",
                "jumlah di atas 50.000 g/ml per batch; pecah menjadi beberapa batch",
                Some(field),
            ),
        }
    }
}

fn normal(nama: &str) -> String {
    nama.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Bobot acuan (gram per 1 satuan hitung), tabel 4 `inventori/20-satuan-hortikultura.md`.
fn bobot_gram(nama: &str, satuan: Satuan) -> Option<f64> {
    use Satuan::*;
    let tabel: &[(Satuan, f64)] = match normal(nama).as_str() {
        "bayam" | "kangkung" => &[(Ikat, 200.0)],
        "kemangi" => &[(Ikat, 50.0)],
        "daun bawang" => &[(Batang, 25.0)],
        "sawi hijau" => &[(Bonggol, 300.0)],
        "kol" => &[(Bonggol, 800.0)],
        "wortel" => &[(Buah, 100.0)],
        "kentang" => &[(Buah, 150.0)],
        "tomat" => &[(Buah, 80.0)],
        "cabai merah" => &[(Buah, 5.0)],
        "bawang merah" => &[(Buah, 10.0), (Siung, 10.0)],
        "bawang putih" => &[(Siung, 5.0), (Bonggol, 50.0)],
        "jagung" => &[(Buah, 250.0)],
        _ => &[],
    };
    tabel.iter().find(|(u, _)| *u == satuan).map(|(_, g)| *g)
}

/// `pcs` selalu sah; satuan hitung lain hanya untuk komoditas yang ada di tabel bobot.
pub fn satuan_sah_untuk(nama: &str, satuan: Satuan) -> bool {
    satuan.jenis() != Jenis::Hitung || satuan == Satuan::Pcs || bobot_gram(nama, satuan).is_some()
}

/// Apakah jumlah dalam `simpan` dapat ditampilkan/dimasukkan memakai `lain`.
pub fn dapat_dikonversi(nama: &str, simpan: Satuan, lain: Satuan) -> bool {
    let simpan = simpan.kanonis();
    match (simpan.jenis(), lain.jenis()) {
        (Jenis::Berat, Jenis::Berat) | (Jenis::Volume, Jenis::Volume) => true,
        (Jenis::Hitung, Jenis::Hitung) => {
            simpan == lain
                || (bobot_gram(nama, simpan).is_some() && bobot_gram(nama, lain).is_some())
        }
        (Jenis::Hitung, Jenis::Berat) => bobot_gram(nama, simpan).is_some(),
        (Jenis::Berat, Jenis::Hitung) => bobot_gram(nama, lain).is_some(),
        _ => false,
    }
}

fn hampir_bulat(x: f64) -> bool {
    (x - x.round()).abs() < 1e-6
}

/// Pembulatan setengah ke atas ke 1 desimal, hasil skala x10.
fn bulatkan_x10(basis: f64) -> i64 {
    (basis * 10.0 + 1e-7).round() as i64
}

/// Mengonversi `jumlah` dalam satuan `dari` ke satuan simpan kanonis item, skala x10.
pub fn ke_simpan_x10(
    nama: &str,
    simpan: Satuan,
    jumlah: f64,
    dari: Satuan,
) -> Result<i64, GalatSatuan> {
    if !jumlah.is_finite() || jumlah <= 0.0 {
        return Err(GalatSatuan::TidakPositif);
    }
    if dari.jenis() == Jenis::Hitung && !hampir_bulat(jumlah) {
        return Err(GalatSatuan::Pecahan);
    }
    let simpan = simpan.kanonis();
    let basis = match (dari.jenis(), simpan.jenis()) {
        (Jenis::Berat, Jenis::Berat) | (Jenis::Volume, Jenis::Volume) => jumlah * dari.faktor(),
        (Jenis::Hitung, Jenis::Hitung) if dari == simpan => jumlah,
        (Jenis::Hitung, Jenis::Hitung) => {
            let a = bobot_gram(nama, dari).ok_or(GalatSatuan::TidakBerlaku)?;
            let b = bobot_gram(nama, simpan).ok_or(GalatSatuan::TidakBerlaku)?;
            jumlah * a / b
        }
        (Jenis::Hitung, Jenis::Berat) => {
            jumlah * bobot_gram(nama, dari).ok_or(GalatSatuan::TidakBerlaku)?
        }
        (Jenis::Berat, Jenis::Hitung) => {
            let b = bobot_gram(nama, simpan).ok_or(GalatSatuan::TidakBerlaku)?;
            jumlah * dari.faktor() / b
        }
        _ => return Err(GalatSatuan::TidakBerlaku),
    };
    if simpan.jenis() == Jenis::Hitung {
        if !hampir_bulat(basis) {
            return Err(GalatSatuan::Pecahan);
        }
        let n = basis.round() as i64;
        if n <= 0 {
            return Err(GalatSatuan::TidakPositif);
        }
        return Ok(n * 10);
    }
    let x10 = bulatkan_x10(basis);
    if x10 <= 0 {
        return Err(GalatSatuan::TidakPositif);
    }
    Ok(x10)
}

/// Menolak jumlah gram/ml di atas 50.000 per batch.
pub fn periksa_batas(simpan: Satuan, x10: i64) -> Result<(), GalatSatuan> {
    if simpan.kanonis().jenis() != Jenis::Hitung && x10 > BATAS_BATCH_X10 {
        return Err(GalatSatuan::MelebihiBatas);
    }
    Ok(())
}

fn bulat_4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

/// Mengonversi jumlah simpan (x10) ke satuan `ke`; `None` bila tidak berlaku atau pecahan hitung.
pub fn dari_simpan(nama: &str, simpan: Satuan, x10: i64, ke: Satuan) -> Option<f64> {
    let simpan = simpan.kanonis();
    let basis = x10 as f64 / 10.0;
    if ke == simpan {
        return Some(basis);
    }
    match (simpan.jenis(), ke.jenis()) {
        (Jenis::Berat, Jenis::Berat) | (Jenis::Volume, Jenis::Volume) => {
            Some(bulat_4(basis / ke.faktor()))
        }
        (Jenis::Hitung, Jenis::Hitung) => {
            let v = basis * bobot_gram(nama, simpan)? / bobot_gram(nama, ke)?;
            hampir_bulat(v).then(|| v.round())
        }
        (Jenis::Berat, Jenis::Hitung) => {
            let v = basis / bobot_gram(nama, ke)?;
            hampir_bulat(v).then(|| v.round())
        }
        (Jenis::Hitung, Jenis::Berat) => {
            Some(bulat_4(basis * bobot_gram(nama, simpan)? / ke.faktor()))
        }
        _ => None,
    }
}

/// Jumlah untuk tampilan: dalam satuan tampil bila bisa, jika tidak kembali ke satuan simpan.
pub fn tampil(nama: &str, simpan: Satuan, x10: i64, tampil: Satuan) -> (f64, Satuan) {
    match dari_simpan(nama, simpan, x10, tampil) {
        Some(v) => (v, tampil),
        None => (x10 as f64 / 10.0, simpan.kanonis()),
    }
}

#[cfg(test)]
mod uji {
    use super::Satuan::*;
    use super::*;

    #[test]
    fn tiga_belas_satuan_baku_dan_hanya_itu() {
        let semua = [
            "gram", "kg", "ml", "liter", "sdm", "sdt", "pcs", "siung", "buah", "ikat", "batang",
            "bonggol", "lembar",
        ];
        for s in semua {
            assert_eq!(Satuan::parse(s).unwrap().as_str(), s);
        }
        for salah in ["g", "ons", "butir", "pak", "Gram", ""] {
            assert!(Satuan::parse(salah).is_none(), "{salah}");
        }
    }

    #[test]
    fn konversi_berat_dan_volume_ke_satuan_kanonis() {
        assert_eq!(ke_simpan_x10("Beras", Gram, 1.0, Kg), Ok(10_000)); // 1 kg = 1000 g
        assert_eq!(ke_simpan_x10("Beras", Gram, 500.0, Gram), Ok(5_000));
        assert_eq!(ke_simpan_x10("Minyak", Ml, 2.0, Liter), Ok(20_000));
        assert_eq!(ke_simpan_x10("Minyak", Ml, 3.0, Sdm), Ok(450)); // 3 x 15 ml
        assert_eq!(ke_simpan_x10("Minyak", Ml, 4.0, Sdt), Ok(200)); // 4 x 5 ml
        assert_eq!(ke_simpan_x10("Minyak", Ml, 1.5, Liter), Ok(15_000));
    }

    #[test]
    fn contoh_dokumen_sumber() {
        // 2 ikat bayam -> 400 g; 6 buah tomat -> 480 g
        assert_eq!(ke_simpan_x10("Bayam", Gram, 2.0, Ikat), Ok(4_000));
        assert_eq!(ke_simpan_x10("tomat", Gram, 6.0, Buah), Ok(4_800));
        // 500 g bawang merah tampil 50 siung; stok tetap 500 g
        assert_eq!(dari_simpan("Bawang Merah", Gram, 5_000, Siung), Some(50.0));
        // 1 kg beras -> 1000 g
        assert_eq!(dari_simpan("Beras", Gram, 10_000, Kg), Some(1.0));
    }

    #[test]
    fn pembulatan_setengah_ke_atas_satu_desimal() {
        assert_eq!(ke_simpan_x10("Garam", Gram, 0.25, Gram), Ok(3)); // 0,3 g
        assert_eq!(ke_simpan_x10("Garam", Gram, 0.24, Gram), Ok(2));
        assert_eq!(ke_simpan_x10("Garam", Gram, 0.0001, Kg), Ok(1)); // 0,1 g
        assert_eq!(
            ke_simpan_x10("Garam", Gram, 0.04, Gram),
            Err(GalatSatuan::TidakPositif)
        );
    }

    #[test]
    fn jumlah_nol_negatif_dan_nan_ditolak() {
        for j in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                ke_simpan_x10("Beras", Gram, j, Gram),
                Err(GalatSatuan::TidakPositif),
                "{j}"
            );
        }
    }

    #[test]
    fn satuan_hitung_pecahan_ditolak() {
        assert_eq!(
            ke_simpan_x10("Telur Ayam", Pcs, 1.5, Pcs),
            Err(GalatSatuan::Pecahan)
        );
        assert_eq!(
            ke_simpan_x10("Bayam", Gram, 1.5, Ikat),
            Err(GalatSatuan::Pecahan)
        );
        assert_eq!(ke_simpan_x10("Telur Ayam", Pcs, 6.0, Pcs), Ok(60));
    }

    #[test]
    fn satuan_tak_berlaku_untuk_komoditas_di_luar_tabel() {
        assert_eq!(
            ke_simpan_x10("Beras", Gram, 1.0, Ikat),
            Err(GalatSatuan::TidakBerlaku)
        );
        assert_eq!(
            ke_simpan_x10("Telur Ayam", Gram, 100.0, Buah),
            Err(GalatSatuan::TidakBerlaku)
        );
        // sel "—" pada tabel: bayam tidak punya konversi buah/siung
        assert_eq!(
            ke_simpan_x10("Bayam", Gram, 1.0, Buah),
            Err(GalatSatuan::TidakBerlaku)
        );
        assert_eq!(
            ke_simpan_x10("Wortel", Gram, 1.0, Ikat),
            Err(GalatSatuan::TidakBerlaku)
        );
        // berat <-> volume tidak berlaku
        assert_eq!(
            ke_simpan_x10("Minyak", Ml, 100.0, Gram),
            Err(GalatSatuan::TidakBerlaku)
        );
        assert_eq!(
            ke_simpan_x10("Beras", Gram, 1.0, Liter),
            Err(GalatSatuan::TidakBerlaku)
        );
        // pcs tidak punya bobot
        assert_eq!(
            ke_simpan_x10("Tomat", Gram, 1.0, Pcs),
            Err(GalatSatuan::TidakBerlaku)
        );
    }

    #[test]
    fn batas_50000_gram_atau_ml_per_batch() {
        let x10 = ke_simpan_x10("Beras", Gram, 50.0, Kg).unwrap();
        assert_eq!(periksa_batas(Gram, x10), Ok(()));
        let lebih = ke_simpan_x10("Beras", Gram, 50_000.1, Gram).unwrap();
        assert_eq!(periksa_batas(Gram, lebih), Err(GalatSatuan::MelebihiBatas));
        let ml = ke_simpan_x10("Minyak", Ml, 51.0, Liter).unwrap();
        assert_eq!(periksa_batas(Ml, ml), Err(GalatSatuan::MelebihiBatas));
        // satuan hitung tidak dibatasi 50.000
        assert_eq!(periksa_batas(Pcs, 600_000), Ok(()));
    }

    #[test]
    fn simpan_hitung_menerima_gram_hanya_bila_bulat() {
        // 400 g bayam = 2 ikat (bulat); 350 g = 1,75 ikat (pecahan ditolak)
        assert_eq!(ke_simpan_x10("Bayam", Ikat, 400.0, Gram), Ok(20));
        assert_eq!(
            ke_simpan_x10("Bayam", Ikat, 350.0, Gram),
            Err(GalatSatuan::Pecahan)
        );
        // hitung -> hitung lewat bobot: 1 bonggol bawang putih = 10 siung
        assert_eq!(ke_simpan_x10("Bawang Putih", Siung, 1.0, Bonggol), Ok(100));
    }

    #[test]
    fn tampil_kembali_ke_satuan_simpan_bila_tidak_bulat() {
        assert_eq!(tampil("Bayam", Gram, 4_000, Ikat), (2.0, Ikat));
        // 350 g bayam = 1,75 ikat -> tampil sebagai 350 g
        assert_eq!(tampil("Bayam", Gram, 3_500, Ikat), (350.0, Gram));
        // komoditas tanpa bobot: kembali ke satuan simpan
        assert_eq!(tampil("Beras", Gram, 5_000, Ikat), (500.0, Gram));
        assert_eq!(tampil("Beras", Kg, 15_000, Kg), (1.5, Kg));
    }

    #[test]
    fn satuan_sah_untuk_komoditas() {
        assert!(satuan_sah_untuk("Telur Ayam", Pcs));
        assert!(satuan_sah_untuk("Beras", Kg));
        assert!(!satuan_sah_untuk("Beras", Ikat));
        assert!(satuan_sah_untuk("  bayam ", Ikat));
        assert!(!satuan_sah_untuk("Bayam", Siung));
    }
}
