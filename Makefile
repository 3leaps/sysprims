# sysprims Makefile
# GPL-free, cross-platform process utilities library
#
# Compliant with docs/standards/repository-conventions.md
#
# Quick Reference:
#   make help       - Show all available targets
#   make bootstrap  - Install tools (sfetch -> goneat)
#   make check      - Run all quality checks (fmt, lint, test, deny)
#   make fmt        - Format code (cargo fmt)
#   make build      - Build all crates and FFI

.PHONY: all help bootstrap bootstrap-force tools check test fmt lint build clean version install
.PHONY: precommit prepush deps-check audit deny miri msrv
.PHONY: check-windows check-windows-msvc check-windows-gnu
.PHONY: build-release build-ffi cbindgen
.PHONY: build-local-go build-local-ffi-shared go-test header-go go-header go-prebuilt-darwin
.PHONY: release-clean release-download release-checksums release-sign
.PHONY: release-export-keys release-verify-checksums release-verify-signatures
.PHONY: release-verify-keys release-notes release-upload
.PHONY: version-patch version-minor version-major version-set

# -----------------------------------------------------------------------------
# Configuration
# -----------------------------------------------------------------------------

# Version from Cargo.toml (SSOT) - extracted via cargo metadata
VERSION := $(shell cargo metadata --format-version 1 2>/dev/null | \
	grep -o '"version":"[^"]*"' | head -1 | cut -d'"' -f4 || echo "dev")

# Tool installation directory
# Bootstrap installs sfetch to repo-local bin/
BIN_DIR := $(CURDIR)/bin

# Pinned tool versions for reproducibility
SFETCH_VERSION := latest
GONEAT_VERSION ?= v0.5.1

# Tool paths
# sfetch: repo-local (trust anchor) or PATH
# goneat: user-space PATH only (like prettier, biome, ruff)
SFETCH = $(shell [ -x "$(BIN_DIR)/sfetch" ] && echo "$(BIN_DIR)/sfetch" || command -v sfetch 2>/dev/null)
GONEAT = $(shell command -v goneat 2>/dev/null)

# Rust toolchain (assumed installed - rustup is developer responsibility)
CARGO = cargo

# -----------------------------------------------------------------------------
# Default and Help
# -----------------------------------------------------------------------------

all: check

help: ## Show available targets
	@echo "sysprims - GPL-free Process Utilities"
	@echo "Group-by-default process tree management."
	@echo ""
	@echo "Development:"
	@echo "  help            Show this help message"
	@echo "  bootstrap       Install tools (sfetch -> goneat)"
	@echo "  build           Build all crates (debug)"
	@echo "  build-release   Build all crates (release)"
	@echo "  build-ffi       Build FFI library with C header"
	@echo "  install         Install sysprims binary to ~/.local/bin"
	@echo "  clean           Remove build artifacts"
	@echo ""
	@echo "Go bindings:"
	@echo "  build-local-go      Build FFI for local Go development"
	@echo "  build-local-ffi-shared  Build shared FFI for local consumers"
	@echo "  go-test             Run Go binding tests"
	@echo "  header-go           Generate C header for Go bindings"
	@echo "  go-prebuilt-darwin  Build prebuilt libs for macOS"
	@echo ""
	@echo "Quality gates:"
	@echo "  check           Run all quality checks (fmt, lint, test, deny)"
	@echo "  check-windows   Fast Windows compile check (no SDK)"
	@echo "  check-windows-msvc  cargo check for x86_64-pc-windows-msvc"
	@echo "  check-windows-gnu   cargo check for x86_64-pc-windows-gnu"
	@echo "  test            Run test suite"
	@echo "  fmt             Format code (cargo fmt)"
	@echo "  lint            Run linting (cargo clippy)"
	@echo "  precommit       Pre-commit checks (fast: fmt, clippy)"
	@echo "  prepush         Pre-push checks (thorough: fmt, clippy, test, deny)"
	@echo "  deny            Run cargo-deny license and advisory checks"
	@echo "  audit           Run cargo-audit security scan"
	@echo "  miri            Run Miri UB detection on unsafe code (nightly)"
	@echo "  msrv            Verify build with MSRV (Rust 1.81)"
	@echo ""
	@echo "Release (manual signing workflow):"
	@echo "  release-download      Download CI artifacts from GitHub"
	@echo "  release-checksums     Generate SHA256SUMS and SHA512SUMS"
	@echo "  release-sign          Sign checksums (requires SYSPRIMS_MINISIGN_KEY)"
	@echo "  release-export-keys   Export public signing keys"
	@echo "  release-verify        Verify checksums, signatures, and keys"
	@echo "  release-upload        Upload signed artifacts to GitHub"
	@echo "  release               Full release workflow (all of the above)"
	@echo ""
	@echo "Version management:"
	@echo "  version         Print current version"
	@echo "  version-patch   Bump patch version (0.1.0 -> 0.1.1)"
	@echo "  version-minor   Bump minor version (0.1.0 -> 0.2.0)"
	@echo "  version-major   Bump major version (0.1.0 -> 1.0.0)"
	@echo "  version-set     Set explicit version (V=X.Y.Z)"
	@echo "  version-sync    Sync VERSION to Cargo.toml"
	@echo ""
	@echo "Current version: $(VERSION)"

