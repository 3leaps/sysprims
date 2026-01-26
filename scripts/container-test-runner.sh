#!/bin/bash
# scripts/container-test-runner.sh
#
# Runs the privileged test suite inside the container.
# Designed to be safe - only affects processes inside the container.
#
# Usage:
#   docker run --rm -v $(pwd):/workspace:ro \
#     -v $(pwd)/target:/workspace/target \
#     sysprims-test-fixture
#
# Exit codes:
#   0 - All tests passed
#   1 - Test failures or errors

set -euo pipefail

# Auto-detect architecture for musl target
ARCH=$(uname -m)
case "$ARCH" in
x86_64) TARGET="x86_64-unknown-linux-musl" ;;
aarch64) TARGET="aarch64-unknown-linux-musl" ;;
arm64) TARGET="aarch64-unknown-linux-musl" ;;
*)
	echo "Unsupported architecture: $ARCH"
	exit 1
	;;
esac

echo "=== sysprims Container Test Fixture ==="
echo "User: $(whoami)"
echo "Rust: $(rustc --version)"
echo "Arch: $ARCH"
echo "Target: $TARGET"
echo ""

# Ensure musl target is installed
rustup target add "$TARGET" 2>/dev/null || true

# Create fixture processes for cross-user permission tests.
# We avoid PID 1 in tests (forbidden by safety protocols).
ROOT_PID_FILE="/tmp/sysprims_root_sleep.pid"
OTHER_USER_PID_FILE="/tmp/sysprims_testuser2_sleep.pid"

cleanup() {
	for f in "$ROOT_PID_FILE" "$OTHER_USER_PID_FILE"; do
		if [[ -f "$f" ]]; then
			pid="$(cat "$f" 2>/dev/null || true)"
			if [[ -n "${pid:-}" ]]; then
				kill -9 "$pid" 2>/dev/null || true
			fi
			rm -f "$f" 2>/dev/null || true
		fi
	done
}
trap cleanup EXIT

# Root-owned process (for EPERM checks as non-root)
nohup sleep 3600 >/dev/null 2>&1 &
echo $! >"$ROOT_PID_FILE"
chmod 644 "$ROOT_PID_FILE" 2>/dev/null || true

# testuser2-owned process (for cross-user permission checks as testuser)
su testuser2 -c "sh -c 'nohup sleep 3600 >/dev/null 2>&1 & echo \$! >\"$OTHER_USER_PID_FILE\"'"
chmod 644 "$OTHER_USER_PID_FILE" 2>/dev/null || true

# Step 1: Build sysprims with all test features
# Exclude sysprims-ts-napi: N-API cdylib cannot build on musl
echo "[1/4] Building sysprims..."
cargo build --workspace --exclude sysprims-ts-napi --target "$TARGET" \
	--features privileged-tests,cross-user-tests

# Step 2: Run standard tests first (should pass)
echo ""
echo "[2/4] Running standard tests..."
cargo test --workspace --exclude sysprims-ts-napi --target "$TARGET"

# Step 3: Run privileged tests (only available in container)
echo ""
echo "[3/4] Running privileged tests as root..."
cargo test --workspace --exclude sysprims-ts-napi --target "$TARGET" \
	--features privileged-tests \
	-- --test-threads=1 # Sequential to avoid race conditions

# Step 4: Run cross-user tests as non-root user
echo ""
echo "[4/4] Running cross-user tests as testuser..."

# testuser needs read access to workspace and write access to target
mkdir -p /workspace/target/container
chmod -R a+rwx /workspace/target/container 2>/dev/null || true
chown -R testuser:testuser /workspace/target/container 2>/dev/null || true

# Make cargo accessible to all users (root home is 700 by default)
chmod 755 /root
chmod -R a+rX /root/.cargo 2>/dev/null || true
chmod -R a+rX /root/.rustup 2>/dev/null || true

su testuser -c "
    export PATH=\"/root/.cargo/bin:\$PATH\"
    export CARGO_HOME=/root/.cargo
    export RUSTUP_HOME=/root/.rustup
    export CARGO_TARGET_DIR=/workspace/target/container
    cargo test --workspace --exclude sysprims-ts-napi --target $TARGET \
        --features cross-user-tests \
        -- --test-threads=1
"

echo ""
echo "=== All container tests passed ==="
