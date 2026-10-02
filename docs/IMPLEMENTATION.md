# Cluster Sentinel v0.3
## 実装要件とテスト手順

本書は、データ型、通信形式、設定の優先順位、保存処理、probeの実行方法と障害時の処理を定める。
Docker Compose疑似クラスタを含むテスト方法、実装の順序、各段階の完了条件も記載する。
監視対象や機能の要件、設計原則は[SPEC.md](SPEC.md)を参照する。

本書と`SPEC.md`が競合する場合は、SPEC.mdのアーキテクチャ原則を優先する。

---

# 1. 配布するバイナリ

本番環境へ配布する成果物は、

```text
sentinel
```

の単一実行バイナリとする。

内部実装は、複数のモジュールやcrateへ分割してよい。各監視対象ホストに配布するSentinel実行ファイルは一つにする。

以下は同じバイナリのサブコマンドとして提供する。

```bash
sentinel controller
sentinel agent

sentinel status
sentinel entity ...
sentinel incident ...
sentinel dependency ...
sentinel peers

sentinel doctor
sentinel config ...
sentinel version
```

---

# 2. 開発環境

主な開発環境は、

```text
Windows
└── WSL2 Linux
    ├── Rust toolchain
    ├── Coding Agent
    ├── Cluster Sentinel repository
    └── Docker Engine / Docker Compose
```

を想定する。

実際のSlurm本番ノードを日常的な開発環境として使用することを前提にしない。

---

# 3. テスト環境の4層

テスト環境は以下の4層に分ける。

```text
Level 1
Unit / Mock Tests

Level 2
In-process Simulation

Level 3
Docker Compose Pseudo Cluster

Level 4
VM / Real Cluster Validation
```

各層で検証する内容を分ける。

---

# 4. Level 1単体・mockテスト

対象

```text
domain model
configuration
Slurm parser
dependency graph
state engine
diagnosis rules
incident correlation
peer assignment
database
protocol serialization
```

外部サービスを必要としない。

CI上で必ず実行可能であること。

---

# 5. Level 2プロセス内のシミュレーション

実際のネットワークやプロセスを使わず、人工的なProbe結果で検証する仕組みを実装する。

例

```text
Host:
HEALTHY

SSH:
FAILED

Sentinel:
HEALTHY

Slurm:
IDLE
```

等のObservationを投入し、State / Diagnosis / Incidentが期待通り生成されるか検証する。

短時間で実行でき、同じ入力に対して同じ結果を返すこと。

---

# 6. Level 3 Docker Compose疑似クラスタ

Docker Composeによる疑似クラスタを、開発に必要な成果物とし、統合テスト環境として実装する。

本番クラスタに影響を与えず、以下を検証する。

- 複数ホスト相当環境の再現
- Agent / Controller実通信
- Slurm実サービス
- SSH
- プロセスやサービスの停止
- ノード間の相互監視
- 経路遮断
- controller障害
- agent障害
- NFS論理障害
- 関連する診断のincidentへの統合

---

# 7. 疑似クラスタの構成

最低限以下を提供する。

```text
sentinel-controller

compute01
compute02
compute03

filesrv01
filesrv02
```

概念構成

```text
                      controller
                 Sentinel Controller
                      slurmctld
                          │
          ┌───────────────┼───────────────┐
          │               │               │
      compute01       compute02       compute03
      sentinel        sentinel        sentinel
       slurmd           slurmd          slurmd
          │               │               │
          └──────── peer monitoring ───────┘

             filesrv01        filesrv02
             sentinel         sentinel
```

必要に応じて規模を変更可能にする。

---

# 8. Docker用ファイルの配置

推奨

```text
/
├── src/
├── docs/
├── tests/
│
└── dev/
    └── compose/
        ├── compose.yaml
        │
        ├── images/
        │   ├── base/
        │   ├── controller/
        │   ├── compute/
        │   └── fileserver/
        │
        ├── config/
        │   ├── slurm.conf
        │   ├── sentinel/
        │   └── ssh/
        │
        ├── scripts/
        │   ├── up
        │   ├── down
        │   ├── reset
        │   └── wait-healthy
        │
        └── scenarios/
            ├── stop-agent
            ├── stop-slurmd
            ├── drain-node
            ├── isolate
            ├── stop-fileserver
            ├── pause-host
            └── recover-all
```

---

# 9. 疑似クラスタの起動要件

以下を満たす。

```text
docker compose up
```

