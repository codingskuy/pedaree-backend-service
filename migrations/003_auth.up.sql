-- PDR-BE-003: pengguna dan refresh token (reuse-detection).
-- Pola mengikuti docs/auth.md di lumbung-backend-service: refresh disimpan sebagai hash.

CREATE TABLE pengguna (
    id TEXT PRIMARY KEY CHECK (id LIKE 'usr\_%'),
    household_id TEXT NOT NULL CHECK (household_id LIKE 'hh\_%'),
    id_pengguna TEXT NOT NULL CHECK (char_length(btrim(id_pengguna)) BETWEEN 3 AND 80),
    kata_sandi_hash TEXT NOT NULL,
    aktif BOOLEAN NOT NULL DEFAULT TRUE,
    dibuat_pada TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX pengguna_id_pengguna_uq ON pengguna (lower(btrim(id_pengguna)));
CREATE INDEX pengguna_household_idx ON pengguna (household_id);

CREATE TABLE refresh_token (
    id TEXT PRIMARY KEY CHECK (id LIKE 'rtk\_%'),
    pengguna_id TEXT NOT NULL REFERENCES pengguna (id),
    -- Satu keluarga = satu rantai rotasi sejak login; dipakai ulang = seluruh pengguna dicabut.
    keluarga_id TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    id_perangkat TEXT,
    dibuat_pada TIMESTAMPTZ NOT NULL DEFAULT now(),
    kedaluwarsa_pada TIMESTAMPTZ NOT NULL,
    dipakai_pada TIMESTAMPTZ,
    dicabut_pada TIMESTAMPTZ
);
CREATE INDEX refresh_token_pengguna_idx ON refresh_token (pengguna_id);
CREATE INDEX refresh_token_keluarga_idx ON refresh_token (keluarga_id);
