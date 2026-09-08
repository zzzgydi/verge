SHELL := /bin/sh

.PHONY: dev mihomo release release-run release-size

dev:
	@./scripts/dev.sh

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
