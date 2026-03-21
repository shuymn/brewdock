#!/usr/bin/env bash
#
# Capture brewdock pipeline baselines with tracing phase breakdowns inside a
# disposable Tart macOS VM. The report distinguishes wall/busy/idle time and
# explicit child-process spans so optimization work is not guided by
# `time.busy` alone.
#
# Usage:
#   ./tests/vm-pipeline-baseline.sh [--keep] [--runs <count>] [--output <path>]
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
RUNS=1
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
  ./tests/vm-pipeline-baseline.sh [--keep] [--runs <count>] [--output <path>]

Examples:
  ./tests/vm-pipeline-baseline.sh
  ./tests/vm-pipeline-baseline.sh --runs 3
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
  local run_index=$1
  local label=$2
  local elapsed=$3
  local log_file=$4

  RESULTS+=("${run_index}:${label}:${elapsed}:${log_file}")
}

run_scenario() {
  local run_index=$1
  local label=$2
  local command=$3
  local log_file="$SHARE_DIR/run-${run_index}-${label}.jsonl"
  local output=""
  local elapsed=""

  rm -f "$log_file"
  log "Running baseline scenario: run ${run_index}/${RUNS} - $label"
  if ! output=$(measure_vm_command "BREWDOCK_BENCHMARK_FILE='$MOUNT_PATH/run-${run_index}-${label}.jsonl' $command"); then
    echo "$output"
    fail "scenario failed: $label"
  fi
  echo "$output"

  elapsed=$(printf '%s\n' "$output" | extract_real_time)
  [ -n "$elapsed" ] || fail "missing wall-clock time for $label"
  [ -f "$log_file" ] || fail "missing tracing log for $label"
  record_result "$run_index" "$label" "${elapsed}s" "$log_file"
  pass "$label: ${elapsed}s"
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
require "time"

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

def find_span(event, span_name)
  span = event["span"]
  return span if span && span["name"] == span_name

  spans = event["spans"] || []
  spans.reverse.find { |entry| entry["name"] == span_name }
end

def span_field(span, key)
  return nil unless span

  if span.key?("fields")
    span.dig("fields", key)
  else
    span[key]
  end
end

def parse_timestamp_ms(value)
  return nil if value.to_s.strip.empty?

  Time.iso8601(value).to_r * 1000
rescue ArgumentError
  nil
end

def append_interval(intervals, event, duration_ms)
  finished_at_ms = parse_timestamp_ms(event["timestamp"])
  return if finished_at_ms.nil?

  intervals << [(finished_at_ms - duration_ms).to_f, finished_at_ms.to_f]
end

def merged_interval_ms(intervals)
  return 0.0 if intervals.empty?

  merged = intervals.sort_by(&:first)
  total = 0.0
  current_start, current_end = merged.first

  merged.drop(1).each do |start_ms, end_ms|
    if start_ms <= current_end
      current_end = [current_end, end_ms].max
    else
      total += current_end - current_start
      current_start = start_ms
      current_end = end_ms
    end
  end

  total + (current_end - current_start)
end

def median(values)
  sorted = values.sort
  count = sorted.length
  return 0.0 if count.zero?
  return sorted[count / 2] if count.odd?

  (sorted[(count / 2) - 1] + sorted[count / 2]) / 2.0
end

def summarize(values)
  {
    "median" => median(values),
    "mean" => values.sum(0.0) / values.length,
    "min" => values.min,
    "max" => values.max,
  }
end

scenario_runs = Hash.new { |hash, key| hash[key] = [] }
phase_runs = Hash.new do |hash, key|
  hash[key] = Hash.new do |phase_hash, phase_name|
    phase_hash[phase_name] = Hash.new { |metric_hash, metric| metric_hash[metric] = [] }
  end
end

while ARGV.any?
  _run_index = ARGV.shift
  label = ARGV.shift
  wall = ARGV.shift.delete_suffix("s").to_f
  path = ARGV.shift
  phases = Hash.new do |hash, key|
    hash[key] = {
      "wall_ms" => 0.0,
      "busy_ms" => 0.0,
      "idle_ms" => 0.0,
      "child_process_ms" => 0.0,
      "wall_intervals" => [],
      "child_intervals" => [],
    }
  end

  File.foreach(path) do |line|
    event = JSON.parse(line)
    fields = event["fields"] || {}
    next unless fields["message"] == "close"

    busy_ms = parse_duration_ms(fields["time.busy"])
    idle_ms = parse_duration_ms(fields["time.idle"])
    wall_ms = busy_ms + idle_ms

    phase_close_span = event["span"]

    if phase_close_span && phase_close_span["name"] == "bd.phase"
      phase_span = phase_close_span
      phase = span_field(phase_span, "phase")
      unless phase.nil? || phase.empty?
        phases[phase]["busy_ms"] += busy_ms
        phases[phase]["idle_ms"] += idle_ms
        append_interval(phases[phase]["wall_intervals"], event, wall_ms)
      end
    end

    next unless find_span(event, "bd.child_process")
    phase_span = find_span(event, "bd.phase")
    next unless phase_span

    phase = span_field(phase_span, "phase")
    next if phase.nil? || phase.empty?

    append_interval(phases[phase]["child_intervals"], event, wall_ms)
  rescue JSON::ParserError
    next
  end

  phases.each_value do |metrics|
    metrics["wall_ms"] = merged_interval_ms(metrics.delete("wall_intervals"))
    metrics["child_process_ms"] = merged_interval_ms(metrics.delete("child_intervals"))
  end

  scenario_runs[label] << wall
  phases.each do |phase_name, metrics|
    phase_runs[label][phase_name]["wall_ms"] << metrics["wall_ms"]
    phase_runs[label][phase_name]["busy_ms"] << metrics["busy_ms"]
    phase_runs[label][phase_name]["idle_ms"] << metrics["idle_ms"]
    phase_runs[label][phase_name]["child_process_ms"] << metrics["child_process_ms"]
  end
end

puts "# Pipeline Baseline"
puts
run_count = scenario_runs.values.first&.length || 0
puts "_Aggregated across #{run_count} run(s); scenario wall and phase timings use median, with mean/min/max shown for spread._"
puts
puts "| Scenario | Wall | Top Wall Phases | Top Child Phases |"
puts "|---|---:|---|---|"
scenario_runs.each do |label, wall_values|
  wall_summary = summarize(wall_values)
  phases =
    phase_runs[label]
      .map do |phase_name, metrics|
        [phase_name, metrics.transform_values { |values| summarize(values) }]
      end
      .sort_by { |(_, metrics)| -metrics["wall_ms"]["median"] }
  top_wall =
    phases.first(3).map do |name, metrics|
      format("%s %.1fms", name, metrics["wall_ms"]["median"])
    end.join(", ")
  top_child =
    phases
      .select { |(_, metrics)| metrics["child_process_ms"]["median"] > 0.0 }
      .first(3)
      .map do |name, metrics|
        format("%s %.1fms", name, metrics["child_process_ms"]["median"])
      end
      .join(", ")
  wall_text = format(
    "%.2fs (mean %.2fs, min %.2fs, max %.2fs)",
    wall_summary["median"],
    wall_summary["mean"],
    wall_summary["min"],
    wall_summary["max"],
  )
  puts "| #{label} | #{wall_text} | #{top_wall.empty? ? "-" : top_wall} | #{top_child.empty? ? "-" : top_child} |"
end
puts
puts "## Phase Breakdown"
puts
phase_runs.each do |label, phases|
  summarized_phases =
    phases
      .map do |phase_name, metrics|
        [phase_name, metrics.transform_values { |values| summarize(values) }]
      end
      .sort_by { |(_, metrics)| -metrics["wall_ms"]["median"] }
  puts "### #{label}"
  puts
  puts "| Phase | Wall Time | Busy Time | Idle Time | Child Process |"
  puts "|---|---|---|---|---|"
  summarized_phases.each do |name, metrics|
    puts format(
      "| %s | %.1fms (mean %.1f, min %.1f, max %.1f) | %.1fms (mean %.1f, min %.1f, max %.1f) | %.1fms (mean %.1f, min %.1f, max %.1f) | %.1fms (mean %.1f, min %.1f, max %.1f) |",
      name,
      metrics["wall_ms"]["median"],
      metrics["wall_ms"]["mean"],
      metrics["wall_ms"]["min"],
      metrics["wall_ms"]["max"],
      metrics["busy_ms"]["median"],
      metrics["busy_ms"]["mean"],
      metrics["busy_ms"]["min"],
      metrics["busy_ms"]["max"],
      metrics["idle_ms"]["median"],
      metrics["idle_ms"]["mean"],
      metrics["idle_ms"]["min"],
      metrics["idle_ms"]["max"],
      metrics["child_process_ms"]["median"],
      metrics["child_process_ms"]["mean"],
      metrics["child_process_ms"]["min"],
      metrics["child_process_ms"]["max"],
    )
  end
  puts
end
RUBY
)

  local args=()
  local entry=""
  local run_index=""
  local label=""
  local wall=""
  local path=""
  for entry in "${RESULTS[@]}"; do
    run_index=${entry%%:*}
    label=${entry#*:}
    label=${label%%:*}
    wall=${entry#*:*:}
    wall=${wall%%:*}
    path=${entry##*:}
    args+=("$run_index" "$label" "$wall" "$path")
  done

  ruby -e "$ruby_script" "${args[@]}"
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --keep)
      KEEP_VM=true
      shift
      ;;
    --runs)
      shift
      [ "$#" -gt 0 ] || fail "missing value for --runs"
      RUNS=$1
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

case "$RUNS" in
  ''|*[!0-9]*)
    fail "--runs must be a positive integer"
    ;;
esac

[ "$RUNS" -ge 1 ] || fail "--runs must be >= 1"

setup_vm

for run_index in $(seq 1 "$RUNS"); do
  prepare_clean_prefix
  run_scenario "$run_index" "update" "/tmp/bd update"

  prepare_clean_prefix
  run_scenario "$run_index" "install-tree" "/tmp/bd install tree"

  prepare_clean_prefix
  run_scenario "$run_index" "install-jq-wget" "/tmp/bd install jq wget"

  prepare_clean_prefix
  prepare_upgrade_fixture
  run_scenario "$run_index" "upgrade-dry-run-jq" "/tmp/bd upgrade --dry-run jq"
done

markdown=$(render_markdown)
printf '%s\n' "$markdown"

if [ -n "$OUTPUT_PATH" ]; then
  mkdir -p "$(dirname "$OUTPUT_PATH")"
  printf '%s\n' "$markdown" >"$OUTPUT_PATH"
  pass "Wrote markdown summary to $OUTPUT_PATH"
fi
