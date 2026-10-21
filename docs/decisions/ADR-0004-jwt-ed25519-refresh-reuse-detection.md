# ADR-0004 — JWT akses Ed25519 15 menit dan refresh 7 hari dengan reuse-detection

Status: Diterima | Tanggal: 2026-10-21

## Konteks

Tiket PDR-BE-003 (issue #3). `produk/20-kontrak-api-KMP-mobile.md` dan `produk/21-kontrak-api-web-bun.md` menyebut JWT 30 hari; keputusan terkunci repo (ADR-0001, `AGENTS.md`) adalah akses 15 menit + refresh 7 hari dengan reuse-detection, audiens `pedaree`. Pola acuan: `docs/auth.md` di `lumbung-backend-service` (ed25519).

## Keputusan

1. Akses: JWT EdDSA (Ed25519), umur 900 detik, audiens `pedaree`, klaim `hh` membawa rumah tangga. Kunci dari konfigurasi (`JWT_ED25519_PRIVATE`/`PUBLIC`), bukan kode.
2. Refresh: token buram acak, 7 hari, sekali pakai, disimpan sebagai hash SHA-256; reuse mencabut SEMUA refresh pengguna dan dijawab `401 REFRESH_DIPAKAI_ULANG` (bukan 409; OpenAPI v1.0.0 sudah menetapkan 401 di deskripsi).
3. `X-Household-Id` (wajib per kontrak) divalidasi terhadap klaim `hh`; ketidakcocokan dijawab `401 HOUSEHOLD_TIDAK_COCOK` agar data rumah tangga lain tidak bisa diakses dengan token sah milik orang lain. Kode 403 tidak dipakai karena kontrak hanya mendefinisikan 401.
4. Kata sandi argon2id; login memverifikasi hash palsu bila akun tak ada.

## Konsekuensi

- Pendaftaran pengguna belum di kontrak; provisioning lewat `auth::buat_pengguna` sampai ada tiket.
- Rotasi kunci JWT (`kid`) belum ada; mengganti kunci memaksa login ulang seluruh pengguna.
