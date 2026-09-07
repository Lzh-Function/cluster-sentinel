# 変更履歴

本プロジェクトは milestone 単位で構築されています
（[docs/IMPLEMENTATION.md](docs/IMPLEMENTATION.md) §94）。

## 未リリース

### M2 — Agent + Protocol

* Wire protocol v1（versioned JSON over HTTP）。binary version と protocol version を分離。
  未知 field を許容し、controller 更新が fleet 全体を落とさない設計。
* 認証: cluster-scoped bearer credential。未認証動作は提供しない。
  credential はファイルまたは環境変数から読み、バイナリへ埋め込まない。
  比較は constant-time、`Debug` 出力は常に redact。
* Controller HTTP API: `/v1/health`（無認証）、`/v1/agents/register`、
  `/v1/agents/heartbeat`、`/v1/observations/batch`。
  **コマンドを実行する route は存在しない**（テストで検査）。
* Agent session 管理: boot ID の変化のみを reboot と判定し、
  agent プロセスの再起動と区別する。
* Runtime discovery: `SystemInspector` 抽象により、
  実機に無い構成（GPU node、fileserver 等）に対してもテスト可能。
  capability の **不在** も明示的に記録し、role hint を上書きできるようにする。
* Local spool: WAL、age / rows / bytes による上限、順序付き replay、
  idempotent 再送、重要な行を優先保持。送信前に書く（ADR 0002）。
* Agent daemon: 登録・heartbeat・spool flush。controller 不在時も動作を継続。
* CLI: `sentinel controller`、`sentinel agent`、`sentinel doctor`。
* `docs/SECURITY.md` を追加。

### M1 — Passive Controller + Slurm

* 共通 external command runner: timeout、出力サイズ上限（truncate 事実の記録）、
  allowlist、構造化エラー。probe が個別に `Command::new()` を書くことを禁止。
* Slurm integration: `scontrol show nodes -o` / `show partitions -o` / `ping` の
  parser、hostlist 展開、node state（base + flag + suffix）の解釈。
  未知 field・未知 state に対して寛容。
* Inventory: 複数 provider からの merge、discovery source 保存、
  消失時の stale 化（削除しない）、endpoint 未発見の edge の遅延解決。
* Slurm inventory provider（Host / Service / Scheduler entity と依存 edge を生成）と
  static config provider。
* State engine: probe → component の宣言的 mapping、debounce、severity cap。
  再起動時は保存済み state から再開。
* 永続化: entity / capability / label / dependency / observation / state /
  transition の read-write。observation ingestion は idempotent。
* Passive controller: discovery cycle 1 回で inventory・observation・state を更新。
  provider が失敗しても他 provider の結果を消さない。
* CLI: `sentinel status`、`sentinel discover`、`sentinel entity list|show`、
  `sentinel dependency list`。全て `--json` 対応。
* `fixtures/slurm/` に実出力形式の fixture を追加し、parser テストの入力とする。

### M0 — Repository / Core Domain

* Core domain model を追加: `Environment` / `ManagedEntity` / `Capability` /
  `DependencyEdge` / `ProbeDefinition` / `Observation` / `EntityState` /
  `StateTransition` / `Diagnosis` / `Incident`。
* Entity identity を natural key からの UUIDv5 導出に決定（ADR 0001）。
* Capability 解決の優先順位を実装（force-disable > force-enable >
  runtime discovery > role hint）。
* Dependency を cycle 許容の汎用有向グラフとして実装。全 traversal に visited set。
* Debounce / hysteresis（既定: warning 2、critical 3、recovery 2）。
* 設定の読み込み・優先順位追跡・検証。未知の `config_version` は拒否。
* SQLite core schema の初期 migration。controller 複数台を禁止しない設計。
* CLI skeleton: `sentinel version`、`sentinel config check`、`sentinel config show`。
* `src/` へ deployment 固有名が混入していないことを検査するテスト。
