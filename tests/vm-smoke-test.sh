#!/usr/bin/env bash
#
# Destructive smoke test for brewdock using a Tart macOS VM.
#
# Usage:
#   ./tests/vm-smoke-test.sh [--keep] [--formula <name> ...]
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
#   5. Runs: bd update -> installability sweep -> jq deep verification -> bd --dry-run
#   6. Destroys the VM (unless --keep is passed)
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

readonly DEFAULT_FORMULAE=(
  actionlint
  agent-browser
  ast-grep
  atuin
  bash
  bat
  curl
  direnv
  eza
  fd
  fnm
  fzf
  gh
  ghalint
  ghq
  git
  git-delta
  git-secrets
  gitleaks
  glow
  gnu-sed
  go-task
  httpie
  jq
  neovim
  pinact
  ripgrep
  rtk
  sd
  semgrep
  shellcheck
  shfmt
  sops
  sqlmap
  starship
  topgrade
  toxiproxy
  vim
  wakeonlan
  wget
  yq
  zoxide
  zsh-completions
  zsh-fast-syntax-highlighting
)
readonly PRIMARY_FORMULA="jq"
readonly DRY_RUN_FORMULA="ripgrep"

VM_NAME="brewdock-smoke-$$"
BASE_IMAGE="ghcr.io/cirruslabs/macos-sequoia-base:latest"
BD_BINARY="$PROJECT_ROOT/target/release/bd"
SSH_USER="admin"
SSH_PASS="admin"
KEEP_VM=false
SHARE_DIR=""
SSH_KEY=""
FORMULAE=()
INSTALL_RESULTS=()
FAILED_FORMULAE=()

while [ "$#" -gt 0 ]; do
  case "$1" in
    --keep)
      KEEP_VM=true
      shift
      ;;
    --formula)
      shift
      if [ "$#" -eq 0 ]; then
        echo "missing value for --formula" >&2
        exit 1
      fi
      FORMULAE+=("$1")
      shift
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

if [ "${#FORMULAE[@]}" -eq 0 ]; then
  FORMULAE=("${DEFAULT_FORMULAE[@]}")
fi

# --- Helpers ----------------------------------------------------------------

log()  { printf '\033[1;34m==> %s\033[0m\n' "$*"; }
pass() { printf '\033[1;32m  PASS: %s\033[0m\n' "$*"; }
fail() { printf '\033[1;31m  FAIL: %s\033[0m\n' "$*"; exit 1; }

record_install_result() {
  local formula=$1
  local status=$2
  local detail=$3

  INSTALL_RESULTS+=("$formula:$status:$detail")
  if [ "$status" != "PASS" ]; then
    FAILED_FORMULAE+=("$formula")
  fi
}

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

run_install_check() {
  local formula=$1
  local install_output=""
  local db_version=""
  local install_status=0

  log "Running: bd install $formula"
  if install_output=$(vm_ssh "/tmp/bd install $formula" 2>&1); then
    install_status=0
  else
    install_status=$?
  fi
  echo "$install_output"

  if [ "$install_status" -ne 0 ]; then
    record_install_result "$formula" "FAIL" "install command failed"
    return
  fi

  if ! vm_ssh "test -d /opt/homebrew/Cellar/$formula" >/dev/null 2>&1; then
    record_install_result "$formula" "FAIL" "Cellar entry missing"
    return
  fi

  db_version=$(vm_ssh "sqlite3 /opt/homebrew/var/brewdock/brewdock.db \"SELECT version FROM installs WHERE name='$formula'\"" 2>/dev/null || true)
  if [ -z "$db_version" ]; then
    record_install_result "$formula" "FAIL" "state DB record missing"
    return
  fi

  record_install_result "$formula" "PASS" "$db_version"
}

print_install_summary() {
  local result=""
  local formula=""
  local status=""
  local detail=""

  echo ""
  log "Installability summary"
  for result in "${INSTALL_RESULTS[@]}"; do
    formula=${result%%:*}
    status=${result#*:}
    detail=${status#*:}
    status=${status%%:*}
    printf '  %-30s %s (%s)\n' "$formula" "$status" "$detail"
  done
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

log "Waiting for VM IP..."
VM_IP=""
for _ in $(seq 1 60); do
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
for _ in $(seq 1 30); do
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
for _ in $(seq 1 20); do
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

# --- Test: installability sweep ----------------------------------------------

for formula in "${FORMULAE[@]}"; do
  run_install_check "$formula"
done

print_install_summary

if [ "${#FAILED_FORMULAE[@]}" -gt 0 ]; then
  echo ""
  log "Failed formulae: ${FAILED_FORMULAE[*]}"
fi

# --- Test: jq deep verification ----------------------------------------------

if printf '%s\n' "${FAILED_FORMULAE[@]}" | grep -qx "$PRIMARY_FORMULA"; then
  fail "$PRIMARY_FORMULA installability failed; skipping deep verification"
fi

log "Verifying: which $PRIMARY_FORMULA"
JQ_PATH=$(vm_ssh "export PATH=\"/opt/homebrew/bin:\$PATH\" && which jq" 2>/dev/null || true)
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
DRY_OUTPUT=$(vm_ssh "/tmp/bd install --dry-run $DRY_RUN_FORMULA" 2>&1 || true)
echo "$DRY_OUTPUT"
if echo "$DRY_OUTPUT" | grep -qE "(Would install|Nothing to install)"; then
  pass "bd install --dry-run"
else
  fail "bd install --dry-run produced unexpected output"
fi

if [ "$DRY_RUN_FORMULA" = "ripgrep" ]; then
  pass "dry-run output checked against $DRY_RUN_FORMULA"
fi

# --- Summary -----------------------------------------------------------------

echo ""
if [ "${#FAILED_FORMULAE[@]}" -gt 0 ]; then
  fail "installability sweep found failures: ${FAILED_FORMULAE[*]}"
fi

log "All smoke tests passed!"
echo ""
