#!/usr/bin/env bash
#
# Capture brewdock pipeline baselines with tracing phase breakdowns inside a
# disposable Tart macOS VM.
#
# Usage:
#   ./tests/vm-pipeline-baseline.sh [--keep] [--output <path>]
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

BASE_IMAGE="ghcr.io/cirruslabs/macos-sequoia-base:latest"
VM_NAME="brewdock-pipeline-baseline-$$"
SSH_USER="admin"
SSH_PASS="admin"
BD_BINARY="$PROJECT_ROOT/target/release/bd"
KEEP_VM=false
OUTPUT_PATH=""
SHARE_DIR=""
SSH_KEY=""
VM_IP=""
MOUNT_PATH="/Volumes/My Shared Files/brewdock"
RESULTS=()

log()  { printf '\033[1;34m==> %s\033[0m\n' "$*"; }
pass() { printf '\033[1;32m  PASS: %s\033[0m\n' "$*"; }
fail() { printf '\033[1;31m  FAIL: %s\033[0m\n' "$*"; exit 1; }

usage() {
  cat <<'EOF'
Usage:
  ./tests/vm-pipeline-baseline.sh [--keep] [--output <path>]

Examples:
  ./tests/vm-pipeline-baseline.sh
  ./tests/vm-pipeline-baseline.sh --output docs/pipeline-baseline.md
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

measure_vm_command() {
  local command=$1
  local encoded_command=""
  local output=""
  local status=0

  encoded_command=$(printf '%s' "$command" | base64)

  if output=$(vm_ssh "printf '%s' '$encoded_command' | base64 -d >/tmp/codex-pipeline-command.sh && chmod +x /tmp/codex-pipeline-command.sh && /usr/bin/time -p bash /tmp/codex-pipeline-command.sh" 2>&1); then
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

record_result() {
  local label=$1
  local elapsed=$2
  local log_file=$3

  RESULTS+=("${label}:${elapsed}:${log_file}")
}

prepare_clean_prefix() {
  log "Preparing clean /opt/homebrew prefix"
  vm_ssh "sudo rm -rf /opt/homebrew && sudo mkdir -p /opt/homebrew && sudo chown \$USER /opt/homebrew"
  pass "Prefix ready"
}

setup_vm() {
  log "Preflight checks"

  command -v tart >/dev/null 2>&1 || fail "tart not found. Install: brew install cirruslabs/cli/tart"
  command -v expect >/dev/null 2>&1 || fail "expect not found (should be pre-installed on macOS)"
  [ -x "$BD_BINARY" ] || fail "Release binary not found at $BD_BINARY. Run: cargo build --release -p brewdock-cli"

  SHARE_DIR=$(mktemp -d)
  SSH_KEY="$SHARE_DIR/id_ed25519"
  ssh-keygen -t ed25519 -f "$SSH_KEY" -N "" -q
  cp "$BD_BINARY" "$SHARE_DIR/bd"
  chmod +x "$SHARE_DIR/bd"

  log "Cloning VM '$VM_NAME' from $BASE_IMAGE"
  tart clone "$BASE_IMAGE" "$VM_NAME"

  log "Starting VM with shared directory"
  tart run "$VM_NAME" --no-graphics --dir="brewdock:$SHARE_DIR" &

  log "Waiting for VM IP"
  for _ in $(seq 1 60); do
    VM_IP=$(tart ip "$VM_NAME" 2>/dev/null || true)
    [ -n "$VM_IP" ] && break
    sleep 2
  done
  [ -n "$VM_IP" ] || fail "could not obtain VM IP after 120s"
  pass "VM IP: $VM_IP"

  log "Waiting for SSH (password auth)"
  for _ in $(seq 1 30); do
    if ssh_with_password "true" >/dev/null 2>&1; then
      break
    fi
    sleep 2
  done

  log "Installing SSH public key into VM"
  for _ in $(seq 1 20); do
    if ssh_with_password "test -d '$MOUNT_PATH'" >/dev/null 2>&1; then
      break
    fi
    sleep 3
  done
  ssh_with_password \
    "mkdir -p ~/.ssh && chmod 700 ~/.ssh && cat '$MOUNT_PATH/id_ed25519.pub' >> ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys"
  vm_ssh "true" >/dev/null 2>&1 || fail "SSH key auth failed after bootstrap"
  pass "SSH key auth established"

  log "Copying brewdock binary from shared mount"
  vm_ssh "cp '$MOUNT_PATH/bd' /tmp/bd && chmod +x /tmp/bd"
  pass "Binary ready at /tmp/bd"
}

run_scenario() {
  local label=$1
  local command=$2
  local log_file="$SHARE_DIR/${label}.jsonl"
  local output=""
  local elapsed=""

  rm -f "$log_file"
  log "Running baseline scenario: $label"
  if ! output=$(measure_vm_command "BREWDOCK_BENCHMARK_FILE='$MOUNT_PATH/${label}.jsonl' $command"); then
    echo "$output"
    fail "scenario failed: $label"
  fi
  echo "$output"

  elapsed=$(printf '%s\n' "$output" | extract_real_time)
  [ -n "$elapsed" ] || fail "missing wall-clock time for $label"
  [ -f "$log_file" ] || fail "missing tracing log for $label"
  record_result "$label" "${elapsed}s" "$log_file"
  pass "$label: ${elapsed}s"
}

prepare_upgrade_fixture() {
  log "Preparing upgrade dry-run fixture"
  vm_ssh "/tmp/bd install jq >/tmp/pipeline-upgrade-setup.log 2>&1"
  local real_version=""
  real_version=$(vm_ssh "ls /opt/homebrew/Cellar/jq | head -1")
  [ -n "$real_version" ] || fail "could not determine jq version for upgrade fixture"
  vm_ssh "mv /opt/homebrew/Cellar/jq/$real_version /opt/homebrew/Cellar/jq/0.0.0-fake && ln -sfn ../Cellar/jq/0.0.0-fake /opt/homebrew/opt/jq"
  pass "Upgrade fixture ready"
}

render_markdown() {
  local ruby_script=""
  ruby_script=$(cat <<'RUBY'
require "json"

def parse_duration_ms(value)
  text = value.to_s.strip
  return 0.0 if text.empty?

  number = text[/\A[0-9]+(?:\.[0-9]+)?/].to_f
  case text
  when /ns\z/ then number / 1_000_000.0
  when /µs\z/, /us\z/ then number / 1_000.0
  when /ms\z/ then number
  when /s\z/ then number * 1_000.0
  else
    number
  end
end

results = []
while ARGV.any?
  label = ARGV.shift
  wall = ARGV.shift
  path = ARGV.shift
  phases = Hash.new(0.0)

  File.foreach(path) do |line|
    event = JSON.parse(line)
    fields = event["fields"] || {}
    next unless fields["message"] == "close"

    span = event["span"]
    unless span && span["name"] == "bd.phase"
      spans = event["spans"] || []
      span = spans.reverse.find { |entry| entry["name"] == "bd.phase" }
    end
    next unless span

    phase =
      if span.key?("fields")
        span.dig("fields", "phase")
      else
        span["phase"]
      end
    next if phase.nil? || phase.empty?

    phases[phase] += parse_duration_ms(fields["time.busy"])
  rescue JSON::ParserError
    next
  end

  results << [label, wall, phases.sort_by { |(_, value)| -value }]
end

puts "# Pipeline Baseline"
puts
puts "| Scenario | Wall | Top Phases |"
puts "|---|---:|---|"
results.each do |label, wall, phases|
  top = phases.first(3).map { |name, value| format("%s %.1fms", name, value) }.join(", ")
  puts "| #{label} | #{wall} | #{top.empty? ? "-" : top} |"
end
puts
puts "## Phase Breakdown"
puts
results.each do |label, _wall, phases|
  puts "### #{label}"
  puts
  puts "| Phase | Busy Time |"
  puts "|---|---:|"
  phases.each do |name, value|
    puts format("| %s | %.1fms |", name, value)
  end
  puts
end
RUBY
)

  local args=()
  local entry=""
  local label=""
  local wall=""
  local path=""
  for entry in "${RESULTS[@]}"; do
    label=${entry%%:*}
    wall=${entry#*:}
    wall=${wall%%:*}
    path=${entry##*:}
    args+=("$label" "$wall" "$path")
  done

  ruby -e "$ruby_script" "${args[@]}"
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --keep)
      KEEP_VM=true
      shift
      ;;
    --output)
      shift
      [ "$#" -gt 0 ] || fail "missing value for --output"
      OUTPUT_PATH=$1
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

setup_vm

prepare_clean_prefix
run_scenario "update" "/tmp/bd update"

prepare_clean_prefix
run_scenario "install-tree" "/tmp/bd install tree"

prepare_clean_prefix
run_scenario "install-jq-wget" "/tmp/bd install jq wget"

prepare_clean_prefix
prepare_upgrade_fixture
run_scenario "upgrade-dry-run-jq" "/tmp/bd upgrade --dry-run jq"

markdown=$(render_markdown)
printf '%s\n' "$markdown"

if [ -n "$OUTPUT_PATH" ]; then
  mkdir -p "$(dirname "$OUTPUT_PATH")"
  printf '%s\n' "$markdown" >"$OUTPUT_PATH"
  pass "Wrote markdown summary to $OUTPUT_PATH"
fi
