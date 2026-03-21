#!/usr/bin/env bash
#
# Destructive smoke test for brewdock using a Tart macOS VM.
#
# Usage:
#   ./tests/vm-smoke-test.sh [--keep] [--formula <name> ...]
#                            [--skip-cross] [--phase <phases>]
#
# Phases (comma-separated):
#   install  — installability sweep + usability + deep verification
#   upgrade  — bd upgrade tests + dry-run
#   cross    — Homebrew install + bd↔brew cross-compatibility
#   Default: all phases. --skip-cross removes cross from whatever phases run.
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
#   5. Runs: bd update -> installability sweep -> deep verification (if jq requested) -> bd --dry-run
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

# Formulae for cross-compatibility tests (bd ↔ brew).
# Kept small for speed; chosen for simplicity and reliable bottles.
readonly CROSS_TEST_BD_TO_BREW=(jq ripgrep bat)
readonly CROSS_TEST_BREW_TO_BD=(tree figlet)

VM_NAME="brewdock-smoke-$$"
BASE_IMAGE="ghcr.io/cirruslabs/macos-sequoia-base:latest"
BD_BINARY="$PROJECT_ROOT/target/release/bd"
SSH_USER="admin"
SSH_PASS="admin"
KEEP_VM=false
SKIP_CROSS=false
SHARE_DIR=""
SSH_KEY=""
FORMULAE=()
PHASES=()
INSTALL_RESULTS=()
FAILED_FORMULAE=()
USABILITY_RESULTS=()
USABILITY_FAILURES=()
CROSS_RESULTS=()
CROSS_FAILURES=()

while [ "$#" -gt 0 ]; do
  case "$1" in
    --keep)
      KEEP_VM=true
      shift
      ;;
    --skip-cross)
      SKIP_CROSS=true
      shift
      ;;
    --phase)
      shift
      if [ "$#" -eq 0 ]; then
        echo "missing value for --phase" >&2
        exit 1
      fi
      IFS=',' read -ra PHASES <<< "$1"
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

should_run_phase() {
  local phase=$1
  if [ "$phase" = "cross" ] && [ "$SKIP_CROSS" = true ]; then
    return 1
  fi
  if [ "${#PHASES[@]}" -eq 0 ]; then
    return 0
  fi
  printf '%s\n' "${PHASES[@]}" | grep -qx "$phase"
}

# Returns 0 if the formula is in the FORMULAE list.
is_formula_requested() {
  printf '%s\n' "${FORMULAE[@]}" | grep -qx "$1"
}

record_install_result() {
  local formula=$1
  local status=$2
  local detail=$3

  INSTALL_RESULTS+=("$formula:$status:$detail")
  if [ "$status" != "PASS" ]; then
    FAILED_FORMULAE+=("$formula")
  fi
}

record_cross_result() {
  local label=$1
  local status=$2
  local detail=$3

  CROSS_RESULTS+=("$label:$status:$detail")
  if [ "$status" != "PASS" ]; then
    CROSS_FAILURES+=("$label")
  fi
}

# Returns the verification command for a formula, or empty string if none.
# Commands are expected to exit 0 on success. PATH is set by the caller.
get_verify_command() {
  case "$1" in
    actionlint)                    echo "actionlint -version" ;;
    agent-browser)                 echo "agent-browser --version" ;;
    ast-grep)                      echo "sg --version" ;;
    atuin)                         echo "atuin --version" ;;
    bash)                          echo "bash --version" ;;
    bat)                           echo "bat --version" ;;
    curl)                          echo "curl --version" ;;
    direnv)                        echo "direnv version" ;;
    eza)                           echo "eza --version" ;;
    fd)                            echo "fd --version" ;;
    fnm)                           echo "fnm --version" ;;
    fzf)                           echo "fzf --version" ;;
    gh)                            echo "gh --version" ;;
    ghalint)                       echo "ghalint --version" ;;
    ghq)                           echo "ghq --version" ;;
    git)                           echo "git --version" ;;
    git-delta)                     echo "delta --version" ;;
    git-secrets)                   echo "test -x /opt/homebrew/bin/git-secrets" ;;
    gitleaks)                      echo "gitleaks version" ;;
    glow)                          echo "glow --version" ;;
    gnu-sed)                       echo "gsed --version" ;;
    go-task)                       echo "task --version" ;;
    httpie)                        echo "http --version" ;;
    htop)                          echo "htop --version" ;;
    figlet)                        echo "figlet -v" ;;
    pv)                            echo "pv --version" ;;
    jq)                            echo "jq --version" ;;
    neovim)                        echo "nvim --version" ;;
    pinact)                        echo "pinact version" ;;
    ripgrep)                       echo "rg --version" ;;
    rtk)                           echo "rtk --version" ;;
    sd)                            echo "sd --version" ;;
    semgrep)                       echo "semgrep --version" ;;
    shellcheck)                    echo "shellcheck --version" ;;
    shfmt)                         echo "shfmt --version" ;;
    sops)                          echo "sops --version" ;;
    sqlmap)                        echo "test -x /opt/homebrew/bin/sqlmap" ;;
    starship)                      echo "starship --version" ;;
    topgrade)                      echo "topgrade --version" ;;
    toxiproxy)                     echo "toxiproxy-cli --version" ;;
    tree)                          echo "tree --version" ;;
    vim)                           echo "vim --version" ;;
    wakeonlan)                     echo "test -x /opt/homebrew/bin/wakeonlan" ;;
    wget)                          echo "wget --version" ;;
    yq)                            echo "yq --version" ;;
    zoxide)                        echo "zoxide --version" ;;
    zsh-completions)               echo "test -d /opt/homebrew/share/zsh/site-functions" ;;
    zsh-fast-syntax-highlighting)  echo "test -f /opt/homebrew/share/zsh-fast-syntax-highlighting/fast-syntax-highlighting.plugin.zsh" ;;
    *)                             echo "" ;;
  esac
}

