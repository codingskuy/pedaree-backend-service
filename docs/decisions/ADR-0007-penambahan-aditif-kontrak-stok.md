# ADR-0007 — Penambahan aditif kontrak OpenAPI untuk endpoint stok (tanpa bump versi mayor)

Status: Diterima | Tanggal: 2026-10-21

## Konteks

Mengimplementasikan PDR-BE-004 menuntut beberapa kejelasan yang tidak ada di `docs/api/openapi.yaml` v1.0.0. AGENTS.md: kontrak berubah bersama kode dan dicatat di ADR. Semua perubahan di bawah bersifat aditif/klarifikasi dan tidak mematahkan klien yang mengikuti kontrak awal (web dan app-kmp belum memiliki klien produksi).

## Keputusan

1. `OpnameRequest.stokCatatTerakhir` (opsional): deteksi konflik, `409 KONFLIK_OPNAME` bila berbeda dari stok server. Alasan: kontrak menyebut "Konflik opname dijawab 409" tanpa mekanisme.
2. `PantryItemDetail.satuanStok` (wajib di respons): satuan `stokTampil`. Alasan: stok dalam satuan tampil hanya valid bila konversi tidak berpecahan; selain itu jatuh ke satuan simpan. `PantryItemRingkas.satuanTampil` berperilaku sama.
3. `PantryItem.satuanSimpan` selalu kanonis (kg/liter/sdm/sdt dinormalkan ke gram/ml); `stokMinimum` dalam satuan simpan.
4. Respons `409 IDEMPOTENSI_BENTROK` pada POST bermutasi (ADR-0005); katalog kode galat ditambahkan ke deskripsi.
5. Jumlah <= 0 dijawab `400 VALIDASI_GAGAL`; satuan/konversi/pecahan/batas dijawab `422` dengan kode spesifik.

## Konsekuensi

- Klien KMP/web boleh menggenerasikan model dari `openapi.yaml` branch ini; field baru opsional atau mudah dipetakan.
- `429` tetap ada di kontrak tetapi belum diterapkan server.
