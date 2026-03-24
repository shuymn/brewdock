#!/usr/bin/env bash
#
# Benchmark Homebrew ecosystem package managers inside a disposable Tart macOS VM.
#
# Usage:
#   ./tests/vm-benchmark.sh [--keep]
#                          [--formula <name> ...]
#                          [--formula-set <a,b,...> ...]
#                          [--manager <name> ...]
#
# Managers:
#   homebrew | brewdock | zerobrew | nanobrew
#
# Methodology:
#   - Mirrors the public nanobrew / zerobrew benchmark style: single wall-clock
#     timing via `time`, then warm re-install with caches preserved.
#   - Homebrew is measured once per formula, matching the published benchmark
#     tables. brewdock / zerobrew / nanobrew record cold and warm timings.
#   - brewdock has no uninstall command yet, so warm runs preserve
#     `/opt/homebrew/var/brewdock/{cache,blobs,store}` and rebuild the prefix.
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
. "$SCRIPT_DIR/vm-config.sh"

readonly DEFAULT_FORMULAE=(
  tree
  wget
  ffmpeg
)
readonly DEFAULT_FORMULA_SETS=()
readonly DEFAULT_MANAGERS=(
  brewdock
  zerobrew
  nanobrew
  homebrew
)

BASE_IMAGE="$BREWDOCK_VM_BASE_IMAGE"
VM_NAME="brewdock-benchmark-$$"
SSH_USER="admin"
SSH_PASS="admin"
BD_BINARY="$PROJECT_ROOT/target/release/bd"
KEEP_VM=false
FORMULAE=()
FORMULA_SETS=()
SCENARIOS=()
REQUESTED_MANAGERS=()
MANAGERS=()
SHARE_DIR=""
SSH_KEY=""
VM_IP=""
RESULTS=()

log()  { printf '\033[1;34m==> %s\033[0m\n' "$*"; }
pass() { printf '\033[1;32m  PASS: %s\033[0m\n' "$*"; }
fail() { printf '\033[1;31m  FAIL: %s\033[0m\n' "$*"; exit 1; }

usage() {
  cat <<'EOF'
Usage:
  ./tests/vm-benchmark.sh [--keep] [--formula <name> ...] [--formula-set <a,b,...> ...] [--manager <name> ...]

Examples:
  ./tests/vm-benchmark.sh
  ./tests/vm-benchmark.sh --formula tree --formula wget
  ./tests/vm-benchmark.sh --formula-set jq,wget
  ./tests/vm-benchmark.sh --manager brewdock --manager homebrew --formula tree
EOF
}

