//! Subperintah operator (ADR-0011): akun dan household dibuat operator, bukan pendaftaran
//! publik. Pemakaian: `pedaree-backend-service buat-akun --id-pengguna <id> [--household <hh_...>]`.
//! Kata sandi dibaca dari `PEDAREE_KATA_SANDI` atau baris pertama stdin, tidak pernah dari
//! argumen baris perintah (agar tidak tampil di daftar proses/riwayat shell).

use argon2::Argon2;
use sqlx::PgPool;

use crate::auth::buat_pengguna;
use crate::error::AppError;

/// Panjang minimum kata sandi yang dibuat operator.
pub const SANDI_MINIMAL: usize = 12;

#[derive(Debug, PartialEq, Eq)]
pub struct PermintaanAkun {
    pub id_pengguna: String,
    /// `None` = buat household baru; diisi = pakai household yang ada (mis. hasil impor Pawonee).
    pub household_id: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Perintah {
    BuatAkun(PermintaanAkun),
}

#[derive(Debug, PartialEq, Eq)]
pub struct AkunDibuat {
    pub pengguna_id: String,
    pub household_id: String,
    pub household_baru: bool,
}

const BANTUAN: &str =
    "pemakaian: pedaree-backend-service buat-akun --id-pengguna <id> [--household <hh_...>]";

/// Mengurai argumen setelah nama biner. `Ok(None)` = tidak ada subperintah (jalankan server).
pub fn urai(args: &[String]) -> Result<Option<Perintah>, String> {
    let Some(nama) = args.first() else {
        return Ok(None);
    };
    if nama != "buat-akun" {
        return Err(format!("subperintah tidak dikenal: {nama}\n{BANTUAN}"));
    }
    let mut id_pengguna = None;
    let mut household_id = None;
    let mut it = args[1..].iter();
    while let Some(bendera) = it.next() {
        let nilai = it
            .next()
            .ok_or_else(|| format!("{bendera} butuh nilai\n{BANTUAN}"))?;
        match bendera.as_str() {
            "--id-pengguna" => id_pengguna = Some(nilai.clone()),
            "--household" => household_id = Some(nilai.clone()),
            lain => return Err(format!("opsi tidak dikenal: {lain}\n{BANTUAN}")),
        }
    }
    let id_pengguna = id_pengguna.ok_or_else(|| format!("--id-pengguna wajib\n{BANTUAN}"))?;
    Ok(Some(Perintah::BuatAkun(PermintaanAkun {
        id_pengguna,
        household_id,
    })))
}

fn household_valid(id: &str) -> bool {
    id.strip_prefix("hh_").is_some_and(|s| {
        !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric())
    })
}

/// Membuat akun (dan household baru bila `household_id` kosong). Tidak ada endpoint HTTP
/// yang memanggil fungsi ini.
pub async fn buat_akun(
    db: &PgPool,
    permintaan: &PermintaanAkun,
    kata_sandi: &str,
    argon: &Argon2<'_>,
) -> Result<AkunDibuat, AppError> {
    let id_pengguna = permintaan.id_pengguna.trim();
    if !(3..=80).contains(&id_pengguna.chars().count()) {
        return Err(AppError::validasi(
            "id-pengguna harus 3 sampai 80 karakter",
            Some("id-pengguna"),
        ));
    }
    if kata_sandi.chars().count() < SANDI_MINIMAL {
        return Err(AppError::validasi(
            "kata sandi minimal 12 karakter",
            Some("kata-sandi"),
        ));
    }
    let (household_id, household_baru) = match permintaan.household_id.as_deref() {
        Some(id) if household_valid(id) => (id.to_owned(), false),
        Some(_) => {
            return Err(AppError::validasi(
                "household harus berformat hh_<huruf/angka>",
                Some("household"),
            ))
        }
        None => (format!("hh_{}", ulid::Ulid::new()), true),
    };
    let pengguna_id = buat_pengguna(db, &household_id, id_pengguna, kata_sandi, argon).await?;
    Ok(AkunDibuat {
        pengguna_id,
        household_id,
        household_baru,
    })
}

/// Membaca kata sandi dari env atau baris pertama stdin.
pub fn baca_kata_sandi() -> Result<String, String> {
    if let Ok(s) = std::env::var("PEDAREE_KATA_SANDI") {
        if !s.is_empty() {
            return Ok(s);
        }
    }
    let mut baris = String::new();
    std::io::stdin()
        .read_line(&mut baris)
        .map_err(|e| format!("gagal membaca stdin: {e}"))?;
    Ok(baris.trim_end_matches(['\r', '\n']).to_owned())
}

#[cfg(test)]
mod uji {
    use super::*;

    fn a(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn urai_tanpa_subperintah_menjalankan_server() {
        assert_eq!(urai(&[]), Ok(None));
    }

    #[test]
    fn urai_buat_akun() {
        let p = urai(&a(&[
            "buat-akun",
            "--id-pengguna",
            "budi",
            "--household",
            "hh_ABC1",
        ]));
        assert_eq!(
            p,
            Ok(Some(Perintah::BuatAkun(PermintaanAkun {
                id_pengguna: "budi".into(),
                household_id: Some("hh_ABC1".into()),
            })))
        );
        assert!(urai(&a(&["buat-akun"])).is_err());
        assert!(urai(&a(&["daftar"])).is_err());
        assert!(urai(&a(&["buat-akun", "--id-pengguna"])).is_err());
        assert!(urai(&a(&[
            "buat-akun",
            "--id-pengguna",
            "x",
            "--sandi",
            "rahasia"
        ]))
        .is_err());
    }

    #[test]
    fn format_household() {
        assert!(household_valid("hh_01J9Z8QK2M4B"));
        assert!(!household_valid("hh_"));
        assert!(!household_valid("usr_1"));
        assert!(!household_valid("hh_a'; DROP"));
    }
}
