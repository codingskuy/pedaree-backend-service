.PHONY: schemathesis
schemathesis: ## uji properti terhadap backend lokal (butuh Postgres lokal + schemathesis)
	./scripts/schemathesis.sh

.PHONY: pact
pact: ## verifikasi provider Pact (keluar 3 = tidak diverifikasi: belum ada pact/CLI)
	./scripts/pact-verifikasi.sh
