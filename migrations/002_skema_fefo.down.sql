DROP TRIGGER IF EXISTS stok_batch_batas_ins ON stok_batch;
DROP FUNCTION IF EXISTS batasi_batch();
DROP TRIGGER IF EXISTS pantry_items_batas_upd ON pantry_items;
DROP TRIGGER IF EXISTS pantry_items_batas_ins ON pantry_items;
DROP FUNCTION IF EXISTS batasi_item_aktif();
DROP TABLE IF EXISTS mutasi_stok;
DROP TABLE IF EXISTS stok_batch;
DROP TABLE IF EXISTS pantry_items;

-- Pulihkan skema datar template (001).
CREATE TABLE pantry_items (
    id UUID PRIMARY KEY,
    nama TEXT NOT NULL,
    satuan TEXT NOT NULL,
    jumlah DOUBLE PRECISION NOT NULL CHECK (jumlah >= 0),
    kedaluwarsa DATE NOT NULL,
    dibuat_pada TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE mutasi_stok (
    id UUID PRIMARY KEY,
    item_id UUID NOT NULL REFERENCES pantry_items(id),
    jenis TEXT NOT NULL CHECK (jenis IN ('masuk', 'keluar')),
    jumlah DOUBLE PRECISION NOT NULL CHECK (jumlah > 0),
    alasan TEXT NOT NULL DEFAULT '',
    dibuat_pada TIMESTAMPTZ NOT NULL DEFAULT now()
);