または補助コマンド一つで疑似クラスタを起動できること。

起動後、可能な範囲で稼働確認を自動で行う。

開発者が各コンテナへ大量の初期設定を手作業で行わずに済むこと。

---

# 10. Docker内でのSlurm実行

Docker疑似クラスタでは、Slurmパーサーだけでなく可能な範囲で本物の、

```text
munge
slurmctld
slurmd
```

を起動する。

目的は、

```text
slurmd停止
↓
Slurm node state変化
↓
Sentinel observation
↓
Diagnosis
```

という観測から通知までの一連の挙動を検証すること。

Slurm CLIをmockするだけのテストとは別に扱う。

---

# 11. コンテナごとの役割

## Controller

少なくとも

```text
Sentinel controller
Slurm CLI
slurmctld
munge
```

を持つ。

---

## Compute

少なくとも

```text
Sentinel agent
slurmd
munge
SSH server
```

を持つ。

GPUは必須ではない。

---

## Fileserver

少なくとも

```text
Sentinel agent
SSH server
NFS-related test service or simulated storage service
```

を持つ。

WSLやDockerで実NFSを安全に扱えない場合は、サービスの停止や通信障害を再現する。カーネル内でNFS処理が応答しなくなる挙動はVMで検証する。

---

# 12. Dockerと本番環境の区別

本番環境への配置にDocker Composeを使わない。

本番環境では、

```text
systemd
native Linux host
single sentinel binary
```

を基本とする。

コンテナ固有の前提をSentinel coreへ持ち込んではならない。

---

# 13. Docker固有のコードの分離

以下のようなDocker固有知識をcoreへ埋め込んではならない。

```text
container IDs
Docker network names
Compose service names
Docker socket
Docker API
```

Dockerテスト環境は、Sentinelの実行結果を外部から確認するために使う。

---

# 14. 障害注入の仕組み

Dockerテスト環境には障害を繰り返し再現できる仕組みを用意する。

最低限

```text
agent failure
slurmd failure
Slurm DRAIN
host/container stop
host/container pause
controller failure
network isolation
one-direction or observer-specific network failure
fileserver service failure
```

をCLI/scriptから実行可能にする。

---

# 15. シナリオの実行コマンド

目標UX

```bash
./dev/compose/scenarios/stop-agent compute01

./dev/compose/scenarios/stop-slurmd compute01

./dev/compose/scenarios/drain-node compute01

./dev/compose/scenarios/isolate sentinel-controller compute01

./dev/compose/scenarios/stop-fileserver filesrv01

./dev/compose/scenarios/pause-host compute01

./dev/compose/scenarios/recover-all
```

具体的なファイル名は変更可能だが、同等の再現性を提供すること。

---

# 16. シナリオの期待結果の定義

可能ならシナリオの期待結果を機械可読形式にする。

例

```yaml
name: slurmd_failure

target: compute01

expected:
  host: HEALTHY
  ssh: HEALTHY
  slurmd: FAILED

diagnoses:
  required:
    - SLURMD_SERVICE_FAILURE

  forbidden:
    - HOST_UNREACHABLE
```

---

# 17. 経路遮断のシナリオ

経路障害の誤診断を防ぐため、重要な統合テストとする。

例

```text
controller → compute01 FAIL

compute02 → compute01 OK

filesrv01 → compute01 OK
```

期待

```text
PATH_SPECIFIC_NETWORK_FAILURE
```

またはcontroller側の通信異常。

以下を誤って生成してはならない。

```text
HOST_UNREACHABLE
```

---

# 18. ホスト全体の停止シナリオ

例

```bash
docker stop compute01
```

または同等障害注入。

複数observerから到達不能になることを確認する。

期待

```text
HOST_UNREACHABLE
```

ただし、

```text
POWER_OFF
```

とは判定しない。

---

# 19. ホストの一時停止シナリオ

```bash
docker pause compute01
```

等を利用して、プロセス実行スケジュールを含めた無応答状態を模擬する。

これは実際のカーネルのハードロックとは異なるが、

```text
network timeout
agent timeout
SSH timeout
peer failure
```

の複合状態テストとして使用する。

---

# 20. agentの停止シナリオ

Sentinel agentのみ停止。

期待

```text
network      HEALTHY
SSH          HEALTHY
Slurm        HEALTHY
Sentinel     FAILED
```

Diagnosis

```text
SENTINEL_AGENT_FAILURE
```

ホスト全体の障害と誤診断しない。

