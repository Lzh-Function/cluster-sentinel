# Cluster Sentinel
## クラスタの監視と障害診断の仕様

仕様のバージョンは0.3である。最初の導入対象は`example_cluster`と関連インフラとする。
Rustで実装し、CPUアーキテクチャごとに単一バイナリを配布する。
初期の対象環境はLinux、systemd、Slurm、NFSとする。

計算ノード、ストレージ、ネットワーク、スケジューラの構成が変わっても、
設定や連携機能、probeの追加で対応し、共通処理の変更を不要にすることを目標とする。
現在の設定方法と動作は[設定リファレンス](CONFIGURATION.md)と[運用ガイド](OPERATIONS.md)を参照する。

---

# 1. 目的

Cluster Sentinelは、研究用計算クラスタを構成する各種インフラの状態を複数地点から観測し、

```text
異常検知
    ↓
障害レイヤー切り分け
    ↓
依存関係解析
    ↓
インシデント相関
    ↓
証拠保存
    ↓
管理者通知
```

までを行う分散監視・診断システムである。

主な対象障害

- 計算ノードが停止した
- OSは起動しているがSSH不能
- SSHは可能だが`slurmd`が停止
- Hostは正常だがSlurm上のみ`DRAIN`
- Slurm controllerのみ異常
- NFSサーバーが停止
- NFSは応答するがI/Oが極端に遅い
- 共有ストレージ障害で複数計算ノードが連鎖的に異常化
- GPU障害
- ローカルストレージ・NVMeの異常
- 通信経路固有障害
- 監視controller自身の障害
- 再起動後に原因情報が失われる

---

# 2. v0.3での監視モデルの変更

v0.2まではHostを中心概念としていた。

v0.3では、Hostを含むより一般的な、

```text
ManagedEntity
```

を共通のデータモデルとする。

これにより、

```text
Host
Service
Storage
Scheduler
Network Endpoint
External Dependency
```

等を同一監視モデルへ統合する。

---

# 3. 技術固有の処理と共通処理の分離

共通処理では技術ごとの監視を実装せず、以下のデータ型を扱う。

```text
Entity
Capability
Dependency
Probe
Observation
State
Diagnosis
Incident
```

Slurm、NFS、GPU等はすべて連携機能として実装する。

---

# 4. 全体のデータモデル

コードでは、監視対象をEntity、機能情報をCapability、検査をProbe、
観測記録をObservation、状態をState、診断をDiagnosis、障害の管理記録をIncidentと呼ぶ。
以下の型名と図では、この名称を使う。

```text
Environment
│
├── Cluster
│
├── ManagedEntity
│   ├── Host
│   ├── Service
│   ├── Storage
│   ├── Scheduler
│   ├── NetworkEndpoint
│   └── ExternalDependency
│
├── Capability
├── DependencyGraph
├── Probe
├── Observation
├── Diagnosis
└── Incident
```

---

# 5. Environment（環境）

最上位概念を、

```text
Environment
```

とする。

現在は1環境のみでもよい。

例

```text
example-lab
```

将来的に、

```text
example-lab
├── production-cluster
├── test-cluster
└── workstation-group
```

のような構成も扱える。

---

# 6. Cluster（クラスタ）

ClusterはManagedEntityの集合を論理的にグループ化する。

現在

```text
Cluster:
example_cluster
```

ただしSentinelのClusterとSlurmのクラスタを同じ概念として扱わない。

将来的に、

```text
Slurm cluster
storage cluster
monitoring domain
```

が異なる場合にも対応する。

---

# 7. ManagedEntity（監視対象）

Sentinelが状態を持つ監視対象。

基本属性

```text
entity_id
entity_type
display_name
environment_id
cluster_id optional
labels
metadata
discovery_sources
```

---

# 8. 監視対象の種類

初期実装で定義する。

```text
Host
Service
Storage
Scheduler
ExternalDependency
```

NetworkDevice等は将来追加可能。

---

# 9. ホスト

物理または仮想Linuxホスト。

例

```text
head01
node01
node08
node09
node02
...
filesrv01
filesrv02
```

Hostは、

```text
IP
hostname
CPU
RAM
GPU
```

等を属性として持つ。

---

# 10. サービス

Host上で動くサービスを独立entityとして扱う。

例

```text
slurmctld@head01
slurmdbd@head01
slurmd@node09
sshd@node09
nfs-server@filesrv01
```

これにより、

```text
Host healthy
Service failed
```

を自然に表現できる。

---

# 11. ストレージ

StorageをHostとは独立して表現可能にする。

現在

```text
filesrv01-backed-storage
filesrv02-backed-storage
```

将来

```text
Ceph cluster
Lustre filesystem
BeeGFS filesystem
NFS VIP
ZFS pool
```

等にも対応可能。

---

# 12. スケジューラ

Scheduler/control planeも独立entityとする。

現在

```text
Slurm scheduler: example_cluster
```

Host `head01`とSchedulerそのものを区別する。

例

```text
Host(head01)
    ↓ hosts
Service(slurmctld)
    ↓ provides
Scheduler(example_cluster)
```

---

# 13. 外部の依存先

Agentを直接インストールできない対象もdependencyとして表現する。

例

```text
DNS server
gateway
switch
UPS
external authentication server
BMC endpoint
```

---

# 14. 役割ラベル

ホストなどに、運用者が読める役割ラベルを設定できる。

例

