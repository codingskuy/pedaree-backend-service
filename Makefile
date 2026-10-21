.PHONY: schemathesis
schemathesis: ## uji properti terhadap backend lokal (butuh Postgres lokal + schemathesis)
	./scripts/schemathesis.sh