# -----------------------------------------------------------------------------
# Bootstrap - Trust Anchor Chain
# -----------------------------------------------------------------------------
#
# Trust chain: curl -> sfetch -> goneat -> other tools
#
# sfetch (3leaps/sfetch) is the trust anchor - a minimal, auditable binary fetcher.
# goneat (fulmenhq/goneat) is installed via sfetch and manages additional tooling.
#
# NOTE: Rust toolchain (rustup/cargo) is a developer prerequisite, not bootstrapped.

bootstrap: ## Install required tools (sfetch -> goneat)
	@echo "Bootstrapping sysprims development environment..."
	@echo ""
	@# Step 0: Verify prerequisites
	@if ! command -v curl >/dev/null 2>&1; then \
		echo "[!!] curl not found (required for bootstrap)"; \
		exit 1; \
	fi
	@echo "[ok] curl found"
	@if ! command -v cargo >/dev/null 2>&1; then \
		echo "[!!] cargo not found (required)"; \
		echo ""; \
		echo "Install Rust toolchain:"; \
		echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"; \
		exit 1; \
	fi
	@echo "[ok] cargo: $$(cargo --version)"
	@echo ""
	@# Step 1: Install sfetch (trust anchor)
	@mkdir -p "$(BIN_DIR)"
	@if [ ! -x "$(BIN_DIR)/sfetch" ] && ! command -v sfetch >/dev/null 2>&1; then \
		echo "[..] Installing sfetch (trust anchor)..."; \
		curl -fsSL https://github.com/3leaps/sfetch/releases/download/$(SFETCH_VERSION)/install-sfetch.sh | bash -s -- --dest "$(BIN_DIR)"; \
	else \
		echo "[ok] sfetch already installed"; \
	fi
	@# Verify sfetch
	@SFETCH_BIN=""; \
	if [ -x "$(BIN_DIR)/sfetch" ]; then SFETCH_BIN="$(BIN_DIR)/sfetch"; \
	elif command -v sfetch >/dev/null 2>&1; then SFETCH_BIN="$$(command -v sfetch)"; fi; \
	if [ -z "$$SFETCH_BIN" ]; then echo "[!!] sfetch installation failed"; exit 1; fi; \
	echo "[ok] sfetch: $$SFETCH_BIN"
	@echo ""
	@# Step 2: Install goneat via sfetch
	@SFETCH_BIN=""; \
	if [ -x "$(BIN_DIR)/sfetch" ]; then SFETCH_BIN="$(BIN_DIR)/sfetch"; \
	elif command -v sfetch >/dev/null 2>&1; then SFETCH_BIN="$$(command -v sfetch)"; fi; \
	if [ "$(FORCE)" = "1" ] || ! command -v goneat >/dev/null 2>&1; then \
		echo "[..] Installing goneat $(GONEAT_VERSION) via sfetch (user-space)..."; \
		$$SFETCH_BIN --repo fulmenhq/goneat --tag $(GONEAT_VERSION); \
	else \
		echo "[ok] goneat already installed"; \
	fi
	@# Verify goneat (user-space only, not repo-local)
	@if command -v goneat >/dev/null 2>&1; then \
		echo "[ok] goneat: $$(goneat version 2>&1 | head -n1)"; \
	else \
		echo "[!!] goneat installation failed"; exit 1; \
	fi
	@echo ""
	@# Step 3: Install Rust tools via cargo (cargo-deny, cargo-audit, cargo-edit)
	@echo "[..] Checking Rust dev tools..."
	@if ! command -v cargo-deny >/dev/null 2>&1; then \
		echo "[..] Installing cargo-deny..."; \
		cargo install cargo-deny --locked; \
	else \
		echo "[ok] cargo-deny installed"; \
	fi
	@if ! command -v cargo-audit >/dev/null 2>&1; then \
		echo "[..] Installing cargo-audit..."; \
		cargo install cargo-audit --locked; \
	else \
		echo "[ok] cargo-audit installed"; \
	fi
	@if ! cargo set-version -V >/dev/null 2>&1; then \
		echo "[..] Installing cargo-edit..."; \
		cargo install cargo-edit --locked; \
	else \
		echo "[ok] cargo-edit installed"; \
	fi
	@echo ""
	@echo "[ok] Bootstrap complete"
	@echo ""
	@echo "Ensure $(BIN_DIR) is in your PATH, or tools will be found automatically."

