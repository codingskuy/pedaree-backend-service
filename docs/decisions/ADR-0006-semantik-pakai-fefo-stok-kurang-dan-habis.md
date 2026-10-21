# ADR-0006 — Semantik `POST /pantry/consume` saat stok kurang, habis, dan batch kedaluwarsa

Status: Diterima | Tanggal: 2026-10-21

## Konteks

Dokumen sumber tidak konsisten: `produk/10-alur-catat-stok.md` Alur B menyebut "bila stok kurang ... tampilkan kekurangan" sekaligus "stok 0 menolak dengan pesan `stok habis`"; OpenAPI menyebut respons `200` dengan daftar `kurang` dan `422` untuk stok habis. Alur kedaluwarsa menyatakan batch KEDALUWARSA terkunci dari FEFO.

## Keputusan

1. Urutan FEFO: `tanggalKedaluwarsa` terdekat dulu; tanpa tanggal (`null`) di urutan akhir dan tertua dulu (FIFO); seri dipecah oleh `tanggalMasuk` lalu waktu catat.
2. Batch dapat dipakai bila `status = AKTIF`, `jumlahAktif > 0` dan `tanggalKedaluwarsa` null atau >= hari ini (kalender WIB). Batch yang kedaluwarsa HARI INI masih boleh dipakai (status SOP `HARI_INI`); kemarin ke belakang terkunci (hanya bisa dibuang).
3. Stok ada tetapi kurang: pakai semua yang ada, respons `200` dengan `kurang`.
4. Item tanpa stok yang dapat dipakai sama sekali (nol atau hanya batch kedaluwarsa): seluruh permintaan `422 STOK_HABIS`, tidak ada pemotongan (atomik, satu transaksi). Pengecualian: bila item yang sama muncul lagi di baris berikutnya dalam permintaan yang sama dan stoknya baru habis oleh baris sebelumnya, hasilnya `kurang`, bukan `422`.
5. Opname membandingkan stok fisik dengan stok yang dapat dipakai (batch kedaluwarsa dikecualikan; keluarnya lewat Buang).

## Konsekuensi

- Klien yang mengirim banyak item dalam satu permintaan harus menangani `422 STOK_HABIS` per permintaan (field menunjuk baris), atau memisahkan permintaan.
- Bila produk menghendaki pemakaian parsial untuk item kosong, ubah poin 4 lewat ADR baru dan bump kontrak.
