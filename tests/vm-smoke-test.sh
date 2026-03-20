#!/usr/bin/env bash
#
# Destructive smoke test for brewdock using a Tart macOS VM.
#
# Usage:
#   ./tests/vm-smoke-test.sh [--keep]
#
# Prerequisites:
#   - Tart (https://tart.run) installed
#   - Base image pulled:  tart pull ghcr.io/cirruslabs/macos-sequoia-base:latest
#   - Release binary built: cargo build --release -p brewdock-cli
#
# Authentication:
#   Uses a temporary SSH keypair injected via Tart directory sharing.
#   No sshpass or expect needed — only standard macOS tools.
#
# The script:
#   1. Generates a temporary SSH keypair
#   2. Clones a disposable VM from the base image
#   3. Boots the VM with a shared directory containing the keypair and binary
#   4. Installs the public key into the VM via the mount, then uses key auth
#   5. Runs: bd update -> bd install jq -> verify -> bd upgrade -> bd --dry-run
#   6. Destroys the VM (unless --keep is passed)
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

VM_NAME="brewdock-smoke-$$"
BASE_IMAGE="ghcr.io/cirruslabs/macos-sequoia-base:latest"
BD_BINARY="$PROJECT_ROOT/target/release/bd"
SSH_USER="admin"
SSH_PASS="admin"
KEEP_VM=false
SHARE_DIR=""
SSH_KEY=""

for arg in "$@"; do
  case "$arg" in
    --keep) KEEP_VM=true ;;
  esac
done

# --- Helpers ----------------------------------------------------------------

log()  { printf '\033[1;34m==> %s\033[0m\n' "$*"; }
pass() { printf '\033[1;32m  PASS: %s\033[0m\n' "$*"; }
fail() { printf '\033[1;31m  FAIL: %s\033[0m\n' "$*"; exit 1; }

cleanup() {
  if [ "$KEEP_VM" = true ]; then
    log "Keeping VM '$VM_NAME' (--keep). Stop/delete manually:"
    echo "  tart stop $VM_NAME; tart delete $VM_NAME"
  else
    log "Cleaning up VM '$VM_NAME'..."
    tart stop "$VM_NAME" 2>/dev/null || true
    tart delete "$VM_NAME" 2>/dev/null || true
  fi

  if [ -n "$SHARE_DIR" ]; then
    rm -rf "$SHARE_DIR"
  fi
}
trap cleanup EXIT

vm_ssh() {
  ssh -i "$SSH_KEY" \
    -o StrictHostKeyChecking=no \
    -o UserKnownHostsFile=/dev/null \
    -o LogLevel=ERROR \
    "$SSH_USER@$VM_IP" "$@"
}

vm_scp() {
  scp -i "$SSH_KEY" \
    -o StrictHostKeyChecking=no \
    -o UserKnownHostsFile=/dev/null \
    -o LogLevel=ERROR \
    "$@"
}

# Feed password to an interactive SSH command via expect (macOS built-in).
# Used only once to bootstrap the SSH key; all subsequent calls use key auth.
ssh_with_password() {
  expect -c "
    set timeout 30
    spawn ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
              -o LogLevel=ERROR $SSH_USER@$VM_IP $*
    expect {
      \"*assword*\" { send \"$SSH_PASS\r\"; exp_continue }
      eof
    }
    lassign [wait] pid spawnid os_error value
    exit \$value
  "
}

# --- Preflight --------------------------------------------------------------

log "Preflight checks"

if ! command -v tart &>/dev/null; then
  fail "tart not found. Install: brew install cirruslabs/cli/tart"
fi

if ! command -v expect &>/dev/null; then
  fail "expect not found (should be pre-installed on macOS)"
fi

if [ ! -x "$BD_BINARY" ]; then
  fail "Release binary not found at $BD_BINARY. Run: cargo build --release -p brewdock-cli"
fi

# --- Temporary SSH key + shared directory ------------------------------------