bootstrap-force: ## Force reinstall all tools
	@$(MAKE) bootstrap FORCE=1

tools: ## Verify external tools are available
	@echo "Verifying tools..."
	@# Check cargo (required)
	@if command -v cargo >/dev/null 2>&1; then \
		echo "[ok] cargo: $$(cargo --version)"; \
	else \
		echo "[!!] cargo not found (required - install rustup)"; \
	fi
	@# Check rustfmt
	@if cargo fmt --version >/dev/null 2>&1; then \
		echo "[ok] rustfmt: $$(cargo fmt --version)"; \
	else \
		echo "[!!] rustfmt not found (rustup component add rustfmt)"; \
	fi
	@# Check clippy
	@if cargo clippy --version >/dev/null 2>&1; then \
		echo "[ok] clippy: $$(cargo clippy --version)"; \
	else \
		echo "[!!] clippy not found (rustup component add clippy)"; \
	fi
	@# Check cargo-deny
	@if command -v cargo-deny >/dev/null 2>&1; then \
		echo "[ok] cargo-deny: $$(cargo-deny --version)"; \
	else \
		echo "[!!] cargo-deny not found (cargo install cargo-deny)"; \
	fi
	@# Check cargo-audit
	@if command -v cargo-audit >/dev/null 2>&1; then \
		echo "[ok] cargo-audit: $$(cargo-audit --version)"; \
	else \
		echo "[!!] cargo-audit not found (cargo install cargo-audit)"; \
	fi
	@# Check cargo-edit
	@if cargo set-version -V >/dev/null 2>&1; then \
		echo "[ok] cargo-edit: $$(cargo set-version -V)"; \
	else \
		echo "[!!] cargo-edit not found (cargo install cargo-edit)"; \
	fi
	@# Check sfetch
	@if [ -x "$(BIN_DIR)/sfetch" ]; then \
		echo "[ok] sfetch: $(BIN_DIR)/sfetch"; \
	elif command -v sfetch >/dev/null 2>&1; then \
		echo "[ok] sfetch: $$(command -v sfetch)"; \
	else \
		echo "[!!] sfetch not found (run 'make bootstrap')"; \
	fi
	@# Check goneat (user-space)
	@if command -v goneat >/dev/null 2>&1; then \
		echo "[ok] goneat: $$(goneat version 2>&1 | head -n1)"; \
	else \
		echo "[!!] goneat not found (run 'make bootstrap')"; \
	fi
	@echo ""

# -----------------------------------------------------------------------------
# Quality Gates
# -----------------------------------------------------------------------------

check: fmt-check lint test check-windows deny ## Run all quality checks
	@echo "[ok] All quality checks passed"

check-windows: check-windows-msvc check-windows-gnu ## Fast Windows compile checks (no SDK)
	@echo "[ok] Windows cross-target checks passed"

check-windows-msvc: ## cargo check for x86_64-pc-windows-msvc (no SDK)
	@echo "Checking Windows target compilation (msvc) ..."
	@if ! command -v rustup >/dev/null 2>&1; then \
		echo "[!!] rustup not found (required to add Windows targets)"; \
		exit 1; \
	fi
	@rustup target add x86_64-pc-windows-msvc >/dev/null
	@$(CARGO) check --workspace --exclude sysprims-ts-napi --target x86_64-pc-windows-msvc
	@echo "[ok] Windows MSVC target check passed"

