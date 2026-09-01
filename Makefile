SHELL := /bin/sh

.PHONY: dev mihomo

dev:
	@./scripts/dev.sh

mihomo:
	@./scripts/ensure-mihomo.sh
