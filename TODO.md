# brewdock TODO

Architecture decisions are fixed in [docs/architecture.md](docs/architecture.md).

## Open Questions

None.

## Theme Backlog

- [x] Theme: Workspace scaffold + Layout + platform + core types
  - Outcome: 5-crate workspace が compile; `bd --version` がバージョン表示; Layout パス算出が検証済み
  - Goal: single-crate → 5-crate workspace 移行; Layout・platform・error 基盤型定義
  - Must Not Break: `task check` green
  - Non-goals: formula/bottle/cellar の実装（空 stub のみ）; CI 設定変更
  - Acceptance (EARS):
    - When `cargo run -p brewdock-cli -- --version` is executed, the system shall print the package version and exit 0
    - When `cargo run -p brewdock-cli -- install foo` is executed, the system shall print a stub message and exit 0
    - When `task check` is executed, all gates shall pass
  - Evidence: `run=task check; oracle=all tests pass including Layout path assertions and HostTag parsing; visibility=independent; controls=[context]; missing=[]; companion=none`
  - Gates: `static`, `integration`
  - Executable doc: `cargo run -p brewdock-cli -- --version`; `cargo test -p brewdock-core -- layout`; `cargo test -p brewdock-core -- platform`
  - Why not split vertically further?: Layout・platform・error は相互参照する基盤型; 分割すると中間状態で compile 不可
  - Escalate if: workspace lint 設定が crate 間で衝突する場合

- [x] Theme: Formula types + API client
  - Outcome: Homebrew JSON API レスポンスを型付き parse; supportability check と dep resolve が動作
  - Goal: brewdock-formula crate 実装（types, API client, supportability, resolve）
  - Must Not Break: `task check` green; 既存 crate の public API 不変
  - Non-goals: API キャッシュ永続化; formula 検索; 実 API への E2E テスト
  - Acceptance (EARS):
    - When a valid Homebrew formula JSON is provided, the system shall deserialize it into typed structs
    - When a formula has `disabled=true` or `post_install_defined=true` or no bottle for host tag, the system shall reject it as unsupported
    - When dependencies form a DAG, the system shall return a topologically sorted install order
    - If dependencies contain a cycle, the system shall return CyclicDependency error
  - Evidence: `run=cargo test -p brewdock-formula --all-targets --all-features; oracle=fixture JSON deser, CellarType variants, supportability accept/reject, dep resolve DAG/diamond/cycle; visibility=independent; controls=[context]; missing=[]; companion=none`
  - Gates: `static`, `integration`
  - Executable doc: `cargo test -p brewdock-formula -- types`; `cargo test -p brewdock-formula -- supportability`; `cargo test -p brewdock-formula -- resolve`
  - Why not split vertically further?: types・API・supportability・resolve は install の最小 consumer 単位; types だけ分離すると実データ検証不可
  - Escalate if: Homebrew JSON API のスキーマが想定と大幅に異なる場合

- [ ] Theme: Bottle download + verify + extract
  - Outcome: bottle の DL → SHA256 verify → gzip+tar 展開 → CAS store が動作
  - Goal: brewdock-bottle crate 実装（download, verify, extract, store）
  - Must Not Break: `task check` green; brewdock-formula の public API 不変
  - Non-goals: resume download; parallel chunk download; blob dedup
  - Acceptance (EARS):
    - When a bottle is downloaded, the system shall stream-verify SHA256 during download
    - If SHA256 does not match, the system shall return ChecksumMismatch error
    - When a verified bottle is stored, the system shall extract tar.gz contents to the store directory
    - When a blob exists in store, `has()` shall return true
  - Evidence: `run=cargo test -p brewdock-bottle --all-targets --all-features; oracle=SHA256 match/mismatch, BlobStore lifecycle, fixture tarball extract + tree verification; visibility=independent; controls=[context]; missing=[]; companion=none`
  - Gates: `static`, `integration`
  - Executable doc: `cargo test -p brewdock-bottle -- verify`; `cargo test -p brewdock-bottle -- store`; `cargo test -p brewdock-bottle -- extract`
  - Why not split vertically further?: download → verify → extract → store は単一パイプライン; 各ステップが前段の出力に依存
  - Escalate if: Homebrew bottle の tar 構造が想定と異なる場合

