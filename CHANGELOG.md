# Changelog — pedaree-backend-service

Format mengikuti Keep a Changelog; versi mengikuti TownHall.

## [Belum dirilis]

### Ditambahkan
- PDR-BE-TEST-PACT (kerangka): ADR-0012, fitur `pact-state` (state handler `/_pact/state`), `scripts/pact-verifikasi.sh`, `contract/pact/`. Belum ada pact konsumen, belum ada verifikasi kontrak.
- Fondasi AI-dev v1.0.0: `AGENTS.md`, `CLAUDE.md`, ADR-0001, `docs/ARSITEKTUR-AI-DEV.md` (2026-10-14).
- Kontrak OpenAPI 3.1 `docs/api/openapi.yaml` + referensi Scalar `docs/api/index.html`.
- PDR-BE-002: kerangka Axum 0.8 (`AppState`, `AppError` baku, `JsonBaku`), konfigurasi env, migrasi sqlx maju+balik, skema FEFO `pantry_items`/`stok_batch`/`mutasi_stok` dengan batas 400 item / 2.000 batch (ADR-0002, ADR-0003); CI memakai Postgres 16.

### Diubah
- Axum 0.7.9 naik ke 0.8 (path `{id}`); skema datar template diganti skema batch FEFO; `001_init.sql` menjadi pasangan `.up.sql`/`.down.sql` (isi tidak berubah).
- PDR-BE-003: auth `POST /auth/login` dan `/auth/refresh`; JWT akses Ed25519 15 menit audiens `pedaree`, refresh 7 hari sekali pakai dengan reuse-detection (cabut seluruh refresh pengguna), argon2id, ekstraktor `Pengguna` (validasi audiens/penerbit/`X-Household-Id`), konfigurasi service key lewat env (ADR-0004, `docs/auth.md`).
- PDR-BE-004: endpoint stok `/pantry/*` (item, batch, consume FEFO, opname, pindah, buang, expiring), modul satuan 13 baku + bobot acuan, idempotensi `X-Request-Id` 24 jam di Postgres, openapi.yaml diperbarui (ADR-0005, ADR-0006, ADR-0007).
- Sinyal: dedup per jenis (menipis dan habis sama-sama terkirim) dan kunci idempoten `item_id + jenis + tanggal`; migrasi 006 indeks unik; ADR-0010 bagian pembaruan.
- PDR-BE-005: sinyal `pedaree.stok.v1` ke TitipO (outbox `sinyal_stok`, pemicu menipis/habis, dedup 1 per item per 24 jam, `expiry-sweep` 06:00 WIB untuk `kedaluwarsa.dekat`, pengirim dengan `X-Service-Key`, retry 5/30/300 detik lalu DLQ) (ADR-0008, ADR-0009).