log "Generating temporary SSH keypair"
SHARE_DIR=$(mktemp -d)
SSH_KEY="$SHARE_DIR/id_ed25519"
ssh-keygen -t ed25519 -f "$SSH_KEY" -N "" -q
cp "$BD_BINARY" "$SHARE_DIR/bd"
chmod +x "$SHARE_DIR/bd"
pass "Keypair + binary staged in $SHARE_DIR"

# --- VM Lifecycle ------------------------------------------------------------

log "Cloning VM '$VM_NAME' from $BASE_IMAGE"
tart clone "$BASE_IMAGE" "$VM_NAME"

log "Starting VM (headless) with shared directory..."
tart run "$VM_NAME" --no-graphics --dir="brewdock:$SHARE_DIR" &
VM_PID=$!

log "Waiting for VM IP..."
VM_IP=""
for i in $(seq 1 60); do
  VM_IP=$(tart ip "$VM_NAME" 2>/dev/null || true)
  if [ -n "$VM_IP" ]; then
    break
  fi
  sleep 2
done

if [ -z "$VM_IP" ]; then
  fail "Could not obtain VM IP after 120s"
fi
log "VM IP: $VM_IP"

log "Waiting for SSH (password auth)..."
for i in $(seq 1 30); do
  if ssh_with_password "true" &>/dev/null; then
    break
  fi
  sleep 2
done

# --- Bootstrap SSH key auth --------------------------------------------------

log "Installing SSH public key into VM..."

# The shared directory is auto-mounted at /Volumes/My Shared Files/brewdock
# by the Virtualization framework. Wait for it to appear.
MOUNT_PATH="/Volumes/My Shared Files/brewdock"
for i in $(seq 1 20); do
  if ssh_with_password "test -d '$MOUNT_PATH'" &>/dev/null; then
    break
  fi
  sleep 3
done

ssh_with_password "mkdir -p ~/.ssh && chmod 700 ~/.ssh && cat '$MOUNT_PATH/id_ed25519.pub' >> ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys"

# Verify key auth works
if ! vm_ssh "true" 2>/dev/null; then
  fail "SSH key auth failed after bootstrap"
fi
pass "SSH key auth established"

# --- Setup inside VM --------------------------------------------------------

log "Copying bd binary from shared mount..."
vm_ssh "cp '$MOUNT_PATH/bd' /tmp/bd && chmod +x /tmp/bd"
pass "Binary ready at /tmp/bd"

log "Creating clean /opt/homebrew prefix..."
# The base image ships with Homebrew pre-installed. Wipe it to test
# brewdock on a clean prefix (this is a disposable VM).
vm_ssh "sudo rm -rf /opt/homebrew && sudo mkdir -p /opt/homebrew && sudo chown \$USER /opt/homebrew"
pass "Prefix ready (clean)"

# --- Test: bd update ---------------------------------------------------------

log "Running: bd update"
vm_ssh "/tmp/bd update" || fail "bd update failed"
pass "bd update"

# --- Test: bd install jq -----------------------------------------------------

log "Running: bd install jq"
vm_ssh "/tmp/bd install jq" || fail "bd install jq failed"
pass "bd install jq"

log "Verifying: which jq"
JQ_PATH=$(vm_ssh 'export PATH="/opt/homebrew/bin:$PATH" && which jq' 2>/dev/null || true)
if [ -z "$JQ_PATH" ]; then
  vm_ssh "test -L /opt/homebrew/bin/jq" || fail "/opt/homebrew/bin/jq symlink does not exist"
  JQ_PATH="/opt/homebrew/bin/jq"
fi
pass "jq found at $JQ_PATH"

log "Verifying: jq --version"
JQ_VERSION=$(vm_ssh "/opt/homebrew/bin/jq --version 2>&1" || true)
if [ -z "$JQ_VERSION" ]; then
  fail "jq --version produced no output"
fi
pass "jq version: $JQ_VERSION"

