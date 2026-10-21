#!/usr/bin/env bash
# Uji properti Schemathesis terhadap backend lokal memakai docs/api/openapi.yaml (PDR-BE-TEST-SCHEMATHESIS).
# Tanpa telemetri, tanpa layanan eksternal. Semua rahasia dibuat sementara (kunci JWT, kata sandi akun uji).
#
# Prasyarat: Postgres lokal (psql/createdb bisa), openssl, cargo, schemathesis (`uv tool install schemathesis`).
# Variabel opsional: PGHOST (default localhost), DB_UJI (default pedaree_schemathesis),
#   PORT (8099), MAX_EXAMPLES (25), ST_EXTRA (argumen tambahan).
set -euo pipefail

cd "$(dirname "$0")/.."
export E2E_TELEMETRY_DISABLED=1
# Schemathesis hanya mengirim data ke Schemathesis.io bila `--report schemathesis`/token diberikan; skrip ini tidak memakainya.
PORT="${PORT:-8099}"
DB_UJI="${DB_UJI:-pedaree_schemathesis}"
MAX_EXAMPLES="${MAX_EXAMPLES:-25}"
PGHOST="${PGHOST:-localhost}"
TMP="$(mktemp -d)"
SERVER_PID=""
cleanup() {
  [[ -n "$SERVER_PID" ]] && kill "$SERVER_PID" 2>/dev/null || true
  rm -rf "$TMP"
  dropdb -h "$PGHOST" --if-exists "$DB_UJI" >/dev/null 2>&1 || true
}
trap cleanup EXIT

dropdb -h "$PGHOST" --if-exists "$DB_UJI" >/dev/null
createdb -h "$PGHOST" "$DB_UJI"
export DATABASE_URL="postgres://${PGUSER:-$USER}@${PGHOST}/${DB_UJI}"

# Kunci Ed25519 sementara (tidak pernah disimpan di repo).
openssl genpkey -algorithm ed25519 -out "$TMP/priv.pem" 2>/dev/null
openssl pkey -in "$TMP/priv.pem" -pubout -out "$TMP/pub.pem" 2>/dev/null
export JWT_ED25519_PRIVATE="$(cat "$TMP/priv.pem")"
export JWT_ED25519_PUBLIC="$(cat "$TMP/pub.pem")"
export JWT_ISSUER="pedaree-schemathesis"
export BIND_ADDR="127.0.0.1:${PORT}"

cargo build --quiet --bin pedaree-backend-service
BIN=target/debug/pedaree-backend-service

# Akun uji lewat CLI operator (ADR-0011); kata sandi acak, tidak dicetak.
SANDI="$(openssl rand -hex 16)"
PEDAREE_KATA_SANDI="$SANDI" "$BIN" buat-akun --id-pengguna penguji-schemathesis --household hh_01J9Z8QK2M4B >/dev/null

"$BIN" >"$TMP/server.log" 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 50); do
  curl -fs "http://127.0.0.1:${PORT}/kesehatan" >/dev/null 2>&1 && break
  sleep 0.2
done
curl -fs "http://127.0.0.1:${PORT}/kesehatan" >/dev/null || { echo "server gagal naik"; cat "$TMP/server.log"; exit 1; }

TOKEN="$(curl -fs -X POST "http://127.0.0.1:${PORT}/auth/login" -H 'content-type: application/json' \
  -d "{\"idPengguna\":\"penguji-schemathesis\",\"kataSandi\":\"${SANDI}\"}" | python3 -c 'import sys,json; print(json.load(sys.stdin)["tokenAkses"])')"

# Operasi yang belum diimplementasikan server dikecualikan (404 RUTE_TIDAK_ADA); lihat README bagian Schemathesis.
EXCLUDE=(${ST_EXCLUDE_PATHS:-/demand/signals /demand/titipo /web/import/csv /web/export/csv /web/export/mutasi /web/titipo/parcels '/web/titipo/parcels/{id}/draft-masuk'})
ARGS=()
for p in "${EXCLUDE[@]}"; do ARGS+=(--exclude-path "$p"); done

set +e
schemathesis run docs/api/openapi.yaml \
  --url "http://127.0.0.1:${PORT}" \
  -H "Authorization: Bearer ${TOKEN}" \
  --max-examples "$MAX_EXAMPLES" \
  --phases examples,coverage,fuzzing \
  --workers 1 \
  "${ARGS[@]}" ${ST_EXTRA:-}
STATUS=$?
set -e
echo "schemathesis keluar dengan kode ${STATUS}"
exit "$STATUS"
