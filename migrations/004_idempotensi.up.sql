-- PDR-BE-004: hasil POST disimpan 24 jam per (rumah tangga, X-Request-Id) agar kirim ulang
-- tidak menggandakan mutasi (kontrak OpenAPI, bagian Idempotensi).
CREATE TABLE idempotensi (
    household_id TEXT NOT NULL,
    request_id UUID NOT NULL,
    endpoint TEXT NOT NULL,
    status_kode INTEGER NOT NULL,
    respons JSONB NOT NULL,
    dibuat_pada TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (household_id, request_id)
);
CREATE INDEX idempotensi_dibuat_idx ON idempotensi (dibuat_pada);
