# cpx — build, test, install.
#
# The Tauri CLI resolves its own before-commands from an unhelpful working
# directory, so the frontend build is sequenced here instead. Every target is
# safe to run from the repository root.

SHELL := /bin/bash
TAURI := ui/node_modules/.bin/tauri
PREFIX ?= $(HOME)/.local
APPS ?= /Applications

.PHONY: help
help:
	@echo "make test       run the Rust suite and typecheck the UI"
	@echo "make cli        build the cpx command"
	@echo "make app        build cpx.app (unsigned; ad-hoc signed for local use)"
	@echo "make install    install the command to $(PREFIX)/bin and the app to $(APPS)"
	@echo "make dev        run the app against a live UI, for development"
	@echo "make release VERSION=x.y.z   build, publish, update the tap"
	@echo "make clean      remove build output"

node_modules: ui/package.json
	pnpm --dir ui install

.PHONY: test
test:
	cargo test
	cargo clippy --all-targets -- -D warnings
	cd ui && ./node_modules/.bin/tsc --noEmit

.PHONY: ui
ui:
	cd ui && ./node_modules/.bin/vite build

.PHONY: cli
cli:
	cargo build --release -p cpx-cli

.PHONY: app
app: ui
	cd crates/cpx-app && ../../$(TAURI) build
	@# Ad-hoc signing is enough for the machine that built it. A build meant
	@# for anyone else needs a Developer ID and notarisation.
	codesign --force --deep --sign - "target/release/bundle/macos/cpx.app"
	@echo "built target/release/bundle/macos/cpx.app"

.PHONY: install
install: cli app
	install -d "$(PREFIX)/bin"
	install -m 755 target/release/cpx "$(PREFIX)/bin/cpx"
	rm -rf "$(APPS)/cpx.app"
	cp -R target/release/bundle/macos/cpx.app "$(APPS)/cpx.app"
	@echo
	@echo "Installed:"
	@echo "  $(PREFIX)/bin/cpx"
	@echo "  $(APPS)/cpx.app"
	@echo
	@echo "Open the app from Spotlight; it lives in the menu bar, not the Dock."
	@echo "To start it at login, add it under System Settings > General > Login Items."

.PHONY: dev
dev: node_modules
	@echo "Starting the UI on :1420, then the app…"
	@cd ui && ./node_modules/.bin/vite --port 1420 --strictPort & \
	  trap 'kill %1 2>/dev/null' EXIT; \
	  sleep 2; \
	  cd crates/cpx-app && ../../$(TAURI) dev

.PHONY: release
release:
	@test -n "$(VERSION)" || { echo "usage: make release VERSION=0.2.0"; exit 1; }
	scripts/release.sh "$(VERSION)"

.PHONY: release-dry
release-dry:
	@test -n "$(VERSION)" || { echo "usage: make release-dry VERSION=0.2.0"; exit 1; }
	scripts/release.sh "$(VERSION)" --dry-run

.PHONY: clean
clean:
	cargo clean
	rm -rf ui/dist