check-windows-gnu: ## cargo check for x86_64-pc-windows-gnu (no SDK)
	@echo "Checking Windows target compilation (gnu) ..."
	@if ! command -v rustup >/dev/null 2>&1; then \
		echo "[!!] rustup not found (required to add Windows targets)"; \
		exit 1; \
	fi
	@rustup target add x86_64-pc-windows-gnu >/dev/null
	@$(CARGO) check --workspace --exclude sysprims-ts-napi --target x86_64-pc-windows-gnu
	@echo "[ok] Windows GNU target check passed"

test: ## Run test suite
	@echo "Running tests..."
	$(CARGO) test --workspace
	@echo "[ok] Tests passed"

fmt: ## Format code (cargo fmt)
	@echo "Formatting..."
	$(CARGO) fmt --all
	@echo "[ok] Formatting complete"

fmt-check: ## Check formatting without modifying
	@echo "Checking formatting..."
	$(CARGO) fmt --all -- --check
	@echo "[ok] Formatting check passed"

lint: ## Run linting (cargo clippy)
	@echo "Linting..."
	$(CARGO) clippy --workspace --all-targets -- -D warnings
	@echo "[ok] Linting passed"

typecheck: ## Run TypeScript type checking
	@echo "Type checking TypeScript bindings..."
	@cd bindings/typescript/sysprims && npx tsc --noEmit -p tsconfig.test.json
	@echo "[ok] TypeScript type check passed"

deny: ## Run cargo-deny license and advisory checks
	@echo "Running cargo-deny..."
	@if command -v cargo-deny >/dev/null 2>&1; then \
		cargo-deny check; \
	else \
		echo "[!!] cargo-deny not found (run 'make bootstrap')"; \
		exit 1; \
	fi
	@echo "[ok] cargo-deny passed"

audit: ## Run cargo-audit security scan
	@echo "Running cargo-audit..."
	@if command -v cargo-audit >/dev/null 2>&1; then \
		cargo-audit audit; \
	else \
		echo "[!!] cargo-audit not found (run 'make bootstrap')"; \
		exit 1; \
	fi
	@echo "[ok] cargo-audit passed"

miri: ## Run Miri to detect undefined behavior in unsafe code (requires nightly)
	@echo "Running Miri..."
	@if rustup run nightly cargo miri --version >/dev/null 2>&1; then \
		rustup run nightly cargo miri test -p sysprims-core --lib && \
		rustup run nightly cargo miri test -p sysprims-ffi --lib; \
	else \
		echo "[!!] Miri not installed. Install with:"; \
		echo "  rustup +nightly component add miri"; \
		exit 1; \
	fi
	@echo "[ok] Miri passed"

msrv: ## Verify build with Minimum Supported Rust Version (1.81)
	@echo "Checking MSRV (1.81)..."
	@if rustup run 1.81 cargo --version >/dev/null 2>&1; then \
		rustup run 1.81 cargo build --workspace && \
		rustup run 1.81 cargo test --workspace; \
	else \
		echo "[!!] Rust 1.81 not installed. Install with:"; \
		echo "  rustup install 1.81"; \
		exit 1; \
	fi
	@echo "[ok] MSRV check passed"

# -----------------------------------------------------------------------------
# Build
# -----------------------------------------------------------------------------

build: ## Build all crates (debug)
	@echo "Building (debug)..."
	$(CARGO) build --workspace
	@echo "[ok] Build complete"

build-release: ## Build all crates (release)
	@echo "Building (release)..."
	$(CARGO) build --workspace --release
	@echo "[ok] Release build complete"

build-ffi: cbindgen ## Build FFI library with C header
	@echo "Building FFI library..."
	$(CARGO) build --package sysprims-ffi --release
	@echo "[ok] FFI build complete"
	@echo "Library: target/release/libsysprims.*"
	@echo "Header: ffi/sysprims-ffi/sysprims.h"

cbindgen: ## Generate C header from FFI crate
	@echo "Generating C header..."
	@if command -v cbindgen >/dev/null 2>&1; then \
		cbindgen --config cbindgen.toml --crate sysprims-ffi --output ffi/sysprims-ffi/sysprims.h; \
		echo "[ok] Generated ffi/sysprims-ffi/sysprims.h"; \
	else \
		echo "[!!] cbindgen not found (cargo install cbindgen)"; \
		exit 1; \
	fi

clean: ## Remove build artifacts
	@echo "Cleaning..."
	$(CARGO) clean
	@rm -rf bin/
	@echo "[ok] Clean complete"

# -----------------------------------------------------------------------------
# Go Bindings
# -----------------------------------------------------------------------------
#
# Build and test Go bindings for sysprims.
# Prebuilt static libraries are committed at release tags.
# Local development uses lib/local/ (gitignored).