log "Verifying: Cellar structure"
vm_ssh "ls /opt/homebrew/Cellar/jq/" || fail "Cellar/jq/ does not exist"
vm_ssh "test -f /opt/homebrew/Cellar/jq/*/INSTALL_RECEIPT.json" || fail "INSTALL_RECEIPT.json missing"
pass "Cellar structure valid"

log "Verifying: opt symlink"
vm_ssh "test -L /opt/homebrew/opt/jq" || fail "opt/jq symlink missing"
pass "opt symlink exists"

log "Verifying: oniguruma dependency installed"
vm_ssh "test -d /opt/homebrew/Cellar/oniguruma" || fail "dependency oniguruma not installed"
pass "oniguruma dependency present"

# --- Test: bd upgrade (already up-to-date) -----------------------------------

log "Running: bd upgrade (should be up-to-date)"
UPGRADE_OUTPUT=$(vm_ssh "/tmp/bd upgrade" 2>&1 || true)
echo "$UPGRADE_OUTPUT"
if echo "$UPGRADE_OUTPUT" | grep -q "Already up-to-date"; then
  pass "bd upgrade (no-op)"
else
  fail "bd upgrade should report up-to-date after fresh install"
fi

# --- Test: bd upgrade (actual version bump) ----------------------------------

log "Faking old jq version in state DB..."
REAL_VERSION=$(vm_ssh "ls /opt/homebrew/Cellar/jq/" 2>/dev/null | head -1)
vm_ssh "sqlite3 /opt/homebrew/var/brewdock/brewdock.db \"UPDATE installs SET version='0.0.0-fake' WHERE name='jq'\""
FAKED=$(vm_ssh "sqlite3 /opt/homebrew/var/brewdock/brewdock.db \"SELECT version FROM installs WHERE name='jq'\"")
if [ "$FAKED" != "0.0.0-fake" ]; then
  fail "failed to fake version in state DB (got: $FAKED)"
fi
pass "State DB faked to 0.0.0-fake"

log "Running: bd upgrade jq (should download and install)"
UPGRADE_OUTPUT=$(vm_ssh "/tmp/bd upgrade jq" 2>&1 || true)
echo "$UPGRADE_OUTPUT"
if echo "$UPGRADE_OUTPUT" | grep -q "Upgraded jq"; then
  pass "bd upgrade jq"
else
  fail "bd upgrade jq did not report upgrade"
fi

log "Verifying: jq still works after upgrade"
JQ_POST_UPGRADE=$(vm_ssh "/opt/homebrew/bin/jq --version 2>&1" || true)
if [ -z "$JQ_POST_UPGRADE" ]; then
  fail "jq --version failed after upgrade"
fi
pass "jq after upgrade: $JQ_POST_UPGRADE"

log "Verifying: state DB updated"
DB_VERSION=$(vm_ssh "sqlite3 /opt/homebrew/var/brewdock/brewdock.db \"SELECT version FROM installs WHERE name='jq'\"")
if [ "$DB_VERSION" = "0.0.0-fake" ]; then
  fail "state DB still has fake version after upgrade"
fi
pass "state DB version: $DB_VERSION"

# --- Test: bd install --dry-run ----------------------------------------------

log "Running: bd install --dry-run ripgrep"
DRY_OUTPUT=$(vm_ssh "/tmp/bd install --dry-run ripgrep" 2>&1 || true)
echo "$DRY_OUTPUT"
if echo "$DRY_OUTPUT" | grep -qE "(Would install|Nothing to install)"; then
  pass "bd install --dry-run"
else
  fail "bd install --dry-run produced unexpected output"
fi

if vm_ssh "test -d /opt/homebrew/Cellar/ripgrep" 2>/dev/null; then
  fail "dry-run installed ripgrep (should not have)"
fi
pass "dry-run did not install"

# --- Summary -----------------------------------------------------------------

echo ""
log "All smoke tests passed!"
echo ""
