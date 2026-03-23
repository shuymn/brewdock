## Context

brewdock の `post_install` handling は Prism-backed の静的 AST 解析 (`brewdock-analysis`) で Ruby formula source を parse し、allowlisted な AST shapes を内部操作列 (`Program`) に lower して `brewdock-cellar` で実行する。

2026-03-23 時点で homebrew/core 8,292 formula 中 7,648 が ok、92 が `post_install_unsupported`。残り 92 formula の unsupported 理由を分析した結果、以下の 3 層に分類された:

1. **静的解析で決定可能** — AST パターン追加で対応可能（例: `bin.install`, `chmod`, `File.exist?("string")`）
2. **install 時に DSL を解釈すれば決定可能** — formula 属性（`name`, `version.major_minor`）、OS 情報（`OS.kernel_version.major`）、環境変数、ファイルシステム状態、プロセス実行（`Utils.safe_popen_read`）が揃えば Rust で評価可能
3. **任意の Ruby 実行が必要** — homebrew/core の `post_install` には現時点で該当なし。architecture.md が予約する escape hatch として保持

「静的解析不可能 = 対応不可能」ではなく、「静的解析フェーズでは決定不能だが install 時には全情報が揃う」という区別が重要。

Regex 互換性: homebrew/core の `post_install` で使われる正規表現は 4 パターンのみ（`/^LANG$|^LC_/`, `/(?<!bin|man)_dir$/`, `/\d+(?:\.\d+)*$/`, `%r{...}o`）で、全て Rust `regex` crate v1.12 で互換。Onigmo 固有機能の使用は確認されていない。

## Decision

三層戦略を採用する:

- **Tier 1 (default)**: 静的 AST 解析 — 現行の `brewdock-analysis` による lowering。ランタイム情報不要。`bd-analyze` によるオフライン分析もこの tier。
- **Tier 2**: install 時 DSL 解釈 — Tier 1 の lowering を拡張し、install 時に利用可能なランタイムコンテキスト（formula metadata, OS 情報, env, filesystem, process execution）を注入して Homebrew DSL プリミティブを Rust で評価する。
- **Tier 3 (reserved)**: Ruby 実行 escape hatch — architecture.md に記載の通り opt-in, last-resort, visible diagnostics 付きで保持。homebrew/core の `post_install` では不要。

Tier 2 は「Ruby を実行する」のではなく「Homebrew DSL の有限集合を Rust で実装する」こと。具体的に必要な DSL プリミティブ:

- 属性: `name`, `version`, `version.major_minor`, `OS.kernel_version.major`, `Hardware::CPU.arch`
- プロセス: `Utils.safe_popen_read` → `std::process::Command`
- ファイルシステム: `Pathname#find`, `relative_path_from`, `exist?`, `directory?`, `symlink?`, `children`
- 環境変数: `ENV[]`, `ENV.filter_map`
- 文字列: `split`, `start_with?`, `[/regex/]`, `strip`, `chomp`
- コレクション: `each`, `map`, `filter_map`, `select`, `sort`, `uniq`
- Regex: Rust `regex` crate で compile し fail-closed（`Regex::new` 失敗 → unsupported）

Tier 2 と Tier 3 の境界: ブロック本体の評価が「DSL プリミティブの組み合わせ」を超えて任意の Ruby 式になった時点で Tier 3 が必要。homebrew/core では現時点で該当なし。

実装上は `brewdock-cellar` の `post_install` ランタイムを entrypoint / execute / rollback に分割し、失敗時の巻き戻しと実行パスを別責務として保っている。これは Tier 1/Tier 2 の意味論を変えず、fail-closed の境界を見通しやすくするための内部構造である。

## Rejected Alternatives

- **ENV 条件を一律 false 扱い**: mariadb 等の `return if ENV["HOMEBREW_GITHUB_ACTIONS"]` を無条件スキップする案。CI 環境で実行した場合に誤動作する。正しくはランタイム `std::env::var` で評価する。
- **ruby+wasm を全ケースに使用**: ~30MB のバイナリサイズ増。`Utils.safe_popen_read` にはプロセス実行能力が必要で WASM sandbox の利点が薄れる。Rust DSL 実装で十分な範囲では過剰。
- **Ruby 実行を primary path にする**: fail-closed アーキテクチャと矛盾。Tier 3 として予約するに留める。

## Consequence

- homebrew/core の `post_install` カバレッジは Tier 1 + Tier 2 で理論上 100% に到達可能（Ruby 依存なし）
- `brewdock-analysis` は Tier 1 専用の純粋解析として維持。Tier 2 のランタイム評価は別の実行パスに置く
- Regex 互換性は `Regex::new` の fail-closed で担保。将来 Onigmo 固有機能が必要になれば `fancy-regex` or `onig` に切り替え
- `bd-analyze` はオフライン分析ツールとして引き続き Tier 1 のみで動作

## Revisit trigger

- homebrew/core の `post_install` が Tier 2 の DSL プリミティブ集合を超える Ruby 構文を使い始めた場合
- Regex パターンが Rust `regex` crate の対応範囲を超えた場合
- third-party tap 対応で DSL カバレッジが不足した場合