GO_BINDINGS_DIR := bindings/go/sysprims
GO_LIB_ROOT := $(GO_BINDINGS_DIR)/lib

# Detect current platform
UNAME_S := $(shell uname -s | tr '[:upper:]' '[:lower:]')
UNAME_M := $(shell uname -m)

# Normalize architecture names for Go
ifeq ($(UNAME_M),x86_64)
    GO_ARCH := amd64
endif
ifeq ($(UNAME_M),aarch64)
    GO_ARCH := arm64
endif
ifeq ($(UNAME_M),arm64)
    GO_ARCH := arm64
endif

# Normalize OS names
ifeq ($(UNAME_S),darwin)
    GO_OS := darwin
    GO_LIB_EXT := .a
    GO_SHARED_EXT := .dylib
    GO_LIB_PREFIX := lib
endif
ifeq ($(UNAME_S),linux)
    GO_OS := linux
    GO_LIB_EXT := .a
    GO_SHARED_EXT := .so
    GO_LIB_PREFIX := lib
endif

GO_LOCAL_LIB := $(GO_LIB_ROOT)/local/$(GO_OS)-$(GO_ARCH)

build-local-go: ## Build FFI for local Go development
	@echo "Building FFI for local Go development ($(GO_OS)-$(GO_ARCH))..."
	$(CARGO) build --release -p sysprims-ffi
	@mkdir -p $(GO_LOCAL_LIB)
	@cp target/release/$(GO_LIB_PREFIX)sysprims_ffi$(GO_LIB_EXT) $(GO_LOCAL_LIB)/
	@echo "[ok] FFI library copied to $(GO_LOCAL_LIB)/"

	@echo "Staging local release-like assets in $(DIST_LOCAL)/release/sysprims-ffi/..."
	@mkdir -p $(DIST_LOCAL)/release/sysprims-ffi/include
	@cp target/release/$(GO_LIB_PREFIX)sysprims_ffi$(GO_LIB_EXT) $(DIST_LOCAL)/release/sysprims-ffi/libsysprims_ffi.a
	@cp ffi/sysprims-ffi/sysprims.h $(DIST_LOCAL)/release/sysprims-ffi/include/sysprims.h
	@cp $(GO_BINDINGS_DIR)/include/sysprims.h $(DIST_LOCAL)/release/sysprims-ffi/include/sysprims-go.h
	@shared_root="target/release/$(GO_LIB_PREFIX)sysprims_ffi$(GO_SHARED_EXT)"; \
	if [ -f "$$shared_root" ]; then \
		cp "$$shared_root" "$(DIST_LOCAL)/release/sysprims-ffi/"; \
	fi

	@# Also stage a release-like layout (static + shared split) for local consumers.
	@mkdir -p $(DIST_LOCAL)/release/sysprims-ffi/lib/$(GO_OS)-$(GO_ARCH)/static
	@cp target/release/$(GO_LIB_PREFIX)sysprims_ffi$(GO_LIB_EXT) $(DIST_LOCAL)/release/sysprims-ffi/lib/$(GO_OS)-$(GO_ARCH)/static/
	@mkdir -p $(DIST_LOCAL)/release/sysprims-ffi/lib/$(GO_OS)-$(GO_ARCH)/shared
	@shared="target/release/$(GO_LIB_PREFIX)sysprims_ffi$(GO_SHARED_EXT)"; \
	if [ -f "$$shared" ]; then \
		cp "$$shared" "$(DIST_LOCAL)/release/sysprims-ffi/lib/$(GO_OS)-$(GO_ARCH)/shared/"; \
	else \
		echo "[--] Shared library not produced at $$shared (skipping)"; \
	fi
	@echo "Built locally from working tree." > $(DIST_LOCAL)/release/sysprims-ffi/LOCAL.txt
	@echo "[ok] Local assets staged at $(DIST_LOCAL)/release/sysprims-ffi/"

