# ADR-0011 — Akun dan household dibuat operator (Opsi C) untuk v1.0.0

Status: Diterima | Tanggal: 2026-10-21

## Konteks

Kontrak backend v1.0.0 tidak punya pendaftaran pengguna; hanya ada `auth::buat_pengguna` internal. Tiga opsi telah diajukan: A (Pedaree punya akun sendiri dan pendaftaran), B (layanan identitas bersama/SSO, Pedaree hanya memvalidasi), C (operator membuat akun, household dipetakan dari app induk). Keputusan manusia: Opsi C untuk v1.0.0.

## Keputusan

1. Akun dan household dibuat operator/admin lewat subperintah biner `pedaree-backend-service buat-akun --id-pengguna <id> [--household <hh_...>]` (modul `cli`, memakai `auth::buat_pengguna`), atau diimpor dari Pawonee dengan memberi `--household` yang sudah ada. Tanpa `--household`, household baru (`hh_<ULID>`) dibuat.
2. Kata sandi dibaca dari `PEDAREE_KATA_SANDI` atau stdin (bukan argumen), minimal 12 karakter, disimpan argon2id.
3. Pedaree hanya melayani login/refresh. TIDAK ada endpoint pendaftaran atau pembuatan household (dijaga uji `tidak_ada_endpoint_pendaftaran_publik`).
4. Klaim `aud` JWT tetap `pedaree`; header `X-Household-Id` tetap harus sama dengan klaim `hh`.

## Konsekuensi

- Tercepat untuk v1.0.0 dan permukaan serangan terkecil; tidak ada pendaftaran mandiri, jadi onboarding pengguna adalah pekerjaan operasional manual.
- Impor massal dari Pawonee belum otomatis: operator menjalankan subperintah per akun dengan household dari Pawonee; skrip massal dapat ditambah tanpa mengubah kontrak.
- Tidak ada tabel household: household hanyalah id teks pada `pengguna` dan data stok; kesalahan ketik household membuat household baru yang terpisah (operator harus teliti).
- Layar login web/app-kmp tidak perlu layar daftar pada v1.0.0.
- Opsi A (akun Pedaree sendiri dengan pendaftaran) dan Opsi B (SSO dengan audiens per service) tetap terbuka untuk versi berikutnya; keduanya memerlukan ADR baru yang menggantikan ADR ini.
