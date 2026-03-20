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

- [x] Theme: Bottle download + verify + extract
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

- [x] Theme: Cellar materialization + receipt + linking + state
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

- [x] Theme: Core orchestration + lock
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

- [x] Theme: CLI wiring
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

- [x] Theme: Polish + UX
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

- [ ] Theme: Compatible bottle selection + install method planning
  - Outcome: `install` / `upgrade` / `--dry-run` が exact host tag だけでなく互換 bottle tag も選べるようになり、formula ごとに `Bottle` か `Source` の install method を一貫して解決できる
  - Goal: bottle selector 導入、`SelectedBottle` と `InstallMethod` 追加、supportability 判定を method planning 前提へ切り替える
  - Must Not Break: `/opt/homebrew` 前提と `Layout` 経由の path 解決を維持する; 既存 bottle install 成功系を壊さない; `task check` green
  - Non-goals: `post_install` 実行; source build 実行; tap/cask 対応
  - Acceptance (EARS):
    - When the host tag has no exact bottle and an older compatible bottle or `all` bottle exists, the system shall select the highest-priority compatible bottle
    - When a formula has `post_install_defined=true`, the system shall remain plannable if an install method can still be resolved
    - If neither a compatible bottle nor a source build plan can be resolved, the system shall return unsupported
  - Evidence: `run=task test; oracle=selector exact/compatible/all tests, added formula JSON field deserialization tests, install/upgrade/dry-run method planning tests; visibility=independent; controls=[context]; missing=[]; companion=none; notes=CLI and core must show the same resolved method`
  - Gates: `static`, `integration`, `system`
  - Executable doc: `cargo test -p brewdock-formula -- selector`; `cargo test -p brewdock-formula -- supportability`; `cargo test -p brewdock-core -- install_method`; `cargo test -p brewdock-cli -- dry_run`
  - Why not split vertically further?: selector だけ先に入れても install/upgrade/dry-run が別経路のままだと user-visible contract が閉じない
  - Escalate if: Homebrew JSON API の追加 field が想定より不安定で source planning の contract を固定できない場合

- [ ] Theme: Restricted `post_install` execution without Ruby runtime
  - Outcome: 対応済み DSL だけを使う bottle formula は Ruby 実行環境なしで `post_install` を完了でき、失敗時は keg/receipt/state が残らない
  - Goal: homebrew-core Ruby source 取得、`def post_install ... end` 抽出、限定 DSL parser/executor、orchestrator への安全な実行タイミング統合
  - Must Not Break: unsupported syntax は fail-closed; receipt/state DB は成功時のみ更新; cleanup は失敗時に必須
  - Non-goals: 任意 Ruby 実行; control flow 対応; helper method 呼び出し; tap formula 対応
  - Acceptance (EARS):
    - When `post_install_defined` is false, the system shall skip hook execution
    - When a supported `post_install` block is present, the system shall execute it after `relocate_keg` and before `link`
    - If unsupported syntax or command failure occurs, the system shall cleanup the keg and shall not write receipt or state DB records
  - Evidence: `run=task test; oracle=post_install block extraction tests, supported DSL/path expression tests, success-path receipt/state persistence tests, failure-path cleanup tests; visibility=independent; controls=[context]; missing=[]; companion=none; notes=initial acceptance formulas are bat, curl, wget`
  - Gates: `static`, `integration`, `system`
  - Executable doc: `cargo test -p brewdock-cellar -- post_install`; `cargo test -p brewdock-core -- post_install`; `tests/vm-smoke-test.sh bat curl wget`
  - Why not split vertically further?: parser 単体では安全性の主契約である cleanup と receipt/state 境界を検証できない
  - Escalate if: 初期対象 formula の `post_install` が列挙済み DSL を超え、subset を広げないと acceptance formula を通せない場合

- [ ] Theme: Generic source fallback build driver
  - Outcome: 互換 bottle がない formula は generic build driver で source install を試行し、対応外 requirements は明示エラーで止まる
  - Goal: `BuildPlan` 導入、build dependency closure install、tarball/git source fetch、build root extraction、generic builder 実装、install method 自動 fallback
  - Must Not Break: source build に Ruby DSL 互換は持ち込まない; `uses_from_macos` は install 対象外; unsupported requirement は fail-closed
  - Non-goals: patch/resource/vendor virtualenv; Linux/Intel; tap/cask; Homebrew DSL 全面互換
  - Acceptance (EARS):
    - When no compatible bottle exists and source metadata is available, the system shall resolve `InstallMethod::Source` and install into the keg path
    - When build dependencies are declared, the system shall install their dependency closure before building the target formula
    - If the build system or requirements are unsupported, the system shall fail explicitly and cleanup without writing receipt or state DB records
  - Evidence: `run=task test; oracle=source plan selection tests, build success/failure cleanup tests, upgrade method reuse tests, representative source fallback VM smoke tests; visibility=independent; controls=[context]; missing=[]; companion=none; notes=initial acceptance formula is wakeonlan or sqlmap, final stop condition is 14 failing formulas green in VM smoke`
  - Gates: `static`, `integration`, `system`
  - Executable doc: `cargo test -p brewdock-core -- source_fallback`; `cargo test -p brewdock-core -- upgrade`; `tests/vm-smoke-test.sh wakeonlan`
  - Why not split vertically further?: planning と build execution を分けると fallback contract の本体である auto-switch と cleanup 条件が閉じない
  - Escalate if: generic driver で対象 formula を通せず、Ruby formula DSL 互換を導入しないと Goal を満たせない場合