build-local-ffi-shared: ## Build shared FFI library and stage it locally
	@echo "Building shared FFI library for local consumers ($(GO_OS)-$(GO_ARCH))..."
	$(CARGO) build --release -p sysprims-ffi
	@mkdir -p $(DIST_LOCAL)/release/sysprims-ffi/lib/$(GO_OS)-$(GO_ARCH)/shared
	@shared="target/release/$(GO_LIB_PREFIX)sysprims_ffi$(GO_SHARED_EXT)"; \
	if [ -f "$$shared" ]; then \
		cp "$$shared" "$(DIST_LOCAL)/release/sysprims-ffi/"; \
		cp "$$shared" "$(DIST_LOCAL)/release/sysprims-ffi/lib/$(GO_OS)-$(GO_ARCH)/shared/"; \
		echo "[ok] Shared library copied to $(DIST_LOCAL)/release/sysprims-ffi/lib/$(GO_OS)-$(GO_ARCH)/shared/"; \
	else \
		echo "[!!] Shared library not produced at $$shared"; \
		exit 1; \
	fi

go-test: build-local-go ## Run Go binding tests
	@echo "Running Go tests..."
	cd $(GO_BINDINGS_DIR) && go test -v ./...
	@echo "[ok] Go tests passed"

header-go: ## Generate C header for Go bindings
	@echo "Generating C header for Go bindings..."
	@if command -v cbindgen >/dev/null 2>&1; then \
		cbindgen --config cbindgen.toml --crate sysprims-ffi --output $(GO_BINDINGS_DIR)/include/sysprims.h; \
		echo "[ok] Generated $(GO_BINDINGS_DIR)/include/sysprims.h"; \
	else \
		echo "[!!] cbindgen not found (cargo install cbindgen)"; \
		exit 1; \
	fi

go-header: header-go ## Back-compat alias
	@echo "[--] go-header is deprecated; use header-go"

go-prebuilt-darwin: ## Build prebuilt libs for macOS (maintainer use)
	@echo "Building prebuilt libs for macOS (both architectures)..."
	rustup target add aarch64-apple-darwin x86_64-apple-darwin
	@mkdir -p $(GO_LIB_ROOT)/darwin-arm64 $(GO_LIB_ROOT)/darwin-amd64
	MACOSX_DEPLOYMENT_TARGET=11.0 $(CARGO) build --release --target aarch64-apple-darwin -p sysprims-ffi
	@cp target/aarch64-apple-darwin/release/libsysprims_ffi.a $(GO_LIB_ROOT)/darwin-arm64/
	MACOSX_DEPLOYMENT_TARGET=11.0 $(CARGO) build --release --target x86_64-apple-darwin -p sysprims-ffi
	@cp target/x86_64-apple-darwin/release/libsysprims_ffi.a $(GO_LIB_ROOT)/darwin-amd64/
	@echo "[ok] Built prebuilt libs for darwin-arm64 and darwin-amd64"

# -----------------------------------------------------------------------------
# Install
# -----------------------------------------------------------------------------
#
# Install sysprims binary to user-space bin directory.
# Default: ~/.local/bin (macOS/Linux)
#
# Override with: make install INSTALL_BINDIR=/usr/local/bin

INSTALL_BINDIR ?= $(HOME)/.local/bin

install: build-release ## Install sysprims binary to INSTALL_BINDIR
	@echo "Installing sysprims to $(INSTALL_BINDIR)..."
	@mkdir -p "$(INSTALL_BINDIR)"
	@cp target/release/sysprims "$(INSTALL_BINDIR)/sysprims"
	@chmod 755 "$(INSTALL_BINDIR)/sysprims"
	@echo "[ok] Installed sysprims to $(INSTALL_BINDIR)/sysprims"
	@echo ""
	@echo "Ensure $(INSTALL_BINDIR) is in your PATH:"
	@echo '  export PATH="$$HOME/.local/bin:$$PATH"'

# -----------------------------------------------------------------------------
# Pre-commit / Pre-push Hooks
# -----------------------------------------------------------------------------
#
# precommit: Fast checks suitable for every commit
#   - Format check, clippy
#   - No tests (too slow for every commit)
#
# prepush: Thorough checks before pushing
#   - Format check, clippy, full test suite, cargo-deny
#
# Install hooks: goneat hooks init && goneat hooks generate && goneat hooks install

precommit: fmt-check lint ## Run pre-commit checks (fast)
	@echo "[ok] Pre-commit checks passed"

prepush: check ## Run pre-push checks (thorough)
	@echo "[ok] Pre-push checks passed"

deps-check: ## Check dependencies for cooling violations
	@echo "Checking dev dependencies..."
	@if command -v goneat >/dev/null 2>&1; then \
		goneat dependencies check --cooling-days 7 --dev-deps-only 2>/dev/null || \
		echo "[--] Dependency cooling check not available"; \
	else \
		echo "[--] goneat not found, skipping dependency check"; \
	fi