```text
controller
compute
fileserver
observer
login
storage
```

ただし役割はロジック制御に使用してはならない。

---

# 15. 役割ラベルの用途

禁止

```text
if role == "fileserver":
    run_nfs_probe()
```

推奨

```text
if capability == "nfs_server":
    enable_nfs_server_probe()
```

Roleは、

```text
UI grouping
operator understanding
default configuration hints
```

にのみ使用する。

---

# 16. Capability（機能情報）

各Entityは0個以上のcapabilityを持つ。

例

```text
host.metrics
network.icmp
network.tcp
ssh.server
systemd
slurm.controller
slurm.compute
gpu.nvidia
storage.local
storage.nfs.client
storage.nfs.server
storage.zfs
storage.smart
journal
observer.peer
```

---

# 17. 機能の組み合わせ

例えば`filesrv01`

```text
Host(filesrv01)

Capabilities:
host.metrics
network.tcp
ssh.server
systemd
storage.local
storage.nfs.server
storage.zfs
journal
observer.peer
```

`node09`

```text
Host(node09)

Capabilities:
host.metrics
network.tcp
ssh.server
systemd
slurm.compute
gpu.nvidia
storage.nfs.client
journal
observer.peer
```

---

# 18. 機能の自動検出

Agentは実行時にcapabilityの候補を検出する。

例

```text
slurmd exists
→ slurm.compute

slurmctld exists
→ slurm.controller

nvidia-smi usable
→ gpu.nvidia

NFS mount exists
→ storage.nfs.client

nfs-server exists
→ storage.nfs.server

zpool available and pool exists
→ storage.zfs
```

---

# 19. 検出結果の上書き

自動検出結果は管理者設定で、

```text
enable
disable
force
```

できる。

例

```toml
[capabilities]
"storage.nfs.server" = "force"
"storage.smart" = "disable"
```

---

# 20. 初期導入先の環境

初期環境

```text
example-lab
```

初期クラスタ

```text
example_cluster
```

---

# 21. 初期監視対象のホスト

Slurm controller

```text
head01
```

Slurm compute

```text
node02
node03
node04
node05
node06
node07

node01

node08
node09

node10
node11

node12
node13
```

Slurm外のインフラ

```text
filesrv01
filesrv02
```

初期監視対象は計16ホスト。

---

# 22. 初期導入先のSlurm設定

```text
SlurmctldHost=head01

SlurmctldPort=6817
SlurmdPort=6818

ReturnToService=1

AccountingStorageHost=head01

SelectType=select/cons_tres
SelectTypeParameters=CR_Core_Memory

GresTypes=gpu
```

SentinelはSlurmの設定を変更しない。

---

# 23. 初期導入先のストレージ共有範囲

filesrv01

```text
node02
node03
node04
node13
node11
```

filesrv02

```text
node05
node06
node07
node12
node10
```

この関係は導入先の設定値として扱い、共通処理の構成には埋め込まない。

---

# 24. 依存グラフ

すべての依存関係を汎用有向グラフとして表現する。

Edge

```text
source_entity
target_entity
dependency_type
criticality
metadata
discovery_source
```

---

# 25. 依存関係の方向

原則として、

```text
A depends_on B
```

を、

```text
A → B
```

として扱う。

例

```text
node02
    → filesrv01-backed-storage
```

---

# 26. 依存関係の種類

初期

```text
depends_on
hosted_on
provides
uses_storage
uses_scheduler
network_reaches
observes
```

連携技術ごとのメタデータを付与可能。

---

# 27. 複数段階の依存関係

DependencyGraphは、NFS以外の依存関係や複数段階の依存も扱う有向グラフとする。

将来

```text
compute01
   ↓
CephFS
   ↓
storage-network
   ↓
switch01
```

や、

```text
compute01
   ↓
Slurm Scheduler
   ↓
slurmctld VIP
   ↓
controller01
controller02
```

を表現可能とする。

---

# 28. 閉路の扱い

実際の依存関係に閉路がある場合も、汎用の有向グラフとして保存する。

原因を探索するときは訪問済みの対象を記録し、閉路による無限再帰を防ぐ。

---

# 29. 依存先に基づくグループ分け

「filesrv01-side」等のグループをcoreに固定定義しない。

DependencyGraphから、

```text
same upstream storage dependency
```

を共有するentityの集合として動的に求める。

---

# 30. 監視対象一覧の作成

監視対象一覧は、複数の検出元の情報から生成する。

```text
Inventory Provider
├── Slurm
├── Agent Registration
├── Static Configuration
└── Future Providers
```

---

# 31. Slurmからの監視対象の検出

以下から計算ノード等を発見。

```bash
scontrol show nodes -o
scontrol show partitions -o
```

Slurm上から消えたホストを即Sentinel監視対象一覧から削除してはならない。

---

# 32. agentの自己登録

Sentinel agentがcontrollerへ自己登録する。

これにより、

```text
filesrv01
filesrv02
login nodes
other hosts
```

をSlurmとは独立して発見できる。

---

# 33. 設定ファイルからの監視対象の登録

Agentをまだ配布していないホストも設定で登録可能。

```toml
[[entities.host]]
name = "filesrv01"

[[entities.host]]
name = "filesrv02"
```

---

# 34. 複数の検出結果の統合

同一entityが複数providerから発見された場合、

```text
Slurm
Agent
Static config
```

を統合する。

検出元の情報も保存する。