- [ ] Theme: Cellar materialization + receipt + linking + state
  - Outcome: bottle 内容の Cellar 配置・receipt 書き込み・symlink 作成・SQLite 状態管理が動作
  - Goal: brewdock-cellar crate 実装（materialize, receipt, link, state）
  - Must Not Break: `task check` green; brewdock-formula の public API 不変
  - Non-goals: migration rollback; garbage collection; cellar audit
  - Acceptance (EARS):
    - When materialize is called, the system shall copy files to `Cellar/<name>/<version>/` and create `opt/<name>` symlink
    - When link is called for a non-keg_only formula, the system shall create relative symlinks from keg dirs to prefix dirs
    - If a link target already exists and points to a different keg, the system shall return LinkCollision error
    - When unlink is called, the system shall remove symlinks and empty parent directories
    - When a formula is installed, the system shall write INSTALL_RECEIPT.json compatible with Homebrew's format
    - When StateDb operations are called, the system shall correctly CRUD install records
  - Evidence: `run=cargo test -p brewdock-cellar --all-targets --all-features; oracle=materialize file presence, receipt JSON structure, symlink targets, collision detection, unlink cleanup, SQLite CRUD + idempotent migration; visibility=independent; controls=[context]; missing=[]; companion=none`
  - Gates: `static`, `integration`
  - Executable doc: `cargo test -p brewdock-cellar -- materialize`; `cargo test -p brewdock-cellar -- link`; `cargo test -p brewdock-cellar -- receipt`; `cargo test -p brewdock-cellar -- state`
  - Why not split vertically further?: materialize → receipt → link → state は install 完了の最小単位; receipt なしの materialize は Homebrew 互換性を壊す
  - Escalate if: Homebrew の INSTALL_RECEIPT.json 形式が想定と大幅に異なる場合

- [ ] Theme: Core orchestration + lock
  - Outcome: mock API + tempdir で install/upgrade フロー全体が動作
  - Goal: brewdock-core に install/upgrade orchestrator と file lock を実装; 全 crate 統合
  - Must Not Break: `task check` green; 下位 crate の public API 不変
  - Non-goals: retry logic; partial failure recovery; concurrent install
  - Acceptance (EARS):
    - When install is planned for a formula with dependencies, the system shall resolve, download, verify, extract, materialize, link in topological order
    - When a formula is already installed, the system shall skip it
    - If a formula is unsupported, the system shall return an error before download
    - When upgrade is called, the system shall unlink old → install new → link new → update state
    - When a lock is held by another process, the system shall block until released
  - Evidence: `run=cargo test -p brewdock-core --all-targets --all-features; oracle=mock repo full install flow, dep order, skip already-installed, unsupported error, upgrade flow, lock acquire/release; visibility=independent; controls=[context]; missing=[]; companion=none`
  - Gates: `static`, `integration`
  - Executable doc: `cargo test -p brewdock-core -- install`; `cargo test -p brewdock-core -- upgrade`; `cargo test -p brewdock-core -- lock`
  - Why not split vertically further?: install orchestrator は formula → bottle → cellar の統合点; lock なしの install は data corruption リスク
  - Escalate if: 下位 crate の API が orchestrator の想定と合わない場合

- [ ] Theme: CLI wiring
  - Outcome: `bd install <formula>` / `bd update` / `bd upgrade` が orchestrator 経由で動作
  - Goal: CLI コマンドを core の orchestrator に接続
  - Must Not Break: `task check` green; core の public API 不変
  - Non-goals: interactive confirmation; progress bar; shell completion
  - Acceptance (EARS):
    - When `bd install <formula>` is executed, the system shall run the install orchestrator and report results
    - When `bd update` is executed, the system shall fetch and cache the formula index
    - When `bd upgrade [<formula>...]` is executed, the system shall run the upgrade orchestrator
  - Evidence: `run=cargo test -p brewdock-cli --all-targets --all-features; oracle=CLI arg parsing, mock+tempdir install e2e; visibility=independent; controls=[context]; missing=[]; companion=none`
  - Gates: `static`, `integration`, `system`
  - Executable doc: `cargo test -p brewdock-cli -- commands`
  - Why not split vertically further?: install/update/upgrade は同一 CLI バイナリの 3 subcommand; 個別に出すと中途半端な UX
  - Escalate if: tokio runtime 構成が orchestrator の async 要件と衝突する場合

- [ ] Theme: Polish + UX
  - Outcome: --dry-run, --verbose/--quiet, ユーザー向けエラーメッセージが動作
  - Goal: エラー UX 改善、dry-run・verbose/quiet フラグ、cargo doc clean
  - Must Not Break: `task check` green; 既存コマンドの動作不変
  - Non-goals: README 書き直し; installer/uninstaller; shell completion
  - Acceptance (EARS):
    - When `--dry-run` is passed, the system shall display the plan without executing
    - When `--verbose` is passed, the system shall increase log detail
    - When `--quiet` is passed, the system shall suppress non-error output
    - When an error occurs, the system shall display a user-friendly message with hint
  - Evidence: `run=task check; oracle=dry-run plan output, verbose/quiet log levels, error display with hints; visibility=independent; controls=[context]; missing=[]; companion=none`
  - Gates: `static`, `integration`
  - Executable doc: `cargo test -p brewdock-cli -- dry_run`; `cargo test -p brewdock-cli -- verbose`
  - Why not split vertically further?: dry-run・verbose・quiet・error hint は同一 UX layer; 個別 PR にする価値が薄い
  - Escalate if: tracing-subscriber の構成が verbose/quiet の要件と合わない場合