---

# 21. SSHの停止シナリオ

SSHサービスのみ停止。

期待

```text
Sentinel RPC HEALTHY
Host         HEALTHY
SSH          FAILED
```

Diagnosis

```text
SSH_SERVICE_FAILURE
```

---

# 22. slurmdの停止シナリオ

`slurmd`のみ停止。

期待

```text
Host       HEALTHY
Sentinel   HEALTHY
SSH        HEALTHY
slurmd     FAILED
```

Diagnosis

```text
SLURMD_SERVICE_FAILURE
```

---

# 23. Slurm DRAINのシナリオ

Slurm制御系からテスト用ノードをDRAINする。

期待

```text
Host       HEALTHY
Sentinel   HEALTHY
SSH        HEALTHY
slurmd     HEALTHY
Slurm      DRAIN
```

Diagnosis

```text
SLURM_ONLY_DEGRADATION
```

Classification

```text
SCHEDULER_DEGRADED
```

---

# 24. controllerの停止シナリオ

Sentinel controllerプロセスまたはcontrollerコンテナを停止する。

Agent側で、

```text
self monitoring
peer monitoring
local spool
```

が継続すること。

Controller復旧後、Observationが再送されること。

---

# 25. ファイルサーバーの停止シナリオ

filesrv01のstorage/NFSサービスを停止する。

ホストやコンテナ自体は稼働を続ける。

期待

```text
filesrv01 host HEALTHY
Sentinel       HEALTHY
SSH            HEALTHY
storage/NFS    FAILED
```

ホスト全体の障害とサービス障害を区別する。

---

# 26. 共有ストレージ障害のシナリオ

複数計算ノードが同じStorage Entityへ依存するテスト用の依存関係を作る。

例

```text
compute01
compute02
    → storage01 → filesrv01
```

filesrv01/storageサービス障害時に、

```text
compute01 storage degradation
compute02 storage degradation
filesrv01 service failure
```

を、

```text
SHARED_STORAGE_FAILURE
```

として相関できること。

---

# 27. クライアント単体のストレージ障害シナリオ

compute01のみストレージアクセスを遮断する。

filesrv01とcompute02は正常。

期待

```text
NFS_CLIENT_FAILURE
```

またはクライアント単体やストレージ経路の診断。

以下を生成してはならない。

```text
SHARED_STORAGE_FAILURE
```

---

# 28. Dockerで保証しないもの

以下についてDockerだけで実機同等性を保証してはならない。

```text
real machine reboot semantics
boot ID change
kernel hard lock
true D-state behavior
NFS hard-mount kernel stall
real systemd host behavior
SMART/NVMe failure
physical GPU failure
IPMI/BMC
NIC hardware failure
physical power loss
```

これらはVMまたは実クラスタでの検証へ委譲する。

---

# 29. Level 4 VM・実クラスタでの検証

Dockerでは十分に再現できない機能を検証する。

主対象

```text
systemd behavior
real reboot
boot ID
NFS hard mount
D-state
kernel/journal behavior
real GPU
NVMe/SMART
actual Slurm daemon interactions
```

---

# 30. VMのテスト環境

必要になった段階で小規模VM環境を使用する。

例

```text
ctrl01
compute01
filesrv01
```

程度でもよい。

VMテスト環境を日常的な主開発環境にすることは必須ではない。

---

# 31. 実クラスタでの検証

`example_cluster`は開発環境ではなく、

```text
staging / production validation environment
```

として扱う。

導入順

```text
read-only controller
↓
single agent
↓
multiple agents
↓
peer monitoring
↓
safe fault injection
↓
full deployment
```

---

# 32. 本番環境での障害注入

実クラスタで危険な障害シミュレーションを自動実行してはならない。

特に、

```text
NFS server shutdown
network-wide iptables changes
reboot
filesystem manipulation
```

等は管理者判断下のみ。

DockerやVMでのテストで代替できるものはそちらで行う。

---

# 33. 初期実装の方針

初期実装では過度な抽象化を避ける。

MVPでは以下を実装しない。

```text
dynamic shared-library plugin system
WASM plugin system
generic rule DSL
distributed consensus
controller HA
arbitrary remote command execution
```

ProbeやDiagnosisRuleはバイナリへコンパイルする。

---

# 34. 推奨するRustのライブラリ

基本候補

```text
tokio
axum
serde
serde_json
toml
clap
tracing
tracing-subscriber
sqlx
rustls
uuid
thiserror
```

