# Auth Pedaree

Pola mengikuti `docs/auth.md` di `lumbung-backend-service`. Keputusan terkunci: akses 15 menit +
refresh 7 hari (bukan JWT 30 hari di `produk/20`/`21`, lihat `AGENTS.md` bagian ketidaksesuaian).

## JWT akses

- Umur 15 menit (`exp - iat = 900`), EdDSA (Ed25519), audiens `pedaree`, penerbit `JWT_ISSUER`.
- Klaim: `iss`, `sub` (`usr_` + ULID), `aud`, `hh` (rumah tangga), `exp`, `iat`, `jti`.
- Diverifikasi ekstraktor `auth::Pengguna`: tanda tangan, audiens, penerbit, kedaluwarsa, algoritma
  (hanya EdDSA). Token audiens lain atau algoritma lain: `401 TIDAK_SAH`.
- Header `X-Household-Id` wajib dan harus sama dengan klaim `hh` (`401 HOUSEHOLD_TIDAK_COCOK`);
  hilang: `400`.

## Refresh token

- Buram (`rt_` + 32 byte acak), umur 7 hari, sekali pakai; hanya hash SHA-256 disimpan.
- `POST /auth/refresh` merotasi token dalam keluarga yang sama.
- Reuse-detection: token yang sudah dipakai/dicabut muncul lagi, seluruh refresh milik pengguna
  dicabut dan `401 REFRESH_DIPAKAI_ULANG`; pengguna wajib login ulang. Pemakaian serentak satu
  token diserialkan dengan `SELECT ... FOR UPDATE` (hanya satu menang).

## Kata sandi

argon2id (parameter bawaan crate `argon2`), format PHC. Login memverifikasi hash palsu bila akun
tidak ada agar waktu respons tidak membocorkan keberadaan akun.

## Konfigurasi (lingkungan, tidak di kode)

| Variabel | Keterangan |
|---|---|
| `JWT_ED25519_PRIVATE` | PEM PKCS#8 kunci privat (boleh satu baris dengan `\n` literal). Wajib. |
| `JWT_ED25519_PUBLIC` | PEM SPKI kunci publik. Wajib. |
| `JWT_ISSUER` | Default `pedaree-backend-service`. |
| `PEDAREE_SERVICE_KEY` | Service key least-privilege untuk `X-Service-Key` ke TitipO (hanya `pedaree.stok.v1`). Simpan sebagai secret, jangan commit. |
| `TITIPO_BASE_URL` | Alamat dasar TitipO. |

Membuat pasangan kunci: `openssl genpkey -algorithm ed25519 -out priv.pem && openssl pkey -in priv.pem -pubout -out pub.pem`.

## Belum tercakup (di luar tiket PDR-BE-003)

- Pendaftaran/provisioning pengguna belum ada di kontrak v1.0.0; fungsi `auth::buat_pengguna` hanya
  untuk provisioning dan uji.
- Pembatasan laju login (`429`), logout/cabut manual, dan rotasi kunci JWT (`kid`).
- `X-Service-Key` masuk (SSR web) belum divalidasi.
