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

.PHONY: all help bootstrap bootstrap-force tools check test fmt lint build clean version
.PHONY: precommit prepush deps-check audit deny
.PHONY: build-release build-ffi cbindgen

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
GONEAT_VERSION := v0.3.21

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
	@echo "Required targets:"
	@echo "  help            Show this help message"
	@echo "  bootstrap       Install tools (sfetch -> goneat)"
	@echo "  check           Run all quality checks (fmt, lint, test, deny)"
	@echo "  test            Run test suite"
	@echo "  fmt             Format code (cargo fmt)"
	@echo "  lint            Run linting (cargo clippy)"
	@echo "  build           Build all crates (debug)"
	@echo "  build-release   Build all crates (release)"
	@echo "  build-ffi       Build FFI library with C header"
	@echo "  clean           Remove build artifacts"
	@echo "  version         Print current version"
	@echo ""
	@echo "Quality gates:"
	@echo "  precommit       Pre-commit checks (fast: fmt, clippy)"
	@echo "  prepush         Pre-push checks (thorough: fmt, clippy, test, deny)"
	@echo "  deny            Run cargo-deny license and advisory checks"
	@echo "  audit           Run cargo-audit security scan"
	@echo "  deps-check      Check dependencies for cooling violations"
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
	@# Step 3: Install Rust tools via cargo (cargo-deny, cargo-audit)
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

check: fmt-check lint test deny ## Run all quality checks
	@echo "[ok] All quality checks passed"

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