原則

- 共通処理とライブラリ層では専用のエラー型
- CLIなどのアプリケーション側では`anyhow`等を利用してよい
- `unwrap()` / `expect()`は限定使用
- probe障害でデーモン全体をpanicさせない

---

# 35. リポジトリ構成

推奨

```text
/
├── Cargo.toml
├── README.md
├── CHANGELOG.md
│
├── docs/
│   ├── SPEC.md
│   ├── IMPLEMENTATION.md
│   ├── ARCHITECTURE.md
│   ├── CONFIGURATION.md
│   ├── OPERATIONS.md
│   ├── SECURITY.md
│   └── DEVELOPMENT.md
│
├── src/
│   ├── main.rs
│   ├── config/
│   ├── entity/
│   ├── capability/
│   ├── dependency/
│   ├── inventory/
│   ├── agent/
│   ├── controller/
│   ├── protocol/
│   ├── probes/
│   ├── observation/
│   ├── state/
│   ├── diagnosis/
│   ├── incident/
│   ├── correlation/
│   ├── notification/
│   ├── persistence/
│   ├── api/
│   └── cli/
│
├── migrations/
│
├── fixtures/
│   ├── slurm/
│   └── example_cluster/
│
├── tests/
│   ├── integration/
│   ├── simulation/
│   └── scenarios/
│
└── dev/
    └── compose/
```

---

# 36. 共通処理のデータ型

以下を明示的データ型として実装する。

```text
Environment
Cluster
ManagedEntity
Capability
DependencyEdge

ProbeDefinition
Observation

EntityState
StateTransition

Diagnosis
Incident

AgentIdentity
AgentSession
PeerAssignment
```

データモデルには専用の型を使い、`HashMap<String, Value>`の集合だけで構成しない。

---

# 37. ManagedEntityのフィールド

最低限

```text
id
environment_id
cluster_id optional
entity_type
canonical_name
display_name
labels
metadata
lifecycle_state
created_at
updated_at
```

Entity type

```text
host
service
storage
scheduler
external_dependency
```

---

# 38. Entityの識別方法

DB内部ではUUID等を使用する。

Discovery統合用識別キー

```text
environment
+
entity_type
+
canonical_name
```

IPアドレスをホストの識別キーとして使用しない。

Slurm NodeNameとホスト名も分離する。

---

# 39. Capabilityの表現

名前空間を持つ文字列とする。

例

```text
host.metrics
network.tcp
ssh.server
systemd

slurm.controller
slurm.compute

storage.local
storage.nfs.client
storage.nfs.server
storage.zfs
storage.smart

gpu.nvidia

journal.read
observer.peer
notification.fallback
```

---

# 40. Capabilityの優先順位

優先順位

```text
explicit force-disable
>
explicit force-enable
>
runtime discovery
>
role-derived hint
```

Roleのみを根拠にprobeを起動しない。

---

# 41. DependencyEdgeのフィールド

最低限

```text
id
source_entity_id
target_entity_id
dependency_type
criticality
metadata
discovery_source
first_seen_at
last_seen_at
```

方向

```text
A depends on B

A → B
```

依存グラフは閉路を許容する。

探索では訪問済みの対象を記録し、閉路による無限探索を防ぐ。

---

# 42. probeのインターフェース

概念

```rust
trait Probe {
    fn id(&self) -> ProbeId;
    fn required_capabilities(&self) -> ...;
    fn default_interval(&self) -> Duration;
    fn default_timeout(&self) -> Duration;

    async fn collect(&self, context: &ProbeContext) -> ProbeResult;
}
```

実際のRustインターフェースはobject safety等に合わせて調整可能。

---

# 43. probeの実行結果

最低限

```text
observation_id
probe_id
target_entity_id
observer_entity_id optional
agent_session_id

started_at
finished_at
duration

status
structured_payload
evidence
error_code optional
error_message optional
```

Status

```text
OK
DEGRADED
FAILED
TIMEOUT
STUCK
UNSUPPORTED
NOT_APPLICABLE
```

---

# 44. 保存済みの観測を変更しない

保存済みObservationを書き換えない。

後から変更可能なのは、

```text
derived state
diagnosis
incident correlation
```

のみ。

---

# 45. 観測の重複挿入を防ぐ

Observationには全体で一意のIDを持たせる。

Agent spool再送でも二重挿入しない。

controllerの取り込みは同じIDを重複挿入しない処理とする。

---

# 46. 時刻の扱い

