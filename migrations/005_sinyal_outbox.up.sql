-- PDR-BE-005: outbox sinyal `pedaree.stok.v1` ke TitipO dan jejak expiry-sweep harian.
-- Sinyal ditulis pada transaksi yang sama dengan mutasi stok (pola outbox); pengirim
-- terpisah mengirim ke `POST /sinyal/pedaree-stok` dengan retry lalu DLQ.

CREATE TABLE sinyal_stok (
    id TEXT PRIMARY KEY CHECK (id LIKE 'sig\_%'),
    household_id TEXT NOT NULL CHECK (household_id LIKE 'hh\_%'),
    item_id TEXT NOT NULL REFERENCES pantry_items (id),
    jenis TEXT NOT NULL CHECK (jenis IN ('stok.menipis', 'stok.habis', 'kedaluwarsa.dekat')),
    -- Kalender WIB; pasangan (item_id, tanggal) adalah kunci idempoten konsumen TitipO.
    tanggal DATE NOT NULL,
    payload JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'ANTRE' CHECK (status IN ('ANTRE', 'TERKIRIM', 'DLQ')),
    percobaan INTEGER NOT NULL DEFAULT 0 CHECK (percobaan >= 0),
    coba_lagi_pada TIMESTAMPTZ NOT NULL DEFAULT now(),
    dikirim_pada TIMESTAMPTZ,
    -- Jawaban penerima: dicatat_sebagai_anjuran (201) atau duplikat_diabaikan (200).
    jawaban TEXT,
    galat_terakhir TEXT,
    dibuat_pada TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX sinyal_stok_antre_idx ON sinyal_stok (coba_lagi_pada) WHERE status = 'ANTRE';
CREATE INDEX sinyal_stok_item_idx ON sinyal_stok (item_id, dibuat_pada DESC);

-- Satu baris per hari WIB yang sudah diklaim expiry-sweep (aman untuk banyak instans).
CREATE TABLE expiry_sweep_log (
    tanggal DATE PRIMARY KEY,
    dimulai_pada TIMESTAMPTZ NOT NULL DEFAULT now(),
    selesai_pada TIMESTAMPTZ,
    sinyal_dibuat INTEGER
);