---

# 35. 監視対象の削除

Entityが一時的に自動検出されなくなっても自動削除しない。

状態

```text
ACTIVE
STALE
REMOVED
```

等を持つ。

インフラ変更による一時的不整合と障害を区別する。

---

# 36. ホストの識別キー

ホストの識別キー

```text
environment + stable host ID
```

を使用する。

通常ホスト名を初期の識別キーとして利用する。

---

# 37. IPアドレスと識別キーの分離

IPをホスト主キーにしてはならない。

1ホストは、

```text
IPv4
IPv6
management network
storage network
InfiniBand
```

等、複数アドレスを持てる。

---

# 38. Slurm NodeNameとホスト名の分離

将来、

```text
Slurm NodeName != hostname
```

でも動作すること。

Mapping

```text
Host Entity
    ↕
Slurm Node Identity
```

として別途保持する。

---

# 39. agent起動時の自動検出

Agentが取得

```text
hostname
FQDN
boot ID
IP addresses
interfaces
OS
kernel
CPU
RAM
GPU
mounts
services
storage
Slurm
NFS
ZFS
```

---

# 40. 単一バイナリ

原則

```text
sentinel
```

1 executable。

Subcommands

```bash
sentinel controller
sentinel agent
sentinel status
sentinel entity
sentinel incident
sentinel doctor
sentinel config
```

---

# 41. CPUアーキテクチャ別のバイナリ

CPUアーキテクチャが異なる場合のみ、

```text
sentinel-linux-x86_64
sentinel-linux-aarch64
```

等を生成する。

マシン固有ビルドは禁止。

---

# 42. controllerの配置先

Controllerを`head01`にハードコードしてはならない。

現在

```text
head01
```

は導入先の設定上のcontrollerに過ぎない。

---

# 43. 複数controllerへの将来対応

v1でHA controllerを実装する必要はない。

ただしデータモデル上、

```text
1 environment = exactly 1 controller
```

という制約を置いてはならない。

将来的な、

```text
primary controller
standby controller
distributed collectors
```

を許容する。

---

# 44. agentの役割

全Linuxホストへ同じagentバイナリを配布可能とする。

Agentが行う処理

```text
self observation
capability discovery
probe execution
peer observation
local buffering
event collection
controller communication
```

---

# 45. 他のホストを監視する機能

`observer.peer` capabilityを持つホストは他entityへのremote probeを実施できる。

現在推奨

```text
head01
filesrv01
filesrv02
compute nodes
```

---

# 46. ノード間の相互監視

単一中央監視だけに依存しない。

各重要ホストを複数observerから観測する。

---

# 47. 監視元の割り当て

Controllerが監視元を動的に割り当てる。

既定値の候補

```text
peer degree = 3
```

---

# 48. 依存関係を考慮した監視元の選択

可能なら、

```text
same dependency domain
different dependency domain
independent infrastructure observer
```

からそれぞれ観測点を選ぶ。

---

# 49. 監視元の割り当て例

node02

```text
node03   → same filesrv01 domain
node05   → different storage domain
filesrv02  → infrastructure observer
```

---

# 50. 複数の観測結果による判定

一つのprobeの失敗だけで、ホスト全体の障害と確定しない。

例

```text
head01 → node09 FAIL
node05 → node09 OK
filesrv01 → node09 OK
```

なら、

```text
PATH_SPECIFIC_NETWORK_FAILURE
```

候補。

---

# 51. ホストへの到達不能

独立observer複数から失敗

```text
head01 → node09 FAIL
node05 → node09 FAIL
filesrv01 → node09 FAIL
```

なら、

```text
HOST_UNREACHABLE
```

confidence HIGH。

---

# 52. 電源状態の判定

通常ネットワークprobeから、

```text
POWER_OFF
```

を断定してはならない。

確定可能なのは、

```text
HOST_UNREACHABLE
```

まで。

---

# 53. 別系統の管理機能との将来連携

将来

```text
IPMI
Redfish
BMC
smart PDU
UPS
```

を連携機能追加した場合、

```text
POWER_OFF_CONFIRMED
POWER_ON_BUT_HOST_UNRESPONSIVE
```

等を判別可能にする。

---

# 54. probeの構成

Probeの実装を共通処理から分離する。

概念インターフェース

```text
Probe
├── probe_id
├── required_capability
├── target_entity_type
├── interval
├── timeout
├── execution_mode
└── collect()
```

---

# 55. probeの実行結果

共通スキーマ