保存時刻はUTC。

probeの所要時間とタイムアウトには、時刻変更の影響を受けない単調増加時計を使う。

incidentの履歴には実時刻を使う。

---

# 47. 時刻ずれ

agentとcontrollerの実時刻の差を監視する。

閾値超過

```text
CLOCK_SKEW
```

時刻ずれがあってもObservationを破棄しない。

---

# 48. probeの実行スケジュール

Probeごとに、

```text
interval
timeout
jitter
enabled
```

を管理する。

probeのスケジュールを、一つの共通タイマーループに直接書き込まない。

---

# 49. probeの失敗の隔離

1つのprobeが、

```text
panic
timeout
external command hang
filesystem stall
```

しても他probeを停止させない。

---

# 50. 外部コマンドの共通実行処理

外部コマンドの実行を共通化する。

必須

```text
timeout
stdout size limit
stderr size limit
exit status
execution duration
allowlisted command
structured error
```

各probeで`Command::new()`を直接使わず、共通の実行処理を使う。

---

# 51. コマンド出力の上限

以下の巨大出力対策を行う。

```text
journalctl
squeue
sacct
zpool
```

出力を切り詰めた場合は、その事実をObservationへ記録する。

---

# 52. journalの取得範囲

incidentの根拠となるjournalの取得範囲

```text
time range
unit
priority
line/byte limit
```

で制限する。

候補

```text
±5 minutes
500 lines
1 MiB
```

設定で変更可能。

---

# 53. NFS検査の安全制約

NFS mountごとに、

```text
max outstanding active filesystem probe = 1
```

とする。

前回のprobeがSTUCKなら、ファイルシステムにアクセスするprobeを新たに起動しない。

TCP/2049など、ファイルシステムへアクセスしない検査は継続できる。

---

# 54. agentのローカルspool

Controller到達不能時のObservationを、

```text
SQLite spool
```

へ保存する。

例

```text
/var/lib/sentinel/spool.db
```

要件

```text
WAL
bounded retention
idempotent resend
ordered replay
crash recovery
```

---

# 55. spoolの容量制限

spoolが無制限に増加しないように、保存量に上限を設ける。

```text
maximum age
maximum rows
maximum bytes
```

を設定で変更可能にする。

重大なイベントや状態遷移は、通常の測定値よりも保持優先度を高くする。

---

# 56. controllerのDB

MVP

```text
SQLite + WAL
```

例

```text
/var/lib/sentinel/sentinel.db
```

DBスキーマは、マイグレーションでのみ変更する。

---

# 57. 設定とDBのバージョン管理

Config

```toml
config_version = 1
```

DBマイグレーションバージョンも保持する。

未対応の設定バージョンは読み込みを拒否する。

---

# 58. 設定値の優先順位

```text
CLI
>
environment variable
>
config file
>
runtime discovery
>
built-in default
```

デバッグ時に、設定値をどこから読み込んだか確認できるようにする。

---

# 59. 既定パス

```text
/etc/sentinel/config.toml

/var/lib/sentinel/

/run/sentinel/
```

ログ出力先は、基本的にjournaldとする。

---

# 60. 設定の検証

```bash
sentinel config check
```

で、

```text
syntax
invalid durations
invalid endpoint
duplicate entity
bad dependency
security configuration
```

等を検証。

---

# 61. controllerとagentの通信形式

MVP

```text
versioned HTTPS API
+
JSON
```

protobuf/gRPCは将来追加可能。

---

# 62. プロトコルのバージョン

例

```text
/v1/...
```

バイナリのバージョンと通信プロトコルのバージョンを分ける。

---

# 63. 最低限必要なAPI

概念

```text
POST /v1/agents/register
POST /v1/agents/heartbeat
POST /v1/observations/batch

GET /v1/agents/{id}/assignments

GET /v1/health
GET /v1/peer/health
```

---

# 64. agentの登録情報

報告

```text
agent version
protocol version
environment
hostname
boot ID
addresses
capabilities
hardware summary
```

Controllerが監視対象一覧へ統合する。

---

# 65. 認証

認証なしの通信は禁止する。

MVPでは、

```text
TLS
+
cluster-scoped credential
```

でもよい。

将来、mTLSやノード別のcredentialへ移行できる設計とする。

Secretをバイナリへ埋め込まない。

---

# 66. 監視元の割り当て

同じ入力から同じ監視元を選び、依存関係も考慮して割り当てることを目標とする。

