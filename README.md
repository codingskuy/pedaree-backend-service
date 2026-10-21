# pedaree-backend-service — Layanan Stok dan Mutasi (Rust)

Divisi **Pedaree (Smart Pantry)**, org **Coding-Skuy**. Opsi A.

TownHall: https://github.com/Coding-Skuy/Pedaree-TownHall

## Ringkasan

Layanan backend Rust untuk stok pantry dan mutasi keluar-masuk bahan.
Basis data utama bernama `pedaree`. Menyediakan API untuk aplikasi seluler,
web, pipeline data, dan model AI.

## Modul pantry-resep

Kontrak lintas divisi (detail: `docs/MODUL-PANTRY-RESEP.md`):

- **Pemilik modul:** Pawonee.
- **Konsumen modul:** Pedaree (backend memanggil API resep Pawonee untuk
  mencocokkan stok dengan kebutuhan resep, lalu mengembalikan status
  ketersediaan bahan).
- Skema resep tidak disimpan permanen di basis data `pedaree`; yang disimpan
  adalah hasil pencocokan per permintaan beserta rujukan `recipe_id` Pawonee.

## Teknologi (versi dikunci)

- Rust 1.84.1
- axum 0.7.9
- tokio 1.42.0
- sqlx 0.8.3 (PostgreSQL)
- serde 1.0.219, serde_json 1.0.140
- PostgreSQL 16.6
- Redis 7.4.1 (antar-muka pengingat kedaluwarsa)

Lihat `Cargo.toml` sebagai sumber kebenaran versi Rust.

## Skema basis data `pedaree`

- `pantry_items(id, nama, satuan, jumlah, kedaluwarsa, dibuat_pada)`
- `mutasi_stok(id, item_id, jenis, jumlah, alasan, dibuat_pada)`
- `kecocokan_resep(id, recipe_id_pawonee, status, dibuat_pada)`

Migrasi ada di `migrations/`.

## Cara jalan

```bash
cargo build
cargo test
DATABASE_URL=postgres://pedaree:pedaree@localhost:5432/pedaree cargo run
```

## CI

Workflow `.github/workflows/ci.yml` menjalankan `cargo fmt --check`,
`cargo clippy`, dan `cargo test`.

## Dokumentasi API

Kontrak REST v1 ditulis sebagai OpenAPI 3.1 di [`docs/api/openapi.yaml`](docs/api/openapi.yaml) dan dirender dengan Scalar di [`docs/api/index.html`](docs/api/index.html). Sisi produsen kontrak bilateral `pedaree.stok.v1` ke TitipO (`POST /sinyal/pedaree-stok`, anjuran saja) tercantum pada bagian `webhooks`.

- Online (GitHub Pages): https://coding-skuy.github.io/pedaree-backend-service/api/
- Lokal: `bun x serve docs/api` lalu buka alamat yang dicetak (mis. http://localhost:3000).
- Validasi: `bun x @redocly/cli@latest lint docs/api/openapi.yaml`.
- Kontrak hanya berubah lewat bump versi + ADR baru di `docs/decisions/`; sumbernya `produk/20`, `produk/21`, dan BRD `30-demand-metrik.md` di Pedaree-TownHall v1.0.0.

## Uji properti API (Schemathesis)

`scripts/schemathesis.sh` menjalankan [Schemathesis](https://schemathesis.readthedocs.io) terhadap
backend lokal memakai `docs/api/openapi.yaml` (tiket PDR-BE-TEST-SCHEMATHESIS). Skrip membuat
basis data sementara `pedaree_schemathesis`, kunci Ed25519 sementara, akun uji lewat
`buat-akun` (ADR-0011) dengan household `hh_01J9Z8QK2M4B`, menjalankan server, login, lalu menguji.
Semua dibersihkan saat selesai; tidak ada rahasia di repo. Telemetri mati
(`E2E_TELEMETRY_DISABLED=1`, tanpa `--report schemathesis`); tidak ada panggilan ke layanan luar.

```bash
uv tool install schemathesis        # sekali saja (diuji dengan 4.32.3)
make schemathesis                   # atau: ./scripts/schemathesis.sh
MAX_EXAMPLES=100 ST_EXTRA="--seed 1" ./scripts/schemathesis.sh
```

Prasyarat: Postgres lokal yang bisa `createdb`/`dropdb` (`PGHOST`, `PGUSER` bila perlu), `openssl`, `cargo`.

Cakupan dan batas yang perlu diketahui:

- Operasi yang belum diimplementasikan server dikecualikan lewat `ST_EXCLUDE_PATHS`
  (`/demand/*`, `/web/*`); tambahkan kembali begitu endpoint hadir.
- `schemathesis.toml`: 422 bisnis (satuan, batas 50.000 g/ml, sisa stok) diterima sebagai penolakan
  terdokumentasi pada `POST /pantry/*`; generator tidak membuat NUL (U+0000), yang dipetakan
  server ke 400 dan diuji unit.
- Operasi yang butuh ID nyata (`consume`, `opname`, `pindah`, `buang`, `GET /pantry/items/{id}`)
  sebagian besar menerima 404 karena ID acak; jalur sukses dicakup uji integrasi `cargo test`,
  bukan Schemathesis. Fase stateful belum dinyalakan.