```text
probe_id
entity_id
timestamp
status
latency
structured_data
evidence
error
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

# 56. probeと連携機能

初期

```text
Host
Network
SSH
Systemd
Slurm
GPU
NFS
Filesystem
Clock
Journal
```

将来

```text
ZFS
SMART
NVMe
Ceph
Lustre
BeeGFS
InfiniBand
IPMI
Redfish
UPS
Prometheus import
```

---

# 57. probe追加時の制約

新しいストレージ技術導入時に、

```text
core state engine
incident engine
database core
```

を書き換えなくて済むこと。

必要なのは原則、

```text
new capability
new probe
new diagnosis rule
```

のみ。

---

# 58. ホストの検査

取得

```text
boot ID
uptime
load
memory
CPU pressure
memory pressure
```

---

# 59. ネットワークの検査

```text
name resolution
ICMP optional
TCP
Sentinel RPC
service-specific TCP
```

---

# 60. SSHの検査

```text
TCP/22
SSH protocol response
local sshd service state
```

認証を必須としない。

---

# 61. agentの稼働確認API

Agentの稼働確認APIへ接続する。

返却

```text
entity identity
boot ID
agent version
capabilities
timestamp
health summary
```

---

# 62. サービスの監視

Service entityを、

```text
systemd
process
port
protocol
application-level probe
```

の組み合わせで観測可能にする。

---

# 63. Slurmとの連携

Slurmとの連携は、独立した機能として実装する。

MVPでは、

```bash
scontrol ping
scontrol show nodes -o
scontrol show partitions -o
squeue
```

等を利用する。

---

# 64. libslurmへの依存

MVPでは、コンパイル時にlibslurmへ依存しない。

理由

```text
Slurm version compatibility
binary portability
deployment simplicity
```

---

# 65. Slurmスケジューラの管理

例

```text
Scheduler:
example_cluster
```

状態

```text
AVAILABLE
DEGRADED
UNAVAILABLE
```

---

# 66. Slurmノードとの対応付け

HostとSlurmノードの識別子を関連付ける。

保存

```text
NodeName
NodeAddr
NodeHostName
State
StateFlags
Reason
ReasonTime
Partitions
CfgTRES
AllocTRES
```

---

# 67. Slurm上だけの異常

```text
Host       HEALTHY
Agent      HEALTHY
SSH        HEALTHY
slurmd     HEALTHY
Storage    HEALTHY
Slurm      DRAIN
```

↓

```text
SCHEDULER_DEGRADED
```

---

# 68. slurmdの停止

```text
Host       HEALTHY
Agent      HEALTHY
SSH        HEALTHY
slurmd     FAILED
Slurm      abnormal
```

↓

```text
SLURMD_SERVICE_FAILURE
```

---

# 69. Slurmのリソース設定の検証

agentが観測した値

```text
CPU
RAM
GPU
```

とSlurmの設定を比較。

Mismatch

```text
RESOURCE_CONFIGURATION_MISMATCH
```

---

# 70. ReturnToServiceによる状態変化

現在

```text
ReturnToService=1
```

Sentinelは、

```text
DOWN
→ host recovery
→ slurmd registration
→ Slurm automatic return
```

という状態遷移を記録する。

---

# 71. ストレージとの連携

StorageはNFSに限定しない。

ストレージ共通のインターフェース

```text
StorageEntity
```

を持つ。

---

# 72. 初期導入先のNFS構成

現在

```text
filesrv01 host
    ↓ provides
NFS service
    ↓ provides
storage entity
    ↓ used by
compute nodes
```

---

# 73. NFSクライアントの検査

```text
mount existence
mount source
server reachability
read-only state
active probe latency
```

---

# 74. NFSサーバーの検査

```text
service state
port 2049
export state
filesystem state
NFS health
```

---

# 75. NFS検査の安全制約

NFS障害時にシステムコールから応答が戻らなくなる場合を考慮する。

禁止

```text
unlimited df
unlimited stat
unlimited ls
```

---

# 76. 同時実行する検査の上限

各NFS mount

```text
max active filesystem probe = 1
```

前回のprobeが応答しないままなら、ファイルシステムにアクセスするprobeを新たに起動しない。

---

# 77. NFSの状態

```text
NFS_OK
NFS_SLOW
NFS_TIMEOUT
NFS_STUCK
```

---

# 78. ストレージ技術の変更

将来NFSからCephFSへ変更しても、

```text
NFS Capability OFF
Ceph Capability ON
Ceph Probe added
Dependency edges updated
```

のみで対応可能なこと。

---

# 79. ZFSとの連携

任意の機能とする。

```text
pool health
device health
scrub state
error counters
capacity
```

---

# 80. SMART・NVMeの検査

任意の機能とする。

```text
temperature
media errors
critical warning
unsafe shutdown
NVMe error
```

---

# 81. GPUの検査

Capability

```text
gpu.nvidia
```

MVPは`nvidia-smi`利用可。

取得

```text
GPU count
UUID
model
temperature
memory
utilization
power
```

---

# 82. GPU設定の不一致

SlurmのGRESに設定したGPU数と、実際に観測したGPU数を比較する。

---

# 83. カーネルイベントの収集

カーネルやjournalの情報は、イベントとして保存する。

例

```text
OOM
NFS server not responding
NVMe timeout
I/O error
GPU Xid
hung task
network down
```

イベントの記録と、診断の生成は分けて扱う。

---

# 84. Observation（観測記録）

Probeが返す事実を、

```text
Observation
```

として保存し、保存後は変更しない。

Diagnosisとは分けて管理する。

---

# 85. 観測記録の例

```text
entity:
filesrv01

probe:
nfs.service

result:
FAILED

timestamp:
10:35:03
```

---

# 86. 観測結果からの状態判定

Observation群からEntity component stateを導出する。

例

```text
HostState
ServiceState
StorageState
SchedulerState
```

---

# 87. 監視対象全体の状態

共通概要

```text
HEALTHY
DEGRADED
UNAVAILABLE
MAINTENANCE
UNKNOWN
```

状態の詳細分類は別のフィールドに保存する。

---

# 88. 状態の詳細分類

例

```text
SSH_DEGRADED
SCHEDULER_DEGRADED
STORAGE_DEGRADED
HOST_UNREACHABLE
SERVICE_FAILURE
```

---

# 89. 状態遷移

状態の変化をイベントとして記録する。

```text
HEALTHY
↓
DEGRADED
↓
UNAVAILABLE
```

---

# 90. 連続失敗・成功回数による状態判定

既定値の候補

```text
warning:
2 failures