各対象につき可能なら、

```text
same dependency domain
different dependency domain
independent observer
```

から観測者を選ぶ。

監視元の数の既定値

```text
3
```

---

# 67. 監視元の選択制約

避ける

```text
self peer
duplicate observer
all observers in same failure domain
```

割り当てのリビジョンを持つ。

---

# 68. 状態エンジン

Probeは測定した事実のみ返す。

Probe内で、

```text
HOST_UNREACHABLE
SHARED_STORAGE_FAILURE
```

等を直接生成しない。

---

# 69. 状態の構成要素

最低限

```text
availability
network
host
agent
ssh
service
scheduler
storage
accelerator
clock
```

NOT_APPLICABLE対応必須。

---

# 70. 連続失敗・成功回数による状態判定

初期既定値

```text
warning = 2 failures
critical = 3 failures
recovery = 2 successes
```

Slurm DRAIN等の明示状態は即時Event化可能。

---

# 71. 診断ルール

MVPではRustで実装したルール。

汎用のルール記述言語は作らない。

最低限

```text
HOST_UNREACHABLE
PATH_SPECIFIC_NETWORK_FAILURE

SSH_SERVICE_FAILURE
SENTINEL_AGENT_FAILURE

SLURMD_SERVICE_FAILURE
SLURM_ONLY_DEGRADATION
SLURM_CONTROL_PLANE_FAILURE

RESOURCE_CONFIGURATION_MISMATCH

NFS_SERVICE_FAILURE
NFS_CLIENT_FAILURE
SHARED_STORAGE_FAILURE

GPU_CONFIGURATION_MISMATCH

CLOCK_SKEW
```

---

# 72. 診断の根拠

Diagnosisは、

```text
evidence observation IDs
affected entities
suspected root entities
rule ID
confidence
```

を保持する。

自然言語だけを保存しない。

---

# 73. 診断の確信度

```text
LOW
MEDIUM
HIGH
CONFIRMED
```

`CONFIRMED`は直接根拠となる観測がある場合のみ。

Peer到達性だけでは通常`HIGH`まで。

---

# 74. 複数の診断の統合

利用

```text
time proximity
shared dependency
same failed service
same scheduler
same network failure pattern
```

---

# 75. incidentの状態遷移

```text
OPEN
ACKNOWLEDGED
RECOVERING
RESOLVED
SUPPRESSED
```

原因となったサービスが復旧しても、依存するクライアントが未復旧ならRESOLVEDにしない。

---

# 76. maintenance中の処理

Maintenance中もObservationを収集する。

抑制するのは基本通知。

異常stateをHEALTHYへ書き換えない。

---

# 77. 通知

通知先ごとのインターフェースを使用。

MVP推奨

```text
generic webhook
```

または、

```text
ntfy
```

通知対象

```text
new incident
severity escalation
meaningful diagnosis change
recovery
resolution
```

Pollingごとの再通知は禁止。

---

# 78. Slurm出力の解析

未知のフィールドやフィールドの順序変更があっても、既知の値を読み取る。

実際のSlurm出力を、テストデータとして保存する。

最低限

```text
IDLE
ALLOCATED
MIXED
DRAIN
DOWN
NOT_RESPONDING
INVALID_REG
```

---

# 79. 初期導入先のテストデータ

現在の実構成は、

```text
fixtures/example_cluster/
```

へテストデータとして保存してよい。

ただしcoreから参照しない。

---

# 80. GPUの検査

MVPは`nvidia-smi`でよい。

想定するGPUなしなら、

```text
NOT_APPLICABLE
```

GPUがあるはずなのに検査コマンドやデバイスがない場合は、異常の候補とする。

---

# 81. ファイルサーバーの検査項目

ファイルサーバーの検査を、一つの`fileserver_health` probeにまとめない。

以下を分ける。

```text
host
service
TCP
export
backing filesystem
capacity
latency
```

---

# 82. ログ

`tracing`使用。

Context

```text
entity_id
agent_id
probe_id
incident_id
request_id
```

秘密情報や大量のコマンド出力をログへ記録しない。

---

# 83. Sentinel自身の状態確認

Sentinel自身について、

```text
uptime
version
spool size
queue depth
DB state
last controller connection
probe failures
```

を確認可能にする。

---

# 84. 終了処理

SIGTERM時

```text
stop new work
cancel/finish probes
flush important data
close DB safely
```

---

# 85. systemd

本番環境はsystemdで実行する。