# -----------------------------------------------------------------------------
# Version Management
# -----------------------------------------------------------------------------
#
# Version is managed via Cargo.toml (SSOT).
# Use: cargo set-version <version> (requires cargo-edit)
# Or edit [workspace.package].version in Cargo.toml directly.

version: ## Print current version
	@echo "$(VERSION)"

# -----------------------------------------------------------------------------
# Release Workflow
# -----------------------------------------------------------------------------
#
# Manual signing workflow (CI builds unsigned, human signs locally):
#
# 1. CI creates draft release on tag push
# 2. Download artifacts: make release-download
# 3. Generate checksums: make release-checksums
# 4. Sign checksums: make release-sign (requires SYSPRIMS_MINISIGN_KEY)
# 5. Export public keys: make release-export-keys
# 6. Verify everything: make release-verify
# 7. Upload signed artifacts: make release-upload

DIST_RELEASE := dist/release
DIST_LOCAL := dist/local
SYSPRIMS_RELEASE_TAG ?= $(shell git describe --tags --abbrev=0 2>/dev/null || echo v$(VERSION))

# Signing keys (set these environment variables)
SYSPRIMS_MINISIGN_KEY ?=
SYSPRIMS_MINISIGN_PUB ?=
SYSPRIMS_PGP_KEY_ID ?=
SYSPRIMS_GPG_HOMEDIR ?=

release-clean: ## Remove dist/release contents
	@echo "Cleaning release directory..."
	rm -rf $(DIST_RELEASE)
	@echo "[ok] Release directory cleaned"

dist-local-clean: ## Remove dist/local contents
	@echo "Cleaning local dist directory..."
	rm -rf $(DIST_LOCAL)
	@echo "[ok] Local dist directory cleaned"

release-download: ## Download release assets from GitHub
	@if [ -z "$(SYSPRIMS_RELEASE_TAG)" ] || [ "$(SYSPRIMS_RELEASE_TAG)" = "v" ]; then \
		echo "Error: No release tag found. Set SYSPRIMS_RELEASE_TAG=vX.Y.Z"; \
		exit 1; \
	fi
	./scripts/download-release-assets.sh $(SYSPRIMS_RELEASE_TAG) $(DIST_RELEASE)

release-checksums: ## Generate SHA256SUMS and SHA512SUMS
	./scripts/generate-checksums.sh $(DIST_RELEASE)

release-sign: ## Sign checksum manifests (requires SYSPRIMS_MINISIGN_KEY)
	@if [ -z "$(SYSPRIMS_MINISIGN_KEY)" ]; then \
		echo "Error: SYSPRIMS_MINISIGN_KEY not set"; \
		echo ""; \
		echo "Set the path to your minisign secret key:"; \
		echo "  export SYSPRIMS_MINISIGN_KEY=/path/to/sysprims.key"; \
		exit 1; \
	fi
	SYSPRIMS_MINISIGN_KEY=$(SYSPRIMS_MINISIGN_KEY) \
	SYSPRIMS_PGP_KEY_ID=$(SYSPRIMS_PGP_KEY_ID) \
	SYSPRIMS_GPG_HOMEDIR=$(SYSPRIMS_GPG_HOMEDIR) \
	./scripts/sign-release-assets.sh $(SYSPRIMS_RELEASE_TAG) $(DIST_RELEASE)

release-export-keys: ## Export public signing keys
	SYSPRIMS_MINISIGN_KEY=$(SYSPRIMS_MINISIGN_KEY) \
	SYSPRIMS_MINISIGN_PUB=$(SYSPRIMS_MINISIGN_PUB) \
	SYSPRIMS_PGP_KEY_ID=$(SYSPRIMS_PGP_KEY_ID) \
	SYSPRIMS_GPG_HOMEDIR=$(SYSPRIMS_GPG_HOMEDIR) \
	./scripts/export-release-keys.sh $(DIST_RELEASE)

release-verify-checksums: ## Verify checksums match artifacts
	@echo "Verifying checksums..."
	cd $(DIST_RELEASE) && shasum -a 256 -c SHA256SUMS
	@echo "[ok] Checksums verified"

release-verify-signatures: ## Verify minisign/PGP signatures
	./scripts/verify-signatures.sh $(DIST_RELEASE)

release-verify-keys: ## Verify exported keys are public-only
	./scripts/verify-public-keys.sh $(DIST_RELEASE)