critical:
3 failures

recovery:
2 successes
```

probeごとに変更可能。

---

# 91. 診断エンジン

ObservationとDependencyGraphから、

```text
DiagnosisCandidate
```

を生成する。

---

# 92. 診断のデータ構造

```text
diagnosis_type
affected_entities
suspected_root_entities
confidence
evidence_refs
reasoning_rules
timestamp
```

---

# 93. 診断の確信度

```text
LOW
MEDIUM
HIGH
CONFIRMED
```

---

# 94. 同じ入力から同じ診断を生成する

共通の診断処理はルールと依存グラフに基づく処理。

LLMを必須にしない。

---

# 95. 共通の依存先に起因する障害の統合

例

```text
node02
node03
node04
node13
node11
```

でストレージ障害が同時発生。

Graph

```text
all → filesrv01-backed-storage
```

filesrv01自身にも異常あり。

↓

```text
SHARED_STORAGE_FAILURE
confidence HIGH
```

---

# 96. クライアント単体のストレージ障害

filesrv01正常。

node02のみ異常。

↓

```text
LOCAL_STORAGE_CLIENT_FAILURE
```

候補。

---

# 97. スケジューラ全体の障害

複数計算ノードで同時にSlurm state取得不能だが、

```text
Host
SSH
Agent
```

は正常。

`slurmctld`サービス異常。

↓

```text
SLURM_CONTROL_PLANE_FAILURE
```

---

# 98. incidentの管理

複数event/diagnosisを、

```text
Incident
```

へまとめる。

---

# 99. incidentのデータ構造

```text
incident_id
status
severity
start_time
end_time
affected_entities
suspected_root_entities
diagnoses
evidence
timeline
```

---

# 100. 重大度

```text
INFO
WARNING
CRITICAL
```

依存する対象の数等を考慮可能。

---

# 101. 影響範囲に応じた重大度

filesrv01障害

```text
5 compute dependents
```

計算ノード1台の障害よりも重大度を高くできる。

---

# 102. 複数の診断を統合する条件

```text
temporal proximity
shared dependencies
same probe failures
same service
same scheduler
same network path
same storage
```

---

# 103. 障害発生時の記録

Incident発生時、

```text
raw observations
peer observations
entity state
dependency snapshot
Slurm state
service state
storage state
kernel events
```

を保存。

---

# 104. 直近の観測結果の保持

Agent/controllerは直近、

```text
30 minutes
```

程度の高頻度observationsを保持。

設定で変更可能。

---

# 105. ローカルspool

Controller到達不能でも、

```text
self observations
peer observations
important events
```

をローカル保存。

---

# 106. 再接続時の再送

Controller復旧後、spoolに保存したデータを時刻順に再送する。

観測ID等により同じIDを重複挿入しない取り込み処理を保証する。

---

# 107. 再起動の検出

Linux boot ID変更

```text
HOST_REBOOTED
```

として保存。

---

# 108. 時刻ずれ

分散した観測履歴の時刻整合性のため、

```text
CLOCK_SKEW
```

を監視。

---

# 109. controllerの障害

Controller自身の死活をpeerから監視。

---

# 110. controller停止時の代替通知

現在の推奨構成

```text
filesrv01
filesrv02
```

にcontroller停止時の代替通知機能を持たせる。

---

# 111. 通知の重複排除

Controllerとfallback observerから同じincidentが二重通知されないよう、

```text
incident fingerprint
notification lease
deduplication key
```

等を使用する。

---

# 112. 通知先との連携

```text
ntfy
Gotify
Slack
Discord
Email
generic webhook
```

通知処理は、通知先ごとのインターフェースで実装する。

---

# 113. 自動復旧の禁止

v1では禁止。

```text
reboot
systemctl restart
scontrol resume
mount/remount
```

を自動実行しない。

---

# 114. 調査コマンドの提示

診断結果から読み取り専用な推奨調査コマンドを提示可能。

例

```text
systemctl status slurmd
journalctl -u slurmd
scontrol show node node09
```

---

# 115. 認証と通信の保護

Agent/controller間通信は認証必須。

最終的な推奨構成

```text
mTLS
```

---

# 116. agentからの遠隔コマンド実行の禁止

Sentinel RPCから任意shellコマンドを実行可能にしてはならない。

Probeは事前定義済み処理だけを実行する。

---

# 117. 実行ユーザーの権限

可能な限り、

```text
sentinel
```

専用ユーザー。

読み取り専用監視を基本とする。

---

# 118. 追加権限が必要な検査

```text
SMART
NVMe
some journal access
BMC
```

等のみ明示的権限追加。

---

# 119. probeの失敗からagentを保護する

probeの失敗やpanicがagent全体を停止させないよう、実行処理を隔離する。

---

# 120. 外部コマンドの実行制限

以下を実行する場合

```text
systemctl
journalctl
scontrol
squeue
nvidia-smi
zpool
smartctl
```

必ずタイムアウトを設定する。

---

# 121. NFSで応答が戻らない処理の扱い

カーネルのD-stateではプロセスを終了できない場合がある。通常のコマンドのタイムアウトとは別に、応答が戻らない処理を管理する。

---

# 122. CPU・メモリなどの使用量

Agent目標

```text
idle CPU << 1 core
RAM <100 MB desirable
low disk I/O
low network traffic
```

---

# 123. 検査間隔の既定値

候補

```text
Sentinel heartbeat       5 s
peer RPC                 5 s
network                  5 s
SSH                     15 s
Slurm                    5 s
service                 10 s
GPU                     15 s
filesystem              30 s
NFS active              30 s
ZFS                     60 s
inventory              300 s
```

検査の集中を避けるため、実行時刻にばらつきを設ける。

---

# 124. データベース

MVP

```text
SQLite
```

WAL mode。

---

# 125. 保存処理のインターフェース

保存処理のインターフェースを設け、

将来

```text
PostgreSQL
```

へ移行可能。

---

# 126. DBに保存するデータ

```text
environments
clusters