可能な限り権限とアクセスの制限

```text
NoNewPrivileges
PrivateTmp
ProtectHome
ProtectSystem
ProtectKernelTunables
ProtectControlGroups
RestrictSUIDSGID
```

必要なパスだけに書き込みを許可する。

---

# 86. CIの検査

最低限

```bash
cargo fmt --check

cargo clippy \
  --all-targets \
  --all-features \
  -- \
  -D warnings

cargo test --all
```

---

# 87. CIでのDockerテスト

可能なCI環境では、

```text
docker compose pseudo-cluster smoke test
```

も実行する。

少なくとも、

```text
cluster startup
agent registration
Slurm discovery
basic peer communication
one or more failure scenarios
```

をCIの統合テスト用ジョブとして実行できるようにする。

Dockerを使えないCIでも単体・mockテストを実行できるよう、Dockerテストを分ける。

---

# 88. テストの種類

必須

```text
unit
mock
simulation
parser fixtures
DB migration
protocol
Docker Compose integration
scenario tests
```

追加

```text
VM
real cluster staged test
```

---

# 89. 代表的な状態と期待結果のテスト

代表状態を固定する。

例

```text
Host OK
SSH OK
Agent OK
slurmd OK
Slurm DRAIN
```

期待

```text
SCHEDULER_DEGRADED
SLURM_ONLY_DEGRADATION
```

---

# 90. CPU・メモリなどの使用量

Agent通常時目標

```text
RAM < 100 MiB desirable
idle CPU substantially below one core
minimal disk writes
minimal network traffic
```

---

# 91. Web UIの実装順序

Core完成後。

順序

```text
CLI
→ API
→ Web UI
```

Dockerテスト環境の存在を理由にUIを先行させない。

---

# 92. 必要なドキュメント

v1までに最低限

```text
README.md
docs/ARCHITECTURE.md
docs/CONFIGURATION.md
docs/OPERATIONS.md
docs/SECURITY.md
docs/DEVELOPMENT.md
```

`DEVELOPMENT.md`にはDocker Composeテスト環境の利用方法を必ず含める。

---

# 93. 開発ガイドに記載する内容

`docs/DEVELOPMENT.md`に最低限記載

```text
WSL/Linux prerequisites

Rust toolchain

Docker prerequisites

pseudo-cluster startup

pseudo-cluster shutdown

scenario execution

reset procedure

running unit tests

running Docker integration tests

known Docker limitations

when VM testing is required
```

---

# 94. 開発段階

## M0、Repository / Core Domain

```text
CLI skeleton
configuration
domain models
SQLite migrations
logging
```

Done

```bash
sentinel version
sentinel config check
```

---

## M1、Passive Controller + Slurm

```text
controller
Slurm discovery
Slurm parser
inventory
persistence
CLI status
```

Done

```bash
sentinel status
```

でテストデータまたは実Slurmからstate表示。

---

## M2、Agent + Protocol

```text
agent daemon
registration
heartbeat
local spool
runtime discovery
```

---

## M3、Docker Compose Pseudo Cluster

この開発段階を正式に追加する。

実装

```text
Compose topology
controller container
compute containers
fileserver containers
real Slurm services where practical
SSH
Sentinel agent/controller
health checks
reset scripts
```

Done

```bash
docker compose up
```

相当で疑似クラスタが再現可能。

Controllerへ複数Agentが登録され、

```bash
sentinel status
```

で認識できる。

---

## M4、Basic Host Monitoring

```text
Sentinel RPC
network
SSH
systemd/process
host metrics
```

Docker上でagent/SSH障害シナリオを通す。

---

## M5、Slurm Diagnosis

```text
slurmd
DRAIN
controller state
resource mismatch
```

Docker上の実Slurmテスト環境で、

```text
stop-slurmd
drain-node
```

シナリオを通す。

---

## M6、Storage / NFS

```text
Storage entities
DependencyGraph
NFS client
NFS server
safe active probing
```

DockerではサービスやネットワークのNFSシナリオを実装。

カーネル内のhard mountの挙動は、VMで検証する要件として記録する。

---

## M7、GPU

```text
NVIDIA discovery
GPU probe
Slurm GRES comparison
```

Dockerではmock中心。

実GPUは実クラスタでの検証。

---

## M8、Peer Monitoring

```text
assignment
remote observation
quorum
path failure detection
```

Dockerでネットワークを遮断するシナリオを必須とする。

---

## M9、Diagnosis / Incident Correlation

