# ADR 0005 — Test Do Runtime v1

## Context

`homebrew/core` formula の `test do` は `brew install` の default path ではなく `brew test` で実行される。brewdock でも Homebrew 準拠を優先し、install/upgrade の default path に `test do` を混ぜない。

一方で `test do` の parse coverage だけでは「何が runtime で実行可能か」が見えない。top formula を見ると `assert_match`, `assert_equal`, `shell_output`, `system`, `testpath.write`, `chomp`, local variable assignment のような小さな subset でかなりの割合を占める。

## Decision

v1 として restricted `test do` runtime を導入する。

- `brewdock-analysis` は `test do` を feature census に加えて runtime IR (`TestProgram`) に lower する
- `brewdock-cellar` は `run_test_do` と `TestDoContext` を持ち、temporary `testpath` sandbox 内でだけ副作用を許可する
- v1 subset は以下に限定する
  - `assert_match`, `assert_equal`
  - `shell_output(command[, expected_status])`
  - `system`
  - `testpath/"..."` と `.write`
  - `bin/"..."`, `prefix/"..."`
  - `version.to_s`
  - local variable assignment / read
  - `.chomp`
- v1.1 で以下を追加:
  - bare `version` (= `version.to_s` と同等)
  - `assert_path_exists`, `refute_path_exists`
  - `refute_match`
  - `mkpath`, `touch`
  - `pipe_output(command, stdin[, expected_status])`
  - `.strip`, `.read`
  - `if OS.mac?` / `unless OS.mac?` 静的折り畳み (macOS 前提)
  - 追加 path bases: `include`, `lib`, `libexec`, `pkgshare`, `sbin`, `share`
- unsupported syntax は lower 時に fail-closed にする
- runtime error は command failure / assertion failure / path validation failure を分けて報告する

## Rejected Alternatives

- **`bd install` / `bd upgrade` に test 実行を混ぜる**: Homebrew の command semantics から外れる
- **Ruby を直接実行する**: native / fail-closed 方針に反する
- **parse coverage だけで止める**: runtime で何が終わっていて何が未対応かを追えない

## Consequence

- `test do` の runtime support は parse support と分離して追跡できる
- v1 は `testpath` sandbox 内に副作用を閉じ込められる
- v1.1 で top-100 の td_rt coverage が ~23% → ~39% に改善
- 次段階では `ENV` (read/write/compiler methods), `resource` blocks, `require`, `free_port`, `cp`/`cp_r`, `cd` を追加候補にできる

## Revisit Trigger

- `test do` を orchestration の product path に接続する時
- top formula の runtime unsupported が v1 subset 外に偏り始めた時