cleanup() {
  if [ "$KEEP_VM" = true ]; then
    log "Keeping VM '$VM_NAME' (--keep). Stop/delete manually:"
    echo "  tart stop $VM_NAME; tart delete $VM_NAME"
  else
    tart stop "$VM_NAME" >/dev/null 2>&1 || true
    tart delete "$VM_NAME" >/dev/null 2>&1 || true
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

manager_selected() {
  local target=$1
  local manager=""

  for manager in "${REQUESTED_MANAGERS[@]}"; do
    if [ "$manager" = "$target" ]; then
      return 0
    fi
  done
  return 1
}

order_managers() {
  local ordered=()
  local manager=""

  for manager in "${DEFAULT_MANAGERS[@]}"; do
    if manager_selected "$manager"; then
      ordered+=("$manager")
    fi
  done

  MANAGERS=("${ordered[@]}")
}

record_result() {
  local scenario=$1
  local manager=$2
  local phase=$3
  local status=$4
  local elapsed=$5
  local note=$6

  RESULTS+=("$scenario:$manager:$phase:$status:$elapsed:$note")
}

add_scenario() {
  local label=$1
  local packages=$2

  SCENARIOS+=("${label}|${packages}")
}

scenario_label() {
  printf '%s' "${1%%|*}"
}

scenario_packages() {
  printf '%s' "${1#*|}"
}

scenario_packages_as_words() {
  printf '%s' "$1" | tr ',' ' '
}

remove_zerobrew_packages() {
  local packages_csv=$1
  local package=""
  local -a package_names=()

  IFS=',' read -r -a package_names <<< "$packages_csv"
  for package in "${package_names[@]}"; do
    vm_ssh "PATH=\"\$HOME/.local/bin:\$PATH\" ~/.local/bin/zb uninstall $package >/dev/null 2>&1 || true"
  done
}

remove_nanobrew_packages() {
  local packages_csv=$1
  local package=""
  local -a package_names=()

  IFS=',' read -r -a package_names <<< "$packages_csv"
  for package in "${package_names[@]}"; do
    vm_ssh "PATH=\"/opt/nanobrew/prefix/bin:\$PATH\" /opt/nanobrew/prefix/bin/nb remove $package >/dev/null 2>&1 || true"
  done
}

remove_homebrew_packages() {
  local packages_csv=$1
  local package=""
  local brew="/opt/homebrew/bin/brew"
  local -a package_names=()

  IFS=',' read -r -a package_names <<< "$packages_csv"
  for package in "${package_names[@]}"; do
    vm_ssh "$brew uninstall $package >/dev/null 2>&1 || true"
  done
}

measure_vm_command() {
  local command=$1
  local encoded_command=""
  local output=""
  local status=0

  encoded_command=$(printf '%s' "$command" | base64)

  if output=$(vm_ssh "printf '%s' '$encoded_command' | base64 -d >/tmp/codex-bench-command.sh && chmod +x /tmp/codex-bench-command.sh && /usr/bin/time -p bash /tmp/codex-bench-command.sh" 2>&1); then
    status=0
  else
    status=$?
  fi

  echo "$output"

  if [ "$status" -ne 0 ]; then
    return "$status"
  fi
}

extract_real_time() {
  awk '$1 == "real" { print $2 }' | tail -1
}

run_benchmark_step() {
  local scenario=$1
  local manager=$2
  local phase=$3
  local command=$4
  local note=${5:-}
  local output=""
  local elapsed=""

  log "Benchmark: $manager $phase $scenario"
  if ! output=$(measure_vm_command "$command"); then
    echo "$output"
    record_result "$scenario" "$manager" "$phase" "FAIL" "-" "${note:-command failed}"
    return 1
  fi
  echo "$output"

  elapsed=$(printf '%s\n' "$output" | extract_real_time)
  if [ -z "$elapsed" ]; then
    record_result "$scenario" "$manager" "$phase" "FAIL" "-" "${note:-missing real time}"
    return 1
  fi

  record_result "$scenario" "$manager" "$phase" "PASS" "${elapsed}s" "${note:-ok}"
  pass "$manager $phase $scenario: ${elapsed}s"
}

brewdock_preserve_and_reset_prefix() {
  vm_ssh '
    set -euo pipefail
    tmpdir=$(mktemp -d)
    preserve_dir() {
      src=$1
      dest=$2
      if [ -d "$src" ]; then
        mkdir -p "$(dirname "$dest")"
        cp -R "$src" "$dest"
      fi
    }

    preserve_dir /opt/homebrew/var/brewdock/cache "$tmpdir/cache"
    preserve_dir /opt/homebrew/var/brewdock/blobs "$tmpdir/blobs"
    preserve_dir /opt/homebrew/var/brewdock/store "$tmpdir/store"

    sudo rm -rf /opt/homebrew
    sudo mkdir -p /opt/homebrew
    sudo chown "$USER" /opt/homebrew
    mkdir -p /opt/homebrew/var/brewdock

    if [ -d "$tmpdir/cache" ]; then
      cp -R "$tmpdir/cache" /opt/homebrew/var/brewdock/cache
    fi
    if [ -d "$tmpdir/blobs" ]; then
      cp -R "$tmpdir/blobs" /opt/homebrew/var/brewdock/blobs
    fi
    if [ -d "$tmpdir/store" ]; then
      cp -R "$tmpdir/store" /opt/homebrew/var/brewdock/store
    fi

    rm -rf "$tmpdir"
  '
}

setup_vm() {
  log "Preflight checks"

  if ! command -v tart >/dev/null 2>&1; then
    fail "tart not found. Install: brew install cirruslabs/cli/tart"
  fi

  if ! command -v expect >/dev/null 2>&1; then
    fail "expect not found (should be pre-installed on macOS)"
  fi

  if [ ! -x "$BD_BINARY" ]; then
    fail "Release binary not found at $BD_BINARY. Run: cargo build --release -p brewdock-cli"
  fi

  log "Generating temporary SSH keypair"
  SHARE_DIR=$(mktemp -d)
  SSH_KEY="$SHARE_DIR/id_ed25519"
  ssh-keygen -t ed25519 -f "$SSH_KEY" -N "" -q
  cp "$BD_BINARY" "$SHARE_DIR/bd"
  chmod +x "$SHARE_DIR/bd"

  log "Cloning VM '$VM_NAME' from $BASE_IMAGE"
  tart clone "$BASE_IMAGE" "$VM_NAME"

  log "Starting VM (headless) with shared directory"
  tart run "$VM_NAME" --no-graphics --dir="brewdock:$SHARE_DIR" &

  log "Waiting for VM IP"
  for _ in $(seq 1 60); do
    VM_IP=$(tart ip "$VM_NAME" 2>/dev/null || true)
    if [ -n "$VM_IP" ]; then
      break
    fi
    sleep 2
  done
  if [ -z "$VM_IP" ]; then
    fail "could not obtain VM IP after 120s"
  fi
  pass "VM IP: $VM_IP"

  log "Waiting for SSH (password auth)"
  for _ in $(seq 1 30); do
    if ssh_with_password "true" >/dev/null 2>&1; then
      break
    fi
    sleep 2
  done

  log "Installing SSH public key into VM"
  MOUNT_PATH="/Volumes/My Shared Files/brewdock"
  for _ in $(seq 1 20); do
    if ssh_with_password "test -d '$MOUNT_PATH'" >/dev/null 2>&1; then
      break
    fi
    sleep 3
  done

  ssh_with_password \
    "mkdir -p ~/.ssh && chmod 700 ~/.ssh && cat '$MOUNT_PATH/id_ed25519.pub' >> ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys"

  if ! vm_ssh "true" >/dev/null 2>&1; then
    fail "SSH key auth failed after bootstrap"
  fi
  pass "SSH key auth established"

  log "Copying brewdock binary from shared mount"
  vm_ssh "cp '$MOUNT_PATH/bd' /tmp/bd && chmod +x /tmp/bd"
  pass "Binary ready at /tmp/bd"

  log "Preparing clean /opt/homebrew prefix"
  vm_ssh "sudo rm -rf /opt/homebrew && sudo mkdir -p /opt/homebrew && sudo chown \$USER /opt/homebrew"
  pass "Prefix ready"
}

install_zerobrew() {
  log "Installing zerobrew via installer"
  vm_ssh "curl -fsSL https://zerobrew.rs/install | bash -s -- --no-modify-path"
  vm_ssh "~/.local/bin/zb --version"
  pass "zerobrew installed"
}

install_nanobrew() {
  log "Installing nanobrew via installer"
  vm_ssh "curl -fsSL https://nanobrew.trilok.ai/install | bash"
  vm_ssh "/opt/nanobrew/prefix/bin/nb help >/dev/null"
  pass "nanobrew installed"
}

install_homebrew() {
  log "Installing Homebrew via official installer"
  vm_ssh "NONINTERACTIVE=1 /bin/bash -c \"\$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\""
  vm_ssh "/opt/homebrew/bin/brew --version | head -1"
  pass "Homebrew installed"
}

benchmark_brewdock_scenario() {
  local scenario=$1
  local packages_csv=$2
  local packages=""

  packages=$(scenario_packages_as_words "$packages_csv")

  if ! run_benchmark_step \
      "$scenario" "brewdock" "cold" \
      "/tmp/bd install $packages"; then
    brewdock_preserve_and_reset_prefix
    record_result "$scenario" "brewdock" "warm" "SKIP" "-" "cold run failed"
    return
  fi

  brewdock_preserve_and_reset_prefix
  if ! run_benchmark_step \
      "$scenario" "brewdock" "warm" \
      "/tmp/bd install $packages"; then
    brewdock_preserve_and_reset_prefix
    return
  fi

  brewdock_preserve_and_reset_prefix
}

benchmark_zerobrew_scenario() {
  local scenario=$1
  local packages_csv=$2
  local packages=""
  local zb="PATH=\"$HOME/.local/bin:$PATH\" ~/.local/bin/zb"

  packages=$(scenario_packages_as_words "$packages_csv")

  if ! run_benchmark_step \
      "$scenario" "zerobrew" "cold" \
      "$zb install $packages"; then
    remove_zerobrew_packages "$packages_csv"
    record_result "$scenario" "zerobrew" "warm" "SKIP" "-" "cold run failed"
    return
  fi

  remove_zerobrew_packages "$packages_csv"

  if ! run_benchmark_step \
      "$scenario" "zerobrew" "warm" \
      "$zb install $packages"; then
    remove_zerobrew_packages "$packages_csv"
    return
  fi

  remove_zerobrew_packages "$packages_csv"
}

benchmark_nanobrew_scenario() {
  local scenario=$1
  local packages_csv=$2
  local packages=""
  local nb="PATH=\"/opt/nanobrew/prefix/bin:$PATH\" /opt/nanobrew/prefix/bin/nb"

  packages=$(scenario_packages_as_words "$packages_csv")

  if ! run_benchmark_step \
      "$scenario" "nanobrew" "cold" \
      "$nb install $packages"; then
    remove_nanobrew_packages "$packages_csv"
    record_result "$scenario" "nanobrew" "warm" "SKIP" "-" "cold run failed"
    return
  fi

  remove_nanobrew_packages "$packages_csv"

  if ! run_benchmark_step \
      "$scenario" "nanobrew" "warm" \
      "$nb install $packages"; then
    remove_nanobrew_packages "$packages_csv"
    return
  fi

  remove_nanobrew_packages "$packages_csv"
}

benchmark_homebrew_scenario() {
  local scenario=$1
  local packages_csv=$2
  local packages=""
  local brew="/opt/homebrew/bin/brew"

  packages=$(scenario_packages_as_words "$packages_csv")

  remove_homebrew_packages "$packages_csv"
  run_benchmark_step \
    "$scenario" "homebrew" "cold" \
    "$brew install $packages"
  remove_homebrew_packages "$packages_csv"
}

run_manager_benchmarks() {
  local manager=$1
  local scenario=""
  local label=""
  local packages_csv=""

  case "$manager" in
    brewdock)
      log "Running brewdock benchmarks"
      for scenario in "${SCENARIOS[@]}"; do
        label=$(scenario_label "$scenario")
        packages_csv=$(scenario_packages "$scenario")
        benchmark_brewdock_scenario "$label" "$packages_csv"
      done
      ;;
    zerobrew)
      install_zerobrew
      for scenario in "${SCENARIOS[@]}"; do
        label=$(scenario_label "$scenario")
        packages_csv=$(scenario_packages "$scenario")
        benchmark_zerobrew_scenario "$label" "$packages_csv"
      done
      ;;
    nanobrew)
      install_nanobrew
      for scenario in "${SCENARIOS[@]}"; do
        label=$(scenario_label "$scenario")
        packages_csv=$(scenario_packages "$scenario")
        benchmark_nanobrew_scenario "$label" "$packages_csv"
      done
      ;;
    homebrew)
      vm_ssh "sudo rm -rf /opt/homebrew"
      install_homebrew
      for scenario in "${SCENARIOS[@]}"; do
        label=$(scenario_label "$scenario")
        packages_csv=$(scenario_packages "$scenario")
        benchmark_homebrew_scenario "$label" "$packages_csv"
      done
      ;;
    *)
      fail "unknown manager: $manager"
      ;;
  esac
}

