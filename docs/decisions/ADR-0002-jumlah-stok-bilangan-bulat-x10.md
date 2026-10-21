# ADR-0002 — Jumlah stok disimpan sebagai bilangan bulat x10

Status: Diterima | Tanggal: 2026-10-21

## Konteks

Tiket PDR-BE-002 (issue #2) menetapkan skema `pantry_items`, `stok_batch`, `mutasi_stok`. `inventori/20-satuan-hortikultura.md` mensyaratkan ketelitian 0,1 g/ml dengan pembulatan setengah ke atas dan satuan hitung bilangan bulat. Dokumen tidak menyebut tipe angka basis data.

## Keputusan

Semua jumlah (stok, minimum, mutasi) disimpan sebagai `BIGINT` dalam sepersepuluh satuan simpan (kolom berakhiran `_x10`). Aritmetika FEFO dan penjumlahan stok memakai bilangan bulat sehingga bebas galat pecahan; API menampilkan `x10 / 10`. Satuan hitung selalu kelipatan 10. Tidak ada `DOUBLE PRECISION` untuk stok.

## Konsekuensi

- Seluruh kode yang membaca/menulis jumlah wajib melewati konversi `x10` di satu modul domain.
- Batas 50.000 g/ml per batch menjadi 500.000 pada kolom `x10`.
