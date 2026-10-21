# Arsitektur pedaree-backend-service

TownHall: https://github.com/Coding-Skuy/Pedaree-TownHall

Dokumen ini menggantikan ringkasan awal template (Axum 0.7.9, skema datar). Keputusan terkunci
ada di `docs/decisions/` dan `AGENTS.md`.

## Lapisan

1. `lib.rs` / `rute()`: router Axum 0.8 (path `{id}`), lapisan Trace + Timeout, fallback JSON.
2. `error.rs`: satu `AppError: IntoResponse` (400/401/404/409/422/429/500) dengan badan
   `{kode, pesan, field}`; ekstraktor `JsonBaku`/`QueryBaku`/`PathBaku` memetakan penolakan ke `AppError`.
3. `state.rs`: `AppState { db: PgPool }`. `config.rs`: konfigurasi dari lingkungan (tanpa rahasia di kode).
4. `db.rs`: koneksi `pedaree` dan `MIGRATOR` (`sqlx::migrate!`, migrasi maju + balik).

## Skema (migrasi 002)

`pantry_items` (kartu bahan), `stok_batch` (unit kedaluwarsa FEFO), `mutasi_stok` (buku kas, 5 jenis:
PAKAI, MASUK, KOREKSI, PINDAH, BUANG). Jumlah bilangan bulat x10 (ADR-0002). Unik
`(household_id, lower(nama))` untuk item aktif; batas 400 item aktif dan 2.000 batch berisi per rumah
tangga lewat trigger. Setiap perubahan `jumlah_aktif` wajib menulis satu `mutasi_stok` dalam transaksi
yang sama (ditegakkan di lapisan layanan mulai PDR-BE-004).

## Catatan

- Pencocokan resep (`kecocokan_resep`) milik alur Pawonee belum dikerjakan; tabelnya dari template
  dibiarkan.
- DB per service: tidak ada akses ke basis data divisi lain.

## Sinyal ke TitipO (PDR-BE-005)

`sinyal.rs` menentukan transisi menipis/habis setelah mutasi dan menulis outbox `sinyal_stok` pada
transaksi yang sama; `pengirim.rs` mengirim ke `POST /sinyal/pedaree-stok` (`X-Service-Key`) dengan
retry 5/30/300 detik lalu DLQ, dan menjalankan `expiry-sweep` harian 06:00 WIB. Dedup 1 sinyal per item
per 24 jam (ADR-0008, ADR-0009). Hanya anjuran restock; tidak ada pembuatan pesanan.
