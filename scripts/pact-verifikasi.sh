#!/usr/bin/env bash
# Verifikasi provider Pact terhadap backend lokal (PDR-BE-TEST-PACT). SKELETON: belum pernah
# dijalankan dengan pact konsumen nyata (belum ada pact web/app-kmp di contract/pact/pacts).
#
# Kode keluar: 0 = semua pact lulus; 1 = ada pact gagal; 3 = TIDAK DIVERIFIKASI (tidak ada berkas
# pact atau pact_verifier_cli belum terpasang). Kode 3 bukan lulus.
#
# Prasyarat: Postgres lokal, openssl, cargo, dan `cargo install --locked pact_verifier_cli`.
set -euo pipefail

cd "$(dirname "$0")/.."
export E2E_TELEMETRY_DISABLED=1
PACT_DIR="${PACT_DIR:-contract/pact/pacts}"
PORT="${PORT:-8098}"
DB_UJI="${DB_UJI:-pedaree_pact}"
PGHOST="${PGHOST:-localhost}"

shopt -s nullglob
BERKAS=("$PACT_DIR"/*.json)
if [[ ${#BERKAS[@]} -eq 0 ]]; then
  echo "TIDAK DIVERIFIKASI: tidak ada berkas pact di $PACT_DIR (menunggu PDR-WEB-TEST-PACT / PDR-KMP-TEST-PACT)." >&2
  exit 3
fi
if ! command -v pact_verifier_cli >/dev/null 2>&1; then
  echo "TIDAK DIVERIFIKASI: pact_verifier_cli belum terpasang (cargo install --locked pact_verifier_cli)." >&2
  exit 3
fi

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
openssl genpkey -algorithm ed25519 -out "$TMP/priv.pem" 2>/dev/null
openssl pkey -in "$TMP/priv.pem" -pubout -out "$TMP/pub.pem" 2>/dev/null
export JWT_ED25519_PRIVATE="$(cat "$TMP/priv.pem")" JWT_ED25519_PUBLIC="$(cat "$TMP/pub.pem")"
export JWT_ISSUER="pedaree-pact" BIND_ADDR="127.0.0.1:${PORT}"

cargo build --quiet --features pact-state --bin pedaree-backend-service
target/debug/pedaree-backend-service >"$TMP/server.log" 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 50); do
  curl -fs "http://127.0.0.1:${PORT}/kesehatan" >/dev/null 2>&1 && break
  sleep 0.2
done

# Akun uji lewat state handler, lalu token (pact tidak boleh memuat token tetap: kedaluwarsa 15 menit).
SANDI="$(openssl rand -hex 16)"
curl -fs -X POST "http://127.0.0.1:${PORT}/_pact/state" -H 'content-type: application/json' \
  -d "{\"state\":\"pengguna terdaftar\",\"params\":{\"idPengguna\":\"pact\",\"kataSandi\":\"${SANDI}\"}}" >/dev/null
TOKEN="$(curl -fs -X POST "http://127.0.0.1:${PORT}/auth/login" -H 'content-type: application/json' \
  -d "{\"idPengguna\":\"pact\",\"kataSandi\":\"${SANDI}\"}" | python3 -c 'import sys,json; print(json.load(sys.stdin)["tokenAkses"])')"

ARGS=()
for f in "${BERKAS[@]}"; do ARGS+=(--file "$f"); done
# Flag mengikuti pact_verifier_cli 1.x; periksa `pact_verifier_cli --help` bila versi berbeda.
pact_verifier_cli "${ARGS[@]}" \
  --hostname 127.0.0.1 --port "$PORT" \
  --state-change-url "http://127.0.0.1:${PORT}/_pact/state" \
  --header "Authorization=Bearer ${TOKEN}" \
  --header "X-Household-Id=hh_01J9Z8QK2M4B" \
  --provider-name pedaree-backend-service