release-verify: release-verify-checksums release-verify-signatures release-verify-keys ## Run all release verification
	@echo "[ok] All release verifications passed"

release-notes: ## Copy release notes to dist
	@src="docs/releases/$(SYSPRIMS_RELEASE_TAG).md"; \
	if [ -f "$$src" ]; then \
		cp "$$src" "$(DIST_RELEASE)/release-notes-$(SYSPRIMS_RELEASE_TAG).md"; \
		echo "[ok] Copied release notes"; \
	else \
		echo "[--] No release notes found at $$src"; \
	fi

release-upload: release-verify release-notes ## Upload signed artifacts to GitHub release
	./scripts/upload-release-assets.sh $(SYSPRIMS_RELEASE_TAG) $(DIST_RELEASE)

release: release-clean release-download release-checksums release-sign release-export-keys release-upload ## Full release workflow (after CI build)
	@echo "[ok] Release $(SYSPRIMS_RELEASE_TAG) complete"

# -----------------------------------------------------------------------------
# Version Management
# -----------------------------------------------------------------------------
#
# Version SSOT is the VERSION file (not Cargo.toml).
# Cargo.toml workspace version should match VERSION file.
#
# Usage:
#   make version-patch    # 0.1.0 -> 0.1.1
#   make version-minor    # 0.1.0 -> 0.2.0
#   make version-major    # 0.1.0 -> 1.0.0
#   make version-set V=1.2.3

VERSION_FILE := VERSION

version-patch: ## Bump patch version (0.1.0 -> 0.1.1)
	@current=$$(cat $(VERSION_FILE)); \
	major=$$(echo $$current | cut -d. -f1); \
	minor=$$(echo $$current | cut -d. -f2); \
	patch=$$(echo $$current | cut -d. -f3); \
	new_patch=$$((patch + 1)); \
	new_version="$$major.$$minor.$$new_patch"; \
	echo "$$new_version" > $(VERSION_FILE); \
	echo "Version bumped: $$current -> $$new_version"

version-minor: ## Bump minor version (0.1.0 -> 0.2.0)
	@current=$$(cat $(VERSION_FILE)); \
	major=$$(echo $$current | cut -d. -f1); \
	minor=$$(echo $$current | cut -d. -f2); \
	new_minor=$$((minor + 1)); \
	new_version="$$major.$$new_minor.0"; \
	echo "$$new_version" > $(VERSION_FILE); \
	echo "Version bumped: $$current -> $$new_version"

version-major: ## Bump major version (0.1.0 -> 1.0.0)
	@current=$$(cat $(VERSION_FILE)); \
	major=$$(echo $$current | cut -d. -f1); \
	new_major=$$((major + 1)); \
	new_version="$$new_major.0.0"; \
	echo "$$new_version" > $(VERSION_FILE); \
	echo "Version bumped: $$current -> $$new_version"

version-set: ## Set explicit version (V=X.Y.Z)
	@if [ -z "$(V)" ]; then \
		echo "Usage: make version-set V=1.2.3"; \
		exit 1; \
	fi
	@echo "$(V)" > $(VERSION_FILE)
	@echo "Version set to $(V)"

version-sync: ## Sync VERSION file to Cargo.toml and TypeScript package.json
	@ver=$$(cat $(VERSION_FILE)); \
	if command -v cargo-set-version >/dev/null 2>&1; then \
		cargo set-version --workspace "$$ver"; \
		echo "[ok] Synced Cargo.toml to $$ver"; \
	else \
		echo "[!!] cargo-edit not installed (cargo install cargo-edit)"; \
		echo "Manual update required: set version = \"$$ver\" in Cargo.toml"; \
	fi
	@ver=$$(cat $(VERSION_FILE)); \
	ts_pkg="bindings/typescript/sysprims/package.json"; \
	if [ -f "$$ts_pkg" ]; then \
		sed -i.bak -e "s/\"version\": \"[0-9]*\.[0-9]*\.[0-9]*\"/\"version\": \"$$ver\"/" "$$ts_pkg"; \
		sed -i.bak -e "s/@3leaps\/sysprims-\([^\"]*\)\": \"[0-9]*\.[0-9]*\.[0-9]*\"/@3leaps\/sysprims-\1\": \"$$ver\"/g" "$$ts_pkg"; \
		rm -f "$$ts_pkg.bak"; \
		echo "[ok] Synced TypeScript package.json to $$ver"; \
	fi
