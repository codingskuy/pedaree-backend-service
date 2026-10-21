# ADR-0009 — Job `expiry-sweep` harian 06:00 WIB dan definisi "kedaluwarsa dekat"

Status: Diterima | Tanggal: 2026-10-21

## Konteks

`notifikasi/10-sop-kedaluwarsa.md` bab 3.1 menetapkan job `expiry-sweep` tiap hari pukul 06:00 WIB, dan kontrak bilateral memiliki jenis `kedaluwarsa.dekat`, tetapi tidak ada dokumen yang menyebut ambang "dekat" untuk sinyal ke TitipO.

## Keputusan

1. Job berjalan di proses layanan (tokio): tidur sampai 06:00 WIB (23:00 UTC) berikutnya, lalu menjalankan sweep. Saat proses mulai setelah 06:00 WIB, sweep hari itu disusul sekali.
2. Setiap hari WIB diklaim lewat tabel `expiry_sweep_log` (`INSERT ... ON CONFLICT DO NOTHING`), sehingga banyak instans atau restart tidak menjalankan sweep ganda.
3. ASUMSI: "dekat" = batch AKTIF yang tersisa 0 sampai 3 hari (status SOP SEGERA dan HARI_INI). WASPADA (4 sampai 7 hari) tidak memicu sinyal ke TitipO (relevansi restock 48 jam). Batch tanpa tanggal tidak termasuk.
4. Sweep menghasilkan satu sinyal `kedaluwarsa.dekat` per item (`sisa` = jumlah batch dalam jendela 3 hari, `kedaluwarsa` = batch terdekat), tunduk pada dedup 24 jam (ADR-0008).

## Konsekuensi

- Ambang dapat diubah lewat konstanta `AMBANG_DEKAT_HARI`; perubahan perilaku wajib dicatat di CHANGELOG.
- Push mobile "naik tingkat" (SOP 3.1) bukan bagian dari repo ini.
