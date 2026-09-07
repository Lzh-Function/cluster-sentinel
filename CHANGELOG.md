# 変更履歴

本プロジェクトは milestone 単位で構築されています
（[docs/IMPLEMENTATION.md](docs/IMPLEMENTATION.md) §94）。

## 未リリース

### M7 — GPU

* `gpu.nvidia` probe（`nvidia-smi` の CSV 出力を parse）。
  `[N/A]` / `[Not Supported]` を parse 失敗ではなく「値なし」として扱う。
* Agent は probe が実際に観測した GPU 数を registration へ反映する。
  capability（GPU を持ち得る）と観測結果（GPU が 2 枚ある）を混同しない。
* GPU 数の不一致は degraded として報告し、node を broken とは呼ばない。
  Slurm に GRES 行が無い場合は「不一致」ではなく「未記載」として扱う。
* GPU の状態を変更する `nvidia-smi` オプションを使わないことをテストで強制。
* 実 GPU の検証は level 4（VM / 実クラスタ）。container では mock のみ。

### M6 — Storage / NFS

* NFS probe を危険度で分割:
  * `nfs.client.mount` — `/proc/self/mounts` のみ。hang 中も安全。
  * `nfs.client.io` — 実 I/O。**mount ごとに同時 1 本**（`SPEC.md` §76）。
  * `nfs.server.port` / `nfs.server.exports` — 到達性と export 一覧。
* `NFS_OK` / `NFS_SLOW` / `NFS_TIMEOUT` / `NFS_STUCK` の区別。
* Storage 診断 rule:
  * `NFS_SERVICE_FAILURE` — host は稼働、export service のみ異常。
  * `SHARED_STORAGE_FAILURE` — 同一 storage の複数 client が同時異常。
    group は dependency graph から**導出**し、宣言しない。
  * `NFS_CLIENT_FAILURE` — 1 client のみ異常、同一 storage の peer は正常。
  最後の 2 つは相互排他であることをテストで強制。
* Agent は NFS mount ごとに probe を個別スケジュール（同時実行枠も個別）。
* Slurm capability 検出をより保守的に変更。
  `slurm.conf` が読めない host は Slurm role を主張しない。
  バイナリの存在だけでは fileserver が compute node を名乗ってしまう。
* `[controller] observe` を追加。controller 自身が remote probe を行うかの制御。

### M5 — Slurm Diagnosis

* Diagnosis engine: typed Rust rule、決定的評価、rule の panic 隔離。
  DSL も LLM も使わない（`SPEC.md` §94）。
* `DiagnosisContext`: rule が参照できるのは保存済みの
  inventory / state / observation のみ。
  外部通信もコマンド実行も行わないため、診断は後から再現・検証できる。
* Slurm rule:
  * `SLURM_ONLY_DEGRADATION` — host は健全、Slurm 上のみ DRAIN。
  * `SLURMD_SERVICE_FAILURE` — host は応答するが slurmd が登録しない。
  * `SLURM_CONTROL_PLANE_FAILURE` — control plane 自体の異常。
  * `RESOURCE_CONFIGURATION_MISMATCH` / `GPU_CONFIGURATION_MISMATCH`。
* すべての rule が「host が健全である積極的証拠」を要求する。
  証拠が無ければ診断を出さない。
  これが無いと、死んだ host が「死んだ daemon」として報告される。
* `sentinel diagnose` を追加。診断・根拠・read-only な調査コマンドを表示。
* 疑似クラスタで検証: drain / slurmd 停止 / host 停止 が
  それぞれ異なる結果（3 つ目は「診断を出さない」）になること。

### M4 — Basic Host Monitoring

* Probe runner: timeout、panic 隔離、target ごとの同時実行数制限。
  probe の panic が daemon を落とさないことをテストで検証。
* Probe 実装:
  * `host.metrics` — `/proc` から load / memory / pressure / uptime / boot ID。
    負荷は degraded であって failed ではない（計算 node は負荷が仕事）。
  * `network.tcp` — 到達性。**refusal は到達可能の証拠**（ADR 0004）。
  * `ssh.service` — banner 確認のみ。認証も remote 実行も行わない。
  * `systemd.unit` — `systemctl show` による read-only な状態取得。
  * `sentinel.agent` — peer から agent の health endpoint を確認。
* Agent の health endpoint（`/v1/agent/health`）。read-only、GET のみ。
  agent 本体をロックせずに応答するため、probe が遅くても peer から見える。
* Agent 側 local probe scheduler（probe ごとの interval と jitter）。
* Controller が observer として remote probe を実行。observation には observer を記録。
* State component（host / network / ssh / agent / service）への mapping。
  未 mapping の probe が生まれないことをテストで強制。
* 疑似クラスタで検証: agent 停止と sshd 停止が、
  それぞれ独立した component 異常として観測されること。

### M3 — Docker Compose 疑似クラスタ

* 6 container（controller 1 / compute 3 / fileserver 2）の疑似クラスタ。
  **実 munge / slurmctld / slurmd** が動作する。
* 障害注入シナリオ 11 種:
  `stop-agent` / `stop-slurmd` / `stop-ssh` / `drain-node` / `stop-fileserver` /
  `degrade-storage` / `pause-host` / `stop-host` / `stop-controller` /
  `isolate` / `recover-all`。すべて冪等で復旧可能。
* `wait-healthy` は container の起動だけでなく、controller API・Slurm の node 登録・
  storage service・agent 登録がすべて揃うまで待つ。
* Docker integration test 11 件（`SENTINEL_DOCKER_TESTS=1` で opt-in）。
  Docker 非対応環境でも unit / simulation テストは通る。
* Slurm role の判定を設定ベースへ変更（ADR 0003）。
  バイナリの存在では compute node が control plane を主張してしまう。
* capability の取り下げを provider 単位で反映（`reconcile_capabilities`）。
* `docs/SECURITY.md`、`dev/compose/README.md` を追加。

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
