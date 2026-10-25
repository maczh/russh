SHELL := /bin/sh

NPM ?= npm
CARGO ?= cargo
TAURI ?= npx tauri
TAURI_MANIFEST := src-tauri/Cargo.toml
TAURI_TARGET ?=

.PHONY: help install dev debug frontend typecheck test check build release bundle \
	bundle-linux bundle-windows bundle-macos bundle-host \
	build-linux build-windows build-macos build-all clean

help:
	@printf '%s\n' \
		'RuSSH build targets:' \
		'  make install         Install JavaScript dependencies' \
		'  make dev             Start the Tauri development app' \
		'  make debug           Start the Tauri development app with Rust backtraces' \
		'  make frontend        Build the Tauri frontend assets' \
		'  make typecheck       Run TypeScript checks' \
		'  make test            Run the test suite' \
		'  make check           Run typecheck and Rust checks' \
		'  make build           Build the release Rust binary' \
		'  make bundle          Build all bundle formats supported by this host' \
		'  make bundle-linux    Build AppImage, DEB and RPM packages' \
		'  make bundle-windows  Build NSIS and MSI installers' \
		'  make bundle-macos    Build the macOS APP and DMG bundles' \
		'  make bundle-host     Select the bundle target from the host OS' \
		'  make build-all       Run Linux, Windows and macOS targets sequentially' \
		'  make clean           Remove generated frontend and Cargo artifacts' \
		'' \
		'Cross-platform note: native packaging requires the matching OS/toolchain.' \
		'Use a Windows, macOS or Linux runner for the corresponding bundle target.'

install:
	$(NPM) install

dev:
	$(NPM) run dev

debug:
	RUST_BACKTRACE=1 RUST_LOG=$${RUST_LOG:-debug} $(NPM) run dev

frontend:
	$(NPM) run tauri:build:frontend

typecheck:
	$(NPM) run typecheck

test:
	$(NPM) test -- --run

check: typecheck
	$(CARGO) fmt --manifest-path $(TAURI_MANIFEST) -- --check
	$(CARGO) check --manifest-path $(TAURI_MANIFEST) --features custom-protocol

build: frontend
	$(CARGO) build --release --manifest-path $(TAURI_MANIFEST) --features custom-protocol $(if $(TAURI_TARGET),--target $(TAURI_TARGET),)

release: build

bundle: bundle-host

bundle-linux:
	$(TAURI) build $(if $(TAURI_TARGET),--target $(TAURI_TARGET),) --bundles appimage,deb,rpm

bundle-windows:
	$(TAURI) build $(if $(TAURI_TARGET),--target $(TAURI_TARGET),) --bundles nsis,msi

bundle-macos:
	$(TAURI) build $(if $(TAURI_TARGET),--target $(TAURI_TARGET),) --bundles app,dmg

bundle-host:
	@case "$$(uname -s)" in \
		Linux*) $(MAKE) bundle-linux ;; \
		Darwin*) $(MAKE) bundle-macos ;; \
		MINGW*|MSYS*|CYGWIN*) $(MAKE) bundle-windows ;; \
		*) echo 'Unsupported host OS; use bundle-linux, bundle-windows, or bundle-macos explicitly.' >&2; exit 1 ;; \
	esac

build-linux:
	$(MAKE) bundle-linux

build-windows:
	$(MAKE) bundle-windows

build-macos:
	$(MAKE) bundle-macos

build-all:
	@echo 'Building all platform bundles requires native CI runners or cross-compilers.'
	@$(MAKE) bundle-linux
	@$(MAKE) bundle-windows
	@$(MAKE) bundle-macos

clean:
	$(CARGO) clean --manifest-path $(TAURI_MANIFEST)
	rm -rf dist/renderer
