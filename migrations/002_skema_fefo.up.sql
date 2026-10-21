-- PDR-BE-002: skema stok FEFO (PantryItem, StokBatch, MutasiStok).
-- Sumber: inventori/10-model-stok-dapur.md bab 2 dan 4.
-- Jumlah disimpan sebagai bilangan bulat BIGINT dalam sepersepuluh satuan simpan
-- (kolom *_x10), agar aritmetika stok bebas galat pecahan; ketelitian 0,1 g/ml.

-- Skema datar template diganti (tabel lama belum berisi data produksi).
DROP TABLE IF EXISTS mutasi_stok;
DROP TABLE IF EXISTS pantry_items;

CREATE TABLE pantry_items (
    id TEXT PRIMARY KEY CHECK (id LIKE 'itm\_%'),
    household_id TEXT NOT NULL CHECK (household_id LIKE 'hh\_%'),
    nama TEXT NOT NULL CHECK (char_length(btrim(nama)) BETWEEN 3 AND 60),
    kategori TEXT NOT NULL CHECK (kategori IN
        ('SAYUR', 'BUMBU', 'BUAH', 'PROTEIN', 'KERING', 'CAIR', 'BEKU', 'SISA')),
    satuan_simpan TEXT NOT NULL CHECK (satuan_simpan IN
        ('gram', 'kg', 'ml', 'liter', 'sdm', 'sdt', 'pcs', 'siung', 'buah', 'ikat', 'batang', 'bonggol', 'lembar')),
    satuan_tampil TEXT NOT NULL CHECK (satuan_tampil IN
        ('gram', 'kg', 'ml', 'liter', 'sdm', 'sdt', 'pcs', 'siung', 'buah', 'ikat', 'batang', 'bonggol', 'lembar')),
    stok_minimum_x10 BIGINT NOT NULL CHECK (stok_minimum_x10 >= 0),
    umur_simpan_hari_default INTEGER NOT NULL CHECK (umur_simpan_hari_default BETWEEN 0 AND 365),
    lokasi_default TEXT NOT NULL CHECK (lokasi_default IN
        ('KULKAS_BAWAH', 'KULKAS_ATAS', 'FREEZER', 'SUHU_RUANG', 'BASKOM_BAWANG')),
    foto_url TEXT,
    arsip BOOLEAN NOT NULL DEFAULT FALSE,
    dibuat_pada TIMESTAMPTZ NOT NULL DEFAULT now(),
    diubah_pada TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Unik (householdId, lower(nama)) hanya untuk item aktif (arsip = false).
CREATE UNIQUE INDEX pantry_items_nama_aktif_uq
    ON pantry_items (household_id, lower(btrim(nama)))
    WHERE NOT arsip;
CREATE INDEX pantry_items_household_idx ON pantry_items (household_id) WHERE NOT arsip;

CREATE TABLE stok_batch (
    id TEXT PRIMARY KEY CHECK (id LIKE 'btc\_%'),
    household_id TEXT NOT NULL CHECK (household_id LIKE 'hh\_%'),
    pantry_item_id TEXT NOT NULL REFERENCES pantry_items (id),
    jumlah_awal_x10 BIGINT NOT NULL CHECK (jumlah_awal_x10 > 0),
    jumlah_aktif_x10 BIGINT NOT NULL CHECK (jumlah_aktif_x10 >= 0),
    tanggal_masuk DATE NOT NULL,
    tanggal_kedaluwarsa DATE,
    lokasi TEXT NOT NULL CHECK (lokasi IN
        ('KULKAS_BAWAH', 'KULKAS_ATAS', 'FREEZER', 'SUHU_RUANG', 'BASKOM_BAWANG')),
    sumber TEXT NOT NULL CHECK (sumber IN
        ('MASUK_BELANJA', 'MASUK_PANEN', 'MASUK_TITIPO', 'MASUK_KOREKSI')),
    status TEXT NOT NULL DEFAULT 'AKTIF' CHECK (status IN ('AKTIF', 'HABIS', 'KEDALUWARSA', 'DIBUANG')),
    dibuat_pada TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (tanggal_kedaluwarsa IS NULL OR tanggal_kedaluwarsa >= tanggal_masuk),
    CHECK (jumlah_aktif_x10 <= jumlah_awal_x10),
    CHECK (status NOT IN ('HABIS', 'DIBUANG') OR jumlah_aktif_x10 = 0)
);

-- Urutan FEFO: kedaluwarsa terdekat, null (FIFO) terakhir, lalu tanggal masuk tertua.
CREATE INDEX stok_batch_fefo_idx
    ON stok_batch (pantry_item_id, tanggal_kedaluwarsa NULLS LAST, tanggal_masuk, dibuat_pada)
    WHERE jumlah_aktif_x10 > 0;
CREATE INDEX stok_batch_household_idx ON stok_batch (household_id);

CREATE TABLE mutasi_stok (
    id TEXT PRIMARY KEY CHECK (id LIKE 'mut\_%'),
    household_id TEXT NOT NULL CHECK (household_id LIKE 'hh\_%'),
    batch_id TEXT NOT NULL REFERENCES stok_batch (id),
    jenis TEXT NOT NULL CHECK (jenis IN ('PAKAI', 'MASUK', 'KOREKSI', 'PINDAH', 'BUANG')),
    jumlah_x10 BIGINT NOT NULL,
    sisa_setelah_x10 BIGINT NOT NULL CHECK (sisa_setelah_x10 >= 0),
    alasan TEXT NOT NULL CHECK (char_length(alasan) BETWEEN 3 AND 140),
    aktor TEXT NOT NULL,
    sumber_resep_id TEXT,
    terjadi_pada TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Tanda jumlah mengikuti jenis: masuk positif, keluar negatif, pindah nol.
    CHECK (
        (jenis = 'MASUK' AND jumlah_x10 > 0)
        OR (jenis IN ('PAKAI', 'BUANG') AND jumlah_x10 < 0)
        OR (jenis = 'KOREKSI' AND jumlah_x10 <> 0)
        OR (jenis = 'PINDAH' AND jumlah_x10 = 0)
    )
);
CREATE INDEX mutasi_stok_batch_idx ON mutasi_stok (batch_id, terjadi_pada);
CREATE INDEX mutasi_stok_household_idx ON mutasi_stok (household_id, terjadi_pada);

-- Batas Varian 1 per rumah tangga: maks 400 item aktif dan 2.000 batch berisi.
-- Dijaga di basis data (trigger + kunci advisory) agar tahan balapan permintaan.
-- SQLSTATE khusus: PDI01 (item), PDB01 (batch); dipetakan ke 422 oleh AppError.
CREATE FUNCTION batasi_item_aktif() RETURNS trigger AS $$
BEGIN
    IF NOT NEW.arsip THEN
        PERFORM pg_advisory_xact_lock(hashtext('pdr_item:' || NEW.household_id));
        IF (SELECT count(*) FROM pantry_items
             WHERE household_id = NEW.household_id AND NOT arsip) >= 400 THEN
            RAISE EXCEPTION 'BATAS_ITEM_AKTIF: maksimum 400 item aktif per rumah tangga'
                USING ERRCODE = 'PDI01';
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER pantry_items_batas_ins BEFORE INSERT ON pantry_items
    FOR EACH ROW EXECUTE FUNCTION batasi_item_aktif();
CREATE TRIGGER pantry_items_batas_upd BEFORE UPDATE OF arsip ON pantry_items
    FOR EACH ROW WHEN (OLD.arsip AND NOT NEW.arsip) EXECUTE FUNCTION batasi_item_aktif();

CREATE FUNCTION batasi_batch() RETURNS trigger AS $$
BEGIN
    IF NEW.jumlah_aktif_x10 > 0 THEN
        PERFORM pg_advisory_xact_lock(hashtext('pdr_batch:' || NEW.household_id));
        IF (SELECT count(*) FROM stok_batch
             WHERE household_id = NEW.household_id AND jumlah_aktif_x10 > 0) >= 2000 THEN
            RAISE EXCEPTION 'BATAS_BATCH: maksimum 2.000 batch berisi per rumah tangga'
                USING ERRCODE = 'PDB01';
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER stok_batch_batas_ins BEFORE INSERT ON stok_batch
    FOR EACH ROW EXECUTE FUNCTION batasi_batch();