result_for() {
  local scenario=$1
  local manager=$2
  local phase=$3
  local entry=""
  local payload=""
  local status=""
  local elapsed=""

  for entry in "${RESULTS[@]}"; do
    if [ "${entry%%:*}" != "$scenario" ]; then
      continue
    fi

    payload=${entry#*:}
    if [ "${payload%%:*}" != "$manager" ]; then
      continue
    fi

    payload=${payload#*:}
    if [ "${payload%%:*}" != "$phase" ]; then
      continue
    fi

    payload=${payload#*:}
    status=${payload%%:*}
    payload=${payload#*:}
    elapsed=${payload%%:*}

    if [ "$status" = "PASS" ]; then
      printf '%s' "$elapsed"
    elif [ "$status" = "SKIP" ]; then
      printf 'SKIP'
    else
      printf 'FAIL'
    fi
    return 0
  done

  printf '-'
}

print_summary() {
  local scenario=""
  local label=""
  echo ""
  log "Benchmark summary"
  printf '| Scenario | Homebrew | brewdock (cold) | brewdock (warm) | zerobrew (cold) | zerobrew (warm) | nanobrew (cold) | nanobrew (warm) |\n'
  printf '|---------|----------|------------------|------------------|------------------|------------------|------------------|------------------|\n'

  for scenario in "${SCENARIOS[@]}"; do
    label=$(scenario_label "$scenario")
    printf '| %s | %s | %s | %s | %s | %s | %s | %s |\n' \
      "$label" \
      "$(result_for "$label" homebrew cold)" \
      "$(result_for "$label" brewdock cold)" \
      "$(result_for "$label" brewdock warm)" \
      "$(result_for "$label" zerobrew cold)" \
      "$(result_for "$label" zerobrew warm)" \
      "$(result_for "$label" nanobrew cold)" \
      "$(result_for "$label" nanobrew warm)"
  done
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --keep)
      KEEP_VM=true
      shift
      ;;
    --formula)
      shift
      if [ "$#" -eq 0 ]; then
        fail "missing value for --formula"
      fi
      FORMULAE+=("$1")
      shift
      ;;
    --formula-set)
      shift
      if [ "$#" -eq 0 ]; then
        fail "missing value for --formula-set"
      fi
      FORMULA_SETS+=("$1")
      shift
      ;;
    --manager)
      shift
      if [ "$#" -eq 0 ]; then
        fail "missing value for --manager"
      fi
      case "$1" in
        homebrew|brewdock|zerobrew|nanobrew)
          REQUESTED_MANAGERS+=("$1")
          ;;
        *)
          fail "unknown manager: $1"
          ;;
      esac
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail "unknown argument: $1"
      ;;
  esac
done

if [ "${#FORMULAE[@]}" -eq 0 ] && [ "${#FORMULA_SETS[@]}" -eq 0 ]; then
  FORMULAE=("${DEFAULT_FORMULAE[@]}")
fi

if [ "${#FORMULA_SETS[@]}" -eq 0 ] && [ "${#FORMULAE[@]}" -eq 0 ]; then
  FORMULA_SETS=("${DEFAULT_FORMULA_SETS[@]}")
fi

for formula in "${FORMULAE[@]}"; do
  add_scenario "$formula" "$formula"
done

for formula_set in "${FORMULA_SETS[@]}"; do
  add_scenario "$(printf '%s' "$formula_set" | tr ',' ' ')" "$formula_set"
done

if [ "${#REQUESTED_MANAGERS[@]}" -eq 0 ]; then
  REQUESTED_MANAGERS=("${DEFAULT_MANAGERS[@]}")
fi

order_managers

setup_vm

for manager in "${MANAGERS[@]}"; do
  run_manager_benchmarks "$manager"
done

print_summary
