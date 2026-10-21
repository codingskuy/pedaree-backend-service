# ADR-0012 — Verifikasi provider Pact: pact_verifier_cli + state handler, dengan fallback

Status: Diterima | Tanggal: 2026-10-22

## Konteks

Backend adalah provider untuk konsumen web (SvelteKit) dan app-kmp. Tiket PDR-BE-TEST-PACT (#13) meminta verifikasi provider terhadap pact konsumen. Per tanggal ini BELUM ADA berkas pact konsumen untuk Pedaree (web dan app-kmp belum menerbitkan pact), jadi verifikasi nyata belum dapat dijalankan. Yang bisa diputuskan dan disiapkan adalah alat, bentuk state handler, dan jalur cadangan. Kontrak sumber kebenaran tetap `docs/api/openapi.yaml`; Schemathesis (PR #15) sudah memeriksa server terhadap spec.

## Keputusan

1. **Verifier utama: `pact_verifier_cli`** (Rust, pact-foundation; `cargo install --locked pact_verifier_cli`) memverifikasi berkas pact di `contract/pact/pacts/*.json` terhadap server lokal, dipanggil lewat `scripts/pact-verifikasi.sh`. Alasan: satu toolchain (Rust) tanpa menambah Node ke repo backend, dan tidak menambah dependensi ke `Cargo.toml`.
2. **State handler** `src/pact_state.rs` di balik fitur Cargo `pact-state` (mati secara bawaan, TIDAK BOLEH aktif di rilis), route `POST /_pact/state` (format `{state, params, action}` bawaan verifier). State awal: `pengguna terdaftar`, `item dengan stok`, `dapur kosong`; state baru ditambahkan bersama pact konsumen. Token tidak boleh tertanam di pact (akses 15 menit): skrip membuat akun lewat state lalu login dan meneruskan `Authorization`/`X-Household-Id` lewat `--header`.
3. **Kode keluar jujur**: skrip keluar 3 ("TIDAK DIVERIFIKASI") bila tidak ada berkas pact atau CLI belum terpasang; 3 bukan lulus. Tidak ada klaim lulus tanpa pact konsumen nyata.
4. **Fallback berurutan** bila langkah di atas tidak layak:
   a. verifier pact-js (Node, pustaka native Pact Rust) dengan state handler yang sama lewat HTTP; dipakai repo saudara di org ini;
   b. bila konsumen app-kmp tidak dapat menerbitkan pact (pact-jvm hanya untuk target JVM/Android, tidak untuk Kotlin/Native iOS), konsumen KMP cukup memakai klien hasil generate dari `openapi.yaml` + Schemathesis di sisi provider; pact hanya dari web;
   c. pact konsumen yang berisi matcher tidak didukung CLI: turunkan ke contoh tetap atau ke Schemathesis.
5. Tidak memakai Pact Broker; berkas pact di-commit di `contract/pact/pacts/`. Telemetri: `E2E_TELEMETRY_DISABLED=1`; tidak ada data keluar.

## Konsekuensi

- Dijalankan nyata: hanya uji state handler (`cargo test --features pact-state --test pact_state`). Hanya ditulis: `scripts/pact-verifikasi.sh` dengan CLI (belum pernah dijalankan, flag CLI belum diverifikasi pada versi terpasang) dan alur pact konsumen.
- Satu route uji ada di kode produksi di balik fitur kompilasi; CI rilis wajib membangun tanpa `--features pact-state`.
- Pekerjaan lanjutan: menunggu pact web/app-kmp, lalu menambah state, menjalankan skrip, dan menambah workflow CI manual (siklus 3).
