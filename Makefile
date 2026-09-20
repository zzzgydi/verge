SHELL := /bin/sh

.PHONY: dev dev-build dev-stop mihomo release release-run release-size

dev:
	@./scripts/dev.sh

dev-build:
	@./scripts/dev.sh --build-only

dev-stop:
	@./scripts/dev.sh --stop

mihomo:
	@./scripts/ensure-mihomo.sh

# Local optimized .app and ZIP, with bundled Mihomo and ad-hoc signing by default.
release:
	@./apps/verge/scripts/build-macos-app.sh
	@$(MAKE) --no-print-directory release-size

# Quit any existing Verge daemon first; VERGE_DATA_DIR can isolate test data.
release-run: release
	@./dist/Verge.app/Contents/MacOS/verge-gpui

release-size:
	@./scripts/release-size.sh
