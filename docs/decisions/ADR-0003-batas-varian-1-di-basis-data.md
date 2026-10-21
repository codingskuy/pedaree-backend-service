# ADR-0003 — Batas Varian 1 (400 item, 2.000 batch) ditegakkan di basis data

Status: Diterima | Tanggal: 2026-10-21

## Konteks

`inventori/10-model-stok-dapur.md` bab 4 membatasi 400 PantryItem aktif dan 2.000 batch per rumah tangga. Penegakan hanya di kode aplikasi rentan terhadap balapan dua permintaan serentak.

## Keputusan

Batas dijaga trigger Postgres dengan kunci advisory per rumah tangga (`pg_advisory_xact_lock`); pelanggaran melempar SQLSTATE khusus `PDI01` (item) dan `PDB01` (batch) yang dipetakan `AppError` ke `422 BATAS_ITEM_AKTIF` / `422 BATAS_BATCH`.

ASUMSI (dokumen tidak menjelaskan): "2.000 batch" dihitung dari batch yang masih berisi (`jumlah_aktif_x10 > 0`); batch riwayat berstatus HABIS/DIBUANG tidak dihitung, agar riwayat tidak memblokir pencatatan baru. Item diarsipkan tidak dihitung terhadap 400.

## Konsekuensi

- Pengaktifan kembali item terarsip juga diperiksa terhadap batas.
- Bila produk menghendaki arti lain untuk "2.000 batch" (semua batch termasuk riwayat), cukup ubah kondisi trigger lewat migrasi baru.
