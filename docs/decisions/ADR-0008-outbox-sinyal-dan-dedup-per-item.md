# ADR-0008 — Sinyal `pedaree.stok.v1` lewat outbox; dedup 1 per item per 24 jam lintas jenis

Status: Diterima | Tanggal: 2026-10-21

## Konteks

Tiket PDR-BE-005 (issue #5). BR-301 menyebut "masing-masing maksimal 1 sinyal per item per 24 jam" (`inventori/10` bab 3), sedangkan kontrak bilateral (KONTEKS §3.1) menyebut "maks 1 per item per 24 jam" dan konsumen TitipO mendedup dengan kunci `item_id + tanggal` (lintas jenis). Pengiriman ke TitipO tidak boleh menggagalkan pencatatan stok.

## Keputusan

1. Sinyal ditulis ke tabel outbox `sinyal_stok` pada transaksi yang sama dengan mutasi stok; pengirim latar belakang mengirimnya ke `POST /sinyal/pedaree-stok` dengan `X-Service-Key` dari konfigurasi. Kegagalan sementara: retry 3x (5/30/300 detik) lalu `DLQ`; kegagalan permanen (4xx selain 408/429): langsung `DLQ`; `200 duplikat_diabaikan` = sukses.
2. Dedup produsen: sinyal baru untuk item yang sama ditekan bila sudah ada sinyal item itu dalam 24 jam terakhir, apa pun jenisnya. Alasan: konsumen akan mengabaikan jenis kedua pada tanggal yang sama (kunci `item_id + tanggal`), jadi mengirimnya hanya menghasilkan `200 duplikat_diabaikan`. Konsekuensinya menipis lalu habis dalam 24 jam hanya menghasilkan satu sinyal (menipis).
3. Pemicu: stok yang dapat dipakai (batch AKTIF belum kedaluwarsa) turun melewati ambang (`stok.menipis`) atau menjadi nol (`stok.habis`) setelah pakai, opname, atau buang. Menambah stok tidak memicu apa pun.

## Konsekuensi

- Bila produk memaksudkan dedup per (item, jenis), ubah kueri dedup di `sinyal::antrekan` dan koordinasikan kunci idempoten dengan TitipO (KONTEKS §3.1) lewat PR TownHall terlebih dulu.
- Sinyal tidak pernah membuat pesanan; satu-satunya jalur keluar adalah `/sinyal/pedaree-stok` (dijaga uji).
- `/demand/signals` (riwayat) dapat dibangun di atas `sinyal_stok` pada tiket lain.