run_usability_check() {
  local formula=$1
  local verify_cmd=""

  verify_cmd=$(get_verify_command "$formula")
  if [ -z "$verify_cmd" ]; then
    USABILITY_RESULTS+=("$formula:SKIP:no verification command")
    return
  fi

  local output=""
  if output=$(vm_ssh "export PATH=\"/opt/homebrew/bin:/opt/homebrew/sbin:\$PATH\" && $verify_cmd" 2>&1); then
    USABILITY_RESULTS+=("$formula:PASS:$(echo "$output" | head -1)")
  else
    USABILITY_RESULTS+=("$formula:FAIL:exit $?")
    USABILITY_FAILURES+=("$formula")
  fi
}

print_usability_summary() {
  local result=""
  local formula=""
  local status=""
  local detail=""

  echo ""
  log "Usability summary"
  for result in "${USABILITY_RESULTS[@]}"; do
    formula=${result%%:*}
    status=${result#*:}
    detail=${status#*:}
    status=${status%%:*}
    printf '  %-30s %s (%s)\n' "$formula" "$status" "$detail"
  done
}

print_cross_summary() {
  local result=""
  local label=""
  local status=""
  local detail=""

  echo ""
  log "Cross-compatibility summary"
  for result in "${CROSS_RESULTS[@]}"; do
    label=${result%%:*}
    status=${result#*:}
    detail=${status#*:}
    status=${status%%:*}
    printf '  %-40s %s (%s)\n' "$label" "$status" "$detail"
  done
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

  # Verify Homebrew-visible install state: receipt must exist in the keg.
  if ! vm_ssh "ls /opt/homebrew/Cellar/$formula/*/INSTALL_RECEIPT.json" >/dev/null 2>&1; then
    record_install_result "$formula" "FAIL" "INSTALL_RECEIPT.json missing"
    return
  fi

  # Extract version from the keg directory name.
  local keg_version=""
  keg_version=$(vm_ssh "ls /opt/homebrew/Cellar/$formula/ | head -1" 2>/dev/null || true)
  record_install_result "$formula" "PASS" "$keg_version"
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

if should_run_phase "install"; then

for formula in "${FORMULAE[@]}"; do
  run_install_check "$formula"
done

print_install_summary

if [ "${#FAILED_FORMULAE[@]}" -gt 0 ]; then
  echo ""
  log "Failed formulae: ${FAILED_FORMULAE[*]}"
fi

# --- Test: usability verification --------------------------------------------

log "Usability verification"
for formula in "${FORMULAE[@]}"; do
  if printf '%s\n' "${FAILED_FORMULAE[@]}" | grep -qx "$formula"; then
    USABILITY_RESULTS+=("$formula:SKIP:install failed")
    continue
  fi
  run_usability_check "$formula"
done

print_usability_summary

if [ "${#USABILITY_FAILURES[@]}" -gt 0 ]; then
  echo ""
  log "Usability failures: ${USABILITY_FAILURES[*]}"
fi

# --- Test: jq deep verification ----------------------------------------------

if is_formula_requested "$PRIMARY_FORMULA"; then

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

else
  log "Skipping $PRIMARY_FORMULA deep verification (not in --formula list)"
fi

fi # install phase

# --- Test: bd upgrade (already up-to-date) -----------------------------------

if should_run_phase "upgrade"; then

log "Running: bd upgrade (should be up-to-date)"
UPGRADE_OUTPUT=$(vm_ssh "/tmp/bd upgrade" 2>&1 || true)
echo "$UPGRADE_OUTPUT"
if echo "$UPGRADE_OUTPUT" | grep -q "Already up-to-date"; then
  pass "bd upgrade (no-op)"
else
  fail "bd upgrade should report up-to-date after fresh install"
fi

# --- Test: bd upgrade (actual version bump) ----------------------------------

if is_formula_requested "$PRIMARY_FORMULA"; then

log "Faking old $PRIMARY_FORMULA version by renaming keg directory..."
REAL_JQ_VERSION=$(vm_ssh "ls /opt/homebrew/Cellar/$PRIMARY_FORMULA/ | head -1")
vm_ssh "mv /opt/homebrew/Cellar/$PRIMARY_FORMULA/$REAL_JQ_VERSION /opt/homebrew/Cellar/$PRIMARY_FORMULA/0.0.0-fake"
vm_ssh "ln -sfn ../Cellar/$PRIMARY_FORMULA/0.0.0-fake /opt/homebrew/opt/$PRIMARY_FORMULA"
FAKED=$(vm_ssh "ls /opt/homebrew/Cellar/$PRIMARY_FORMULA/ | head -1")
if [ "$FAKED" != "0.0.0-fake" ]; then
  fail "failed to fake version in Cellar (got: $FAKED)"
fi
pass "Cellar faked to 0.0.0-fake"

log "Running: bd upgrade $PRIMARY_FORMULA (should download and install)"
UPGRADE_OUTPUT=$(vm_ssh "/tmp/bd upgrade $PRIMARY_FORMULA" 2>&1 || true)
echo "$UPGRADE_OUTPUT"
if echo "$UPGRADE_OUTPUT" | grep -q "Upgraded $PRIMARY_FORMULA"; then
  pass "bd upgrade $PRIMARY_FORMULA"
else
  fail "bd upgrade $PRIMARY_FORMULA did not report upgrade"
fi

log "Verifying: $PRIMARY_FORMULA still works after upgrade"
JQ_POST_UPGRADE=$(vm_ssh "/opt/homebrew/bin/$PRIMARY_FORMULA --version 2>&1" || true)
if [ -z "$JQ_POST_UPGRADE" ]; then
  fail "$PRIMARY_FORMULA --version failed after upgrade"
fi
pass "$PRIMARY_FORMULA after upgrade: $JQ_POST_UPGRADE"

log "Verifying: keg version updated"
KEG_VERSION=$(vm_ssh "ls /opt/homebrew/Cellar/$PRIMARY_FORMULA/ | grep -v fake | head -1" || true)
if [ -z "$KEG_VERSION" ]; then
  fail "no real version keg found after upgrade"
fi
pass "keg version: $KEG_VERSION"

else
  log "Skipping $PRIMARY_FORMULA upgrade test (not in --formula list)"
fi

# --- Test: bd install --dry-run ----------------------------------------------

log "Running: bd install --dry-run $DRY_RUN_FORMULA"
DRY_OUTPUT=$(vm_ssh "/tmp/bd install --dry-run $DRY_RUN_FORMULA" 2>&1 || true)
echo "$DRY_OUTPUT"
if echo "$DRY_OUTPUT" | grep -qE "(Would install|Nothing to install)"; then
  pass "bd install --dry-run"
else
  fail "bd install --dry-run produced unexpected output"
fi

fi # upgrade phase

# --- Cross-test: Install Homebrew --------------------------------------------

if should_run_phase "cross"; then

log "Installing Homebrew for cross-compatibility tests..."
vm_ssh "NONINTERACTIVE=1 /bin/bash -c \"\$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\"" \
  || fail "Homebrew installation failed"
pass "Homebrew installed"

log "Running: brew update"
vm_ssh "/opt/homebrew/bin/brew update" || fail "brew update failed"
pass "brew update"

# --- Cross-test: bd → brew upgrade ------------------------------------------
#
# Verifies that packages installed by bd are recognized by brew and survive
# a brew upgrade cycle.

log "Cross-test: bd → brew upgrade"
for formula in "${CROSS_TEST_BD_TO_BREW[@]}"; do
  if ! is_formula_requested "$formula"; then
    continue
  fi
  if printf '%s\n' "${FAILED_FORMULAE[@]}" | grep -qx "$formula"; then
    record_cross_result "bd→brew:$formula" "SKIP" "install failed"
    continue
  fi

  log "  brew list $formula"
  if ! vm_ssh "/opt/homebrew/bin/brew list $formula" >/dev/null 2>&1; then
    record_cross_result "bd→brew:$formula" "FAIL" "brew does not recognize"
    continue
  fi
  pass "brew recognizes $formula"

  log "  brew upgrade $formula"
  brew_upgrade_output=$(vm_ssh "/opt/homebrew/bin/brew upgrade $formula" 2>&1 || true)
  echo "$brew_upgrade_output"
  pass "brew upgrade $formula completed"

  verify_cmd=$(get_verify_command "$formula")
  if [ -n "$verify_cmd" ]; then
    if vm_ssh "export PATH=\"/opt/homebrew/bin:/opt/homebrew/sbin:\$PATH\" && $verify_cmd" >/dev/null 2>&1; then
      record_cross_result "bd→brew:$formula" "PASS" "works after brew upgrade"
    else
      record_cross_result "bd→brew:$formula" "FAIL" "broken after brew upgrade"
    fi
  else
    record_cross_result "bd→brew:$formula" "PASS" "brew upgrade completed"
  fi
done

# --- Cross-test: brew → bd install ------------------------------------------
#
# Verifies that packages installed by brew can be taken over by bd install,
# and that bd upgrade works after the takeover.

log "Cross-test: brew install → bd install/upgrade"
for formula in "${CROSS_TEST_BREW_TO_BD[@]}"; do
  log "  brew install $formula"
  if ! vm_ssh "/opt/homebrew/bin/brew install $formula" 2>&1; then
    record_cross_result "brew→bd:$formula" "FAIL" "brew install failed"
    continue
  fi
  pass "brew installed $formula"

  # Verify it works after brew install
  verify_cmd=$(get_verify_command "$formula")
  if [ -n "$verify_cmd" ]; then
    if ! vm_ssh "export PATH=\"/opt/homebrew/bin:/opt/homebrew/sbin:\$PATH\" && $verify_cmd" >/dev/null 2>&1; then
      record_cross_result "brew→bd:$formula" "FAIL" "broken after brew install"
      continue
    fi
    pass "$formula works after brew install"
  fi

  # bd install (takeover): formula is already in Cellar with receipt from brew.
  # bd should detect it as already installed via Homebrew-visible filesystem state.
  log "  bd install $formula (takeover)"
  bd_install_output=$(vm_ssh "/tmp/bd install $formula" 2>&1 || true)
  echo "$bd_install_output"

  if [ -n "$verify_cmd" ]; then
    if ! vm_ssh "export PATH=\"/opt/homebrew/bin:/opt/homebrew/sbin:\$PATH\" && $verify_cmd" >/dev/null 2>&1; then
      record_cross_result "brew→bd:$formula" "FAIL" "broken after bd install"
      continue
    fi
    pass "$formula works after bd install"
  fi

  # Verify filesystem state: receipt must exist in the keg.
  if ! vm_ssh "ls /opt/homebrew/Cellar/$formula/*/INSTALL_RECEIPT.json" >/dev/null 2>&1; then
    record_cross_result "brew→bd:$formula" "FAIL" "INSTALL_RECEIPT.json missing after bd install"
    continue
  fi
  keg_ver=$(vm_ssh "ls /opt/homebrew/Cellar/$formula/ | head -1" 2>/dev/null || true)
  pass "$formula in Cellar ($keg_ver)"

  # bd upgrade (should work now that formula is visible via filesystem state)
  log "  bd upgrade $formula"
  bd_upgrade_output=$(vm_ssh "/tmp/bd upgrade $formula" 2>&1 || true)
  echo "$bd_upgrade_output"

  if [ -n "$verify_cmd" ]; then
    if vm_ssh "export PATH=\"/opt/homebrew/bin:/opt/homebrew/sbin:\$PATH\" && $verify_cmd" >/dev/null 2>&1; then
      record_cross_result "brew→bd:$formula" "PASS" "works after full cycle"
    else
      record_cross_result "brew→bd:$formula" "FAIL" "broken after bd upgrade"
    fi
  else
    record_cross_result "brew→bd:$formula" "PASS" "full cycle completed"
  fi
done

print_cross_summary

fi # cross phase

# --- Summary -----------------------------------------------------------------

echo ""
FAILURES=()

if [ "${#FAILED_FORMULAE[@]}" -gt 0 ]; then
  FAILURES+=("install: ${FAILED_FORMULAE[*]}")
fi

if [ "${#USABILITY_FAILURES[@]}" -gt 0 ]; then
  FAILURES+=("usability: ${USABILITY_FAILURES[*]}")
fi

if [ "${#CROSS_FAILURES[@]}" -gt 0 ]; then
  FAILURES+=("cross-test: ${CROSS_FAILURES[*]}")
fi

if [ "${#FAILURES[@]}" -gt 0 ]; then
  for f in "${FAILURES[@]}"; do
    log "FAILED — $f"
  done
  fail "smoke tests found failures"
fi

log "All smoke tests passed!"
echo ""