entities
entity_labels
entity_capabilities
entity_addresses

dependencies

agent_instances
agent_sessions

probes
observations

entity_states
state_transitions

diagnoses

incidents
incident_entities
incident_evidence

notifications
acknowledgements
maintenance_windows
```

---

# 127. スキーマのバージョン管理

Configに必須

```toml
config_version = 1
```

Databaseにもスキーママイグレーションバージョンを持つ。

---

# 128. 設定形式の移行

将来、設定形式を変更したときに、

```bash
sentinel config migrate
```

等でマイグレーション可能にする。

---

# 129. 実環境の構成を設定で管理する

現在のcluster構成をソースコードへ入れない。

導入先固有の値は、

```text
runtime discovery
agent registration
configuration
```

のみ。

---

# 130. 最小のagent設定

例

```toml
config_version = 1
environment = "example-lab"

[controller]
address = "head01:7443"
```

役割ラベルも、自動検出と手動設定を組み合わせられる設計とする。

---

# 131. controllerの設定

```toml
config_version = 1

environment = "example-lab"

[controller]
listen = "0.0.0.0:7443"

[discovery.slurm]
enabled = true

[database]
path = "/var/lib/sentinel/sentinel.db"

[peer_monitoring]
degree = 3
```

---

# 132. 監視対象の明示的な設定

```toml
[[entities]]
type = "host"
name = "filesrv01"

[[entities]]
type = "host"
name = "filesrv02"
```

---

# 133. ラベル

追加の属性情報をラベルとして記録できる。

例

```text
location=entrance-side
location=professor-room
rack=rack01
storage_domain=legacy-o
```

ただし、共通の診断処理を特定のラベル名に依存させてはならない。

---

# 134. インストール

```bash
sudo install -m 0755 sentinel /usr/local/bin/sentinel
```

Agent

```bash
sudo sentinel install agent --controller head01:7443
```

Controller

```bash
sudo sentinel install controller
```

---

# 135. systemdのunit生成

インストール時に、必要なsystemdのservice/unitを生成できるようにする。

---

# 136. CLI

```bash
sentinel status

sentinel entity list
sentinel entity show <id>

sentinel dependency list
sentinel dependency graph

sentinel incident list
sentinel incident show <id>

sentinel peers

sentinel doctor

sentinel config check
sentinel config migrate

sentinel version
```

---

# 137. sentinel status

例

```text
ENVIRONMENT: example-lab

Infrastructure
────────────────────────────
head01       HEALTHY
filesrv01    HEALTHY
filesrv02    DEGRADED

Compute
────────────────────────────
node02     HEALTHY
node03     HEALTHY
...
node09      DEGRADED
```

---

# 138. 監視対象の詳細表示

```text
Entity:
Host(node09)

Capabilities:
host.metrics
ssh.server
slurm.compute
gpu.nvidia
storage.nfs.client

Overall:
DEGRADED

Classification:
SCHEDULER_DEGRADED
```

---

# 139. 依存関係の表示

```text
node02
   │
   ├── uses_scheduler → example_cluster
   │
   └── uses_storage → filesrv01-storage
                          │
                          └── provided_by → filesrv01
```

---

# 140. Web UI

MVP後。

主要画面

```text
Environment overview
Entity list
Entity details
Dependency graph
Incident timeline
Historical state
```

---

# 141. 監視対象の種類に依存しない画面

Frontendでも、

```text
if hostname == filesrv01
```

のようなホスト名に依存する例外処理は禁止する。

Entityのメタデータとcapabilityに応じて表示する。

---

# 142. 初期導入先の配置

```text
head01
  Host
  controller-related capabilities
  observer

filesrv01
  Host
  NFS/storage capabilities
  observer

filesrv02
  Host
  NFS/storage capabilities
  observer

13 Slurm compute nodes
  Host
  Slurm compute capability
  storage client capability
  optional GPU capability
```

---

# 143. 初期導入先の依存グラフ

概念

```text
node02 ─┐
node03 ─┤
node04 ─┼──→ filesrv01-storage → filesrv01
node13   ─┤
node11  ─┘


node05 ─┐
node06 ─┤
node07 ─┼──→ filesrv02-storage → filesrv02
node12─┤
node10  ─┘
```

node01/node08/node09のストレージdependencyは実行時自動検出または設定から登録する。

---

# 144. 将来の変更例Slurm controllerの移設

現在

```text
head01
```

将来

```text
control01
```

に移行。

必要変更

```text
deployment config
Slurm discovery result
dependency edges
```

のみ。

共通処理のコード変更不要。

---

# 145. 将来の変更例SlurmのHA化

```text
control01
control02
    ↓
