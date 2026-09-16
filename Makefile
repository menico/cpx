# cpx — build, test, install.
#
# The command is Rust; the menu bar app is Swift and drives the command, so
# the app bundle carries a copy of the command inside it. Every target is
# safe to run from the repository root.

SHELL := /bin/bash
PREFIX ?= $(HOME)/.local
APPS ?= /Applications
VERSION := $(shell grep -m1 '^version' Cargo.toml | cut -d'"' -f2)

.PHONY: help
help:
	@echo "make test       run the Rust suite, clippy with warnings denied, and the Swift tests"
	@echo "make cli        build the cpx command"
	@echo "make app        build cpx.app (ad-hoc signed for local use)"
	@echo "make install    install the command to $(PREFIX)/bin and the app to $(APPS)"
	@echo "make dev        run the app from the build directory, with the popover open"
	@echo "make release VERSION=x.y.z   build, publish, update the tap"
	@echo "make clean      remove build output"

.PHONY: test
test:
	cargo test
	cargo clippy --all-targets -- -D warnings
	cd app && swift test

.PHONY: cli
cli:
	cargo build --release -p cpx-cli

.PHONY: app
app: cli
	app/build.sh "$(VERSION)" target/release/cpx
	@echo "built target/app/cpx.app"

.PHONY: install
install: app
	install -d "$(PREFIX)/bin"
	install -m 755 target/release/cpx "$(PREFIX)/bin/cpx"
	rm -rf "$(APPS)/cpx.app"
	cp -R target/app/cpx.app "$(APPS)/cpx.app"
	@echo
	@echo "Installed:"
	@echo "  $(PREFIX)/bin/cpx"
	@echo "  $(APPS)/cpx.app"
	@echo
	@echo "Open the app from Spotlight; it lives in the menu bar, not the Dock."
	@echo "Settings has a switch to start it at login."

.PHONY: dev
dev: app
	@echo "Running the app with the popover open (Ctrl-C stops it)…"
	CPX_DEV_SHOW=1 target/app/cpx.app/Contents/MacOS/cpx-app

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
	rm -rf app/.build target/app
