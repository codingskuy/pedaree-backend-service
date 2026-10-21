# Verifikasi provider Pact (PDR-BE-TEST-PACT, #13)

STATUS JUJUR: kerangka. Belum ada pact konsumen Pedaree, jadi TIDAK ADA kontrak yang terverifikasi.
Yang sudah diuji nyata hanya penangani state (`cargo test --features pact-state --test pact_state`).

- Keputusan dan fallback: `docs/decisions/ADR-0012-verifikasi-provider-pact.md`.
- Taruh berkas pact konsumen di `contract/pact/pacts/` (satu berkas per konsumen).
- Jalankan: `make pact` (= `scripts/pact-verifikasi.sh`); keluar 3 berarti tidak diverifikasi.
- State yang dikenal ada di komentar modul `src/pact_state.rs`.
- JANGAN membangun rilis dengan `--features pact-state` (route mengubah data tanpa autentikasi).