Slurm scheduler
```

Service/Scheduler entityとdependency edge追加で対応する。

共通処理の構成変更不要。

---

# 146. 将来の変更例NFSからCephFSへの変更

旧

```text
NFS storage
```

新

```text
Ceph storage
```

変更

```text
disable NFS capabilities
enable Ceph capability
add Ceph probes
update dependencies
```

Incident engine等は変更不要。

---

# 147. 将来の変更例GPUノードの追加

20台追加されても、

```text
Slurm discovery
Agent registration
Capability discovery
```

で監視対象一覧へ追加。

Host固有ソースコード変更不要。

---

# 148. 将来の変更例ログインノードの追加

```text
login01
login02
```

追加。

Capabilities

```text
host
network
ssh
observer
```

必要なprobeだけ実行。

---

# 149. 将来の変更例InfiniBandへの対応

Capability

```text
network.infiniband
```

Probe

```text
IB link
port state
error counters
```

を追加。

Core変更不要。

---

# 150. 将来の変更例BMCへの対応

ExternalDependencyまたはEndpoint Entityとして、

```text
BMC
```

を追加。

Redfish連携機能追加のみ。

---

# 151. 将来の変更例複数クラスタの管理

```text
Environment: example-lab

Cluster:
example_cluster
cluster2
test_cluster
```

を同一controller/databaseで管理可能なデータモデルとする。

v1でUI対応まで必須ではない。

---

# 152. テスト方針

```text
Unit tests
Mock probes
Integration tests
VM-based tests
Production staged rollout
```

---

# 153. 人工的な観測結果を使うテスト

任意entity/probeを、

```text
healthy
degraded
timeout
failed
stuck
```

へ変更できるよう、人工的な検査結果を生成する仕組みを用意する。

---

# 154. 必須の障害シミュレーション

```text
SSH failure
slurmd failure
Slurm DRAIN
controller failure
host unreachable
NFS server failure
NFS client-only failure
multiple dependent node failures
GPU missing
clock skew
agent restart
host reboot
```

---

# 155. Slurm出力の互換性テスト

未知のSlurmフィールドやバージョン差でパーサー全体が失敗しないこと。

---

# 156. 導入段階1 controllerとSlurmの自動検出

ControllerとSlurmによる監視対象の自動検出を実装する。

```text
head01 only
```

---

# 157. 導入段階2 agent

Agentの共通処理を実装する。

```text
node09
node02
```

---

# 158. 導入段階3汎用の監視モデル

汎用のEntityとCapabilityのモデルを完成させる。

---

# 159. 導入段階4ファイルサーバー

filesrv01/02。

```text
NFS server
storage dependency
```

---

# 160. 導入段階5全ホストへのagent展開

全ホストへagentを展開する。

---

# 161. 導入段階6ノード間の相互監視

ノード間の相互監視を実装する。

---

# 162. 導入段階7診断とincident

診断とincidentの管理を実装する。

---

# 163. 導入段階8依存関係に基づく診断の統合

依存関係に基づく診断の統合を実装する。

---

# 164. 導入段階9通知

通知を実装する。

---

# 165. 導入段階10 Web UI

Web UI。

---

# 166. MVPの必須機能

MVP必須

```text
single Rust binary

Environment model
ManagedEntity model
Host entity
Service entity

Capability model

generic dependency graph

Slurm discovery
agent registration
static inventory

network probe
Sentinel RPC
SSH probe
systemd probe
Slurm probe
NFS client/server probe
basic GPU probe

peer monitoring

observations
states
state transitions

diagnosis engine
incident engine
dependency correlation

