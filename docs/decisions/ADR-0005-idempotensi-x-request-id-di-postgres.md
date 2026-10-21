# ADR-0005 — Idempotensi `X-Request-Id` disimpan di Postgres dalam transaksi yang sama

Status: Diterima | Tanggal: 2026-10-21

## Konteks

Tiket PDR-BE-004 (issue #4) dan OpenAPI: setiap `POST` membawa `X-Request-Id` (UUID); hasil disimpan 24 jam dan kirim ulang tidak boleh menggandakan mutasi. Klien mobile bekerja luring dengan antrean sinkron sehingga pengiriman ulang sering terjadi, termasuk serentak.

## Keputusan

Tabel `idempotensi (household_id, request_id, endpoint, status_kode, respons)` di basis data `pedaree`. Setiap `POST` bermutasi: (1) membuka transaksi, (2) mengambil kunci advisory atas `(household, request_id)` dan mencari hasil tersimpan, (3) bila ada dan belum 24 jam, mengembalikan status + badan asli; (4) bila tidak, menjalankan mutasi lalu menulis hasil pada transaksi yang sama sebelum commit. Galat tidak disimpan (transaksi dibatalkan). ID yang sama untuk operasi lain dijawab `409 IDEMPOTENSI_BENTROK`. Catatan lebih dari 24 jam dihapus saat dipakai ulang dan oleh pembersih tiap jam.

Alternatif ditolak: cache di memori/Redis (hilang saat restart, tidak atomik dengan mutasi).

## Konsekuensi

- Mutasi dan catatan idempotensi sama-sama berhasil atau sama-sama dibatalkan.
- Kunci dibatasi per rumah tangga; ID yang sama di rumah tangga lain independen.
- Badan permintaan tidak dibandingkan; ID sama dengan badan berbeda pada endpoint yang sama mengembalikan hasil lama (sesuai kontrak "hasil asli").