```text
state engine
diagnosis rules
shared dependency correlation
incident lifecycle
evidence preservation
```

Dockerの障害シナリオで、観測からincidentの生成、通知までを検証する。

---

## M10、Notification / Operations

```text
notification
deduplication
maintenance
installer
systemd
operations docs
```

---

## M11、VM / Production Validation

Dockerでは検証できない、

```text
systemd
boot ID
reboot
NFS hard mount
kernel events
real GPU
```

を段階的に検証する。

---

## M12、Web UI

共通処理の受け入れテスト通過後のみ。

---

# 95. 各開発段階の完了条件

各開発段階は、

```text
code
tests
documentation
migration if necessary
```

が揃うまでDoneではない。

Docker関連開発段階では、

```text
reproducible Compose environment
automated scenario
expected result
cleanup/reset procedure
```

まで含める。

---

# 96. v1の受け入れ条件

最低限以下を確認する。

### Slurm DRAIN

```text
Host healthy
SSH healthy
Agent healthy
slurmd healthy
Slurm DRAIN
```

を正しく分類。

### slurmd Failure

ホスト全体の障害と区別。

### SSH Failure

Agentは到達可能で、SSHだけが異常な状態を識別する。

### Agent Failure

SSH/Slurm正常でAgentだけ異常。

### Host Unreachable

複数observerから到達不能。

### Path Failure

一部observerのみ到達不能。

### NFS Server Failure

ファイルサーバーのホストとNFSサービスを区別。

### Shared Storage Failure

複数のクライアントの障害を、依存関係に基づいてincidentへまとめる。

### Client-local NFS Failure

ファイルサーバー障害と誤診断しない。

### Controller Failure

ローカルspoolへの保存とノード間の相互監視を継続する。

### Reboot

VMまたは実機でboot ID変更を検出。

### Topology Change

ホストやストレージを追加しても、共通処理の変更は不要とする。

---

# 97. Dockerでのv1受け入れ手順

以下を一連の自動テストとして実行可能であること。

```text
1. Pseudo cluster startup

2. All agents registered

3. Baseline HEALTHY

4. Stop Sentinel agent on compute01

5. SENTINEL_AGENT_FAILURE detected

6. Recover compute01

7. Stop slurmd

8. SLURMD_SERVICE_FAILURE detected

9. Recover slurmd

10. Slurm DRAIN

11. SLURM_ONLY_DEGRADATION detected

12. Isolate controller→compute01 only

13. PATH_SPECIFIC_NETWORK_FAILURE detected

14. Stop filesrv01 storage service

15. Shared-storage incident detected where topology applies

16. Recover all

17. Incidents resolve

18. Environment cleanup succeeds
```

---

# 98. 実装の優先順位

迷った場合

```text
correctness
>
failure isolation
>
diagnostic usefulness
>
security
>
operability
>
testability
>
performance
>
UI polish
```

---

# 99. MVPに含めない機能

MVPでは作らない

```text
plugin marketplace
custom rule language
custom TSDB
distributed DB
full Prometheus replacement
full Grafana replacement
automatic repair
LLM subsystem
Sentinel HA
```

---

# 100. 拡張用インターフェース

拡張点

```text
InventoryProvider
Probe
DiagnosisRule
NotificationProvider
PersistenceRepository
AuthenticationProvider
```

実装する機能に必要なインターフェースを設け、用途が決まっていないものを先に増やさない。

---

# 101. 実環境の設定値をコードに埋め込まない

以下は共通処理のコードへ埋め込まない。

```text
head01
filesrv01
filesrv02

node02-7
node01
node08
node09
node10
node11
node12
node13

current Slurm partitions
current NFS topology
current IP addresses
```

テストデータ、設定、自動検出で扱う。

---

# 102. 処理ごとの役割と検証方法

全実装を通して、

```text
Probe
↓
Observation
↓
State
↓
Diagnosis
↓
Incident
```

の役割を分けて実装する。

また、

```text
Slurm / NFS / NVIDIA / Docker test infrastructure
```

と、

```text
Sentinel core
```

を分離する。

Docker Composeは開発・テスト環境として維持する。
サービス停止や通信遮断を再現し、Sentinelの検知・診断を検証する。

日常の開発とCoding Agentによる自動検証は、WSLとDocker Composeを中心に行う。
Dockerで再現できないカーネル、systemd、ハードウェアの挙動は、VMと実クラスタで段階的に検証する。
