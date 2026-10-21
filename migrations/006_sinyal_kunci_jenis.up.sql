-- Keputusan 2026-10-21 (ADR-0010, bagian pembaruan): dedup produsen per jenis dan kunci
-- idempoten konsumen TitipO menjadi (item_id, jenis, tanggal). Dedup 24 jam per jenis
-- menjamin keunikan ini; indeks menjadi pagar terakhir bila dua instans berpacu.
CREATE UNIQUE INDEX sinyal_stok_kunci_idempoten_idx ON sinyal_stok (item_id, jenis, tanggal);