SQLite
local spool
notifications
CLI
```

---

# 167. MVPに含めない機能

```text
Ceph
Lustre
BeeGFS
IPMI
Redfish
InfiniBand
full ZFS telemetry
SMART fleet management
Prometheus replacement
Grafana replacement
automatic remediation
LLM diagnosis
HA Sentinel controller
```

ただし、共通処理の構成を変えずに追加できること。

---

# 168. 受け入れ条件初期導入先への配布

現在の16ホストへ同一アーキテクチャ用バイナリを配布できる。

---

# 169. 受け入れ条件Slurm DRAIN

```text
Host healthy
Slurm DRAIN
```

を、

```text
SCHEDULER_DEGRADED
```

と判定。

---

# 170. 受け入れ条件slurmd停止

`slurmd`停止をホスト障害と区別。

---

# 171. 受け入れ条件SSH停止

SSH停止をホスト到達不能と区別。

---

# 172. 受け入れ条件NFSサーバー停止

filesrv01のNFSサービス停止を、

```text
NFS_SERVICE_FAILURE
```

と識別。

---

# 173. 受け入れ条件共有ストレージ障害

filesrv01に依存する複数ノードで同時に異常が起きた場合、共有する依存先の障害として一つのincidentにまとめる。

---

# 174. 受け入れ条件クライアント単体のNFS障害

単一クライアントのみ異常ならファイルサーバー障害と誤診断しない。

---

# 175. 受け入れ条件経路障害

head01からだけ到達不能な場合は、ホスト全体の停止と断定しない。

---

# 176. 受け入れ条件controller停止

Sentinel controllerの停止後も、agentの観測とノード間の相互監視を継続する。

---

# 177. 受け入れ条件再起動

boot ID変更を正しく検出。

---

# 178. 受け入れ条件構成変更

テスト環境で、

```text
filesrv01 dependency removal
new filesrv03 registration
```

を行っても共通処理のコード変更なしで新構成を反映可能。

---

# 179. 受け入れ条件役割ラベルに依存しない監視

`role=fileserver`を削除しても、

```text
storage.nfs.server
```

capabilityが存在すればNFS監視が機能すること。

---

# 180. 受け入れ条件Slurm外のホスト

Slurm外ホストをSentinel監視対象一覧へ正常に登録・監視できること。

---

# 181. 受け入れ条件新しいprobeの追加

Dummy capability/probeを追加した際、

```text
DB core
incident core
agent architecture
```

を変更せずObservationとして取り込めること。

---

# 182. 推奨する実装技術

```text
Rust
Tokio
axum
serde
clap
tracing
sqlx
SQLite
rustls
```

Frontend

```text
React
TypeScript
```

---

# 183. リポジトリ構成

```text
/
├── Cargo.toml
├── src/
│   ├── main.rs
│   │
│   ├── environment/
│   ├── entity/
│   ├── capability/
│   ├── dependency/
│   ├── inventory/
│   │   ├── slurm/
│   │   ├── agent/
│   │   └── static_config/
│   │
│   ├── agent/
│   ├── controller/
│   ├── peer/
│   │
│   ├── probes/
│   │   ├── host/
│   │   ├── network/
│   │   ├── ssh/
│   │   ├── systemd/
│   │   ├── slurm/
│   │   ├── nfs/
│   │   ├── storage/
│   │   ├── gpu/
│   │   └── clock/
│   │
│   ├── observation/
│   ├── state/
│   ├── diagnosis/
│   ├── incident/
│   ├── correlation/
│   │
│   ├── notification/
│   ├── storage/
│   ├── api/
│   └── cli/
│
├── migrations/
├── tests/
├── fixtures/
├── docs/
└── packaging/
```

---

# 184. 共通処理の依存方向

モジュール依存は原則、

```text
integrations/probes
        ↓
observation
        ↓
state
        ↓
diagnosis
        ↓
incident
```

とする。

CoreがSlurm/NFS実装へ逆依存してはならない。

---

# 185. 実環境への過度な特化を避ける

本プロジェクトを実装する際、

現在のexample_clusterの具体的構成に最適化しすぎて共通処理の構成を固定化してはならない。

現在のホスト名、partition名、ファイルサーバー構成は、

```text
fixtures
deployment config
runtime discovery
tests
```

には使用してよい。

しかし共通処理へ埋め込んではならない。

---

# 186. 禁止事項

以下を禁止する。

- 監視対象の種類をHostだけに限定する
- compute、fileserver、controllerを固定の継承階層にする
- roleによってprobeを直接分岐する
- IPアドレスをentityの識別キーにする
- Slurm NodeNameとホスト名が同じと仮定する
- Slurmのノード一覧をSentinelの監視対象一覧全体と同一視する
- ストレージの実装をNFSだけに限定する
- filesrv01、filesrv02という名前を共通処理のコードに書く
- Slurmパーティション名の-o、-iに共通処理を依存させる
- controllerのホスト名をソースコードへ埋め込む
- controllerを1台に限定するDBスキーマにする
- 依存関係を木構造に限定する
- 1ホストにつきサービスは一つだけと仮定する
- 全ホストで同じprobeを実行する
- probeの失敗でagent全体をpanicさせる
- 外部コマンドをタイムアウトなしで実行する
- NFSで応答が戻らないシステムコールを無制限に起動する
- 一つの観測元での失敗から、ホスト全体の停止と判定する
- ネットワークの観測だけで電源断と判定する
- controllerの停止中に観測結果を失う
- v1で自動復旧する
- 共通処理をLLMへ依存させる

---

# 187. 設計全体の方針

Cluster Sentinelは、ホスト以外も扱える共通の監視モデルを使う。
Slurmやストレージ固有の監視は、連携機能として実装する。
複数地点の観測結果と依存関係を使って、障害を診断する。
Slurmだけを監視するツールに設計を限定しない。

---

# 188. 構成変更への対応方法

現在は、

```text
Slurm
+
NFS filesrv01
+
NFS filesrv02
+
GPU compute nodes
```

を主要対象とする。

将来、

```text
Slurm HA
+
CephFS
+
Lustre
+
login nodes
+
InfiniBand
+
BMC
+
storage cluster
+
複数scheduler
+
複数cluster
```

へ構成変更された場合も、

```text
Entity追加
Capability追加
Probe追加
Dependency変更
Configuration変更
```

で対応し、

```text
Observation
State
Diagnosis
Incident
Correlation
```

の共通処理の構成を原則変更しない。

---

# 189. 監視と診断の全体像

計算ノード、スケジューラ、controller、ファイルサーバー、ストレージサービスをManagedEntityとして管理する。
将来はネットワーク機器やBMCなどにも拡張する。
capabilityに応じたprobeを複数地点から実行し、観測結果、依存関係、状態遷移を記録する。
診断ルールで原因を判定し、関連する診断をincidentへまとめる。
クラスタの構成が変わっても、この共通処理を使って長期運用できる設計とする。

`example_cluster`を最初の本番導入先とする。共通処理の設計を、この環境固有の構成、Slurmパーティション、ホストの命名規則、NFS構成へ依存させてはならない。
