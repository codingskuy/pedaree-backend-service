CREATE TABLE IF NOT EXISTS pantry_items (
    id UUID PRIMARY KEY,
    nama TEXT NOT NULL,
    satuan TEXT NOT NULL,
    jumlah DOUBLE PRECISION NOT NULL CHECK (jumlah >= 0),
    kedaluwarsa DATE NOT NULL,
    dibuat_pada TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS mutasi_stok (
    id UUID PRIMARY KEY,
    item_id UUID NOT NULL REFERENCES pantry_items(id),
    jenis TEXT NOT NULL CHECK (jenis IN ('masuk', 'keluar')),
    jumlah DOUBLE PRECISION NOT NULL CHECK (jumlah > 0),
    alasan TEXT NOT NULL DEFAULT '',
    dibuat_pada TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS kecocokan_resep (
    id UUID PRIMARY KEY,
    recipe_id_pawonee TEXT NOT NULL,
    status TEXT NOT NULL,
    dibuat_pada TIMESTAMPTZ NOT NULL DEFAULT now()
);
