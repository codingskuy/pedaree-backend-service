# ADR-0010 — Lingkup dedup sinyal (per item untuk TitipO) dan verifikasi kunci idempoten `item_id + tanggal`

Status: Diterima; keputusan 1 dan 3 DIGANTI oleh bagian Pembaruan di bawah | Tanggal: 2026-10-21

## Konteks

Melengkapi ADR-0008 (tidak menggantikannya). Dua dokumen berbeda soal lingkup dedup:

- BR-301 (Pedaree `BRD/30-demand-metrik.md`): "masing-masing (menipis, habis) maksimal 1 per item per 24 jam" = per (item, jenis).
- KONTEKS-LINTAS-DIVISI §3.1 (terkunci 2026-10-07): "maks 1 per item per 24 jam" dan kunci idempoten konsumen `item_id + tanggal` = per item.

Keputusan manusia: untuk data yang mengalir ke divisi lain berlaku KONTEKS §3.1 (per item); BR-301 (per jenis) hanya untuk data internal Pedaree.

## Keputusan

1. Antrean keluar ke TitipO (`sinyal_stok` -> `POST /sinyal/pedaree-stok`) memakai dedup PER ITEM lintas jenis (perilaku ADR-0008, tidak berubah).
2. Saat ini TIDAK ADA data sinyal yang bersifat internal-saja: tabel `sinyal_stok` hanya outbox. Aturan per jenis (BR-301) belum punya tempat berlaku dan tidak diimplementasikan; bila kelak ada riwayat/metrik internal (mis. `/demand/signals`, BR-303), catat per jenis di tabel terpisah dari outbox.
3. Tabrakan pada data internal TIDAK diputuskan di sini (butuh keputusan manusia), contoh konkret di bawah.

### Contoh tabrakan (item `itm_X`, minimum 250 g, WIB)

| Waktu | Kejadian | Per item (KONTEKS §3.1) | Per jenis (BR-301) |
|---|---|---|---|
| 08:00 | stok 500 -> 200 g | `stok.menipis` dikirim | `stok.menipis` dicatat |
| 19:00 | stok 200 -> 0 g | `stok.habis` DITEKAN | `stok.habis` dicatat (jenis berbeda) |
| hasil | | 1 sinyal; TitipO tidak tahu item sudah habis | 2 sinyal pada metrik internal |

Bila kedua aturan dipakai pada data internal yang sama, metrik internal (per jenis) tidak sama dengan yang diterima TitipO (per item); metrik seperti BR-303 ("60% sinyal MENIPIS berujung pesan") tetap bisa dihitung dari catatan per jenis tanpa mengubah antrean keluar.

## Verifikasi idempotensi TitipO `item_id + tanggal`

Sumber: TitipO `pesanan/10-alur-titip-harian.md` §1-§2 dan §6, `pesanan/20-model-data.md`, `titipo-backend-service/docs/api/openapi.yaml` (`/sinyal/pedaree-stok`).

- Kunci itu milik SINYAL, bukan pesanan. Model pesanan TitipO tidak punya batas unik per item per hari: satu rumah tangga boleh membuat beberapa pesanan sehari (jendela 05.00-20.00; satu vendor per keranjang; hanya batas 50 baris/20 kg per pesanan). Jadi item yang sama BISA dipesan dua kali secara sah dalam sehari; kunci sinyal tidak menghalanginya.
- Untuk sinyal: dengan dedup produsen 24 jam, dua sinyal item yang sama hanya mungkin lolos bila jaraknya >= 24 jam, sehingga `tanggal` (hari WIB) selalu berbeda. Tidak ada tabrakan palsu pada sinyal sejenis.
- Satu-satunya tabrakan nyata: jenis berbeda dalam satu hari (contoh di atas). Karena produsen sudah menekannya, konsumen tidak pernah melihatnya; efeknya hilangnya eskalasi menipis -> habis dan kedaluwarsa.dekat yang menyusul stok.menipis.
- Alternatif bila produk ingin eskalasi tetap sampai (TIDAK diterapkan; kontrak TitipO tidak diubah di sini, perlu PR TownHall dahulu + dual-read 30 hari): (a) kunci `item_id + tanggal + jenis`; (b) kunci `item_id + waktu_kirim` (timestamp UTC di payload); (c) nomor urut per item per hari (`urutan`), konsumen menyimpan nilai tertinggi. Pilihan (a) paling kecil perubahannya dan tetap per jenis seperti BR-301.

## Konsekuensi

- Tidak ada perubahan kode atau kontrak API dalam ADR ini.
- Keputusan manusia yang masih terbuka: apakah eskalasi jenis dalam satu hari harus sampai ke TitipO (butuh perubahan kunci di kedua sisi).

## Pembaruan 2026-10-21 — dedup per jenis dan kunci `item_id + jenis + tanggal` (keputusan user, final)

Menutup keputusan manusia yang terbuka di atas. Bagian lama dipertahankan sebagai riwayat.

1. Dedup produsen PER JENIS: `stok.menipis`, `stok.habis`, dan `kedaluwarsa.dekat` masing-masing maksimal 1 sinyal per item per 24 jam. Kasus contoh (minimum 250 g, 500 -> 200 g pukul 08:00, lalu -> 0 pukul 19:00) kini menghasilkan DUA sinyal (kolom "per jenis" pada tabel di atas).
2. Kunci idempoten konsumen menjadi `item_id + jenis + tanggal` (tanggal kalender WIB). Duplikat tetap `200 duplikat_diabaikan`.
3. Implementasi: kueri dedup di `sinyal::antrekan` menambah `jenis = $3`; migrasi `006_sinyal_kunci_jenis` menambah indeks unik `(item_id, jenis, tanggal)` pada `sinyal_stok`; uji: `menipis_pagi_lalu_habis_malam_hari_sama_menghasilkan_dua_sinyal` dan `maksimal_satu_sinyal_per_item_per_jenis_per_24_jam`. Efek samping: `kedaluwarsa.dekat` dari sweep 06:00 tidak lagi ditekan oleh sinyal menipis/habis item yang sama.
4. Batasan kunci: dua sinyal jenis sama hanya lolos bila berjarak >= 24 jam, sehingga `tanggal` WIB pasti berbeda; indeks unik aman bagi data lama (dedup lama lebih ketat).

### Konsumen kunci lama (bukti per 2026-10-21)

- Produksi: BELUM ADA konsumen produksi kunci `item_id + tanggal`. `titipo-backend-service` upstream `main` tidak punya handler `/sinyal/pedaree-stok` (hanya openapi); implementasi penerima baru ada di PR #12 (branch `feat/ttp-be-005-sinyal-pedaree-stok`, belum merge) dengan `ON CONFLICT (item_id, tanggal) DO NOTHING` pada tabel `sinyal_restock`. Tidak ada rilis/tag di kedua repo; `titipo-infra-devops` tidak menyebut Pedaree/sinyal; tidak ada data sinyal produksi. Pedaree sendiri belum mengirim ke produksi.
- Maka masa baca ganda 30 hari (dual-read) TIDAK perlu: tidak ada data/penerima produksi yang berkunci lama. Yang wajib: PR #12 TitipO diubah ke `ON CONFLICT (item_id, jenis, tanggal)` sebelum merge; bila tidak, `stok.habis` sesudah `stok.menipis` di hari yang sama dijawab `200 duplikat_diabaikan` dan eskalasi hilang tanpa tanda.
- Perubahan dokumen TownHall (KONTEKS-LINTAS-DIVISI §3.1, TitipO `pesanan/10` §6) bukan bagian PR ini; usulannya ditangani terpisah oleh manajer.
