# VM / 実クラスタ検証要件 (Level 4)

Docker 疑似クラスタでは再現できず、VM または実機での検証が必要な項目の一覧です
（`docs/IMPLEMENTATION.md` §28-§32, M11）。

**再現できないものを「再現できたことにしない」** ための文書です。
container での検証が real machine の検証を代替すると仮定した箇所は、
実際に壊れたときに最も高くつきます。

## 検証環境

最小構成で十分です。

```text
ctrl01      controller + slurmctld
compute01   agent + slurmd
filesrv01   agent + 実 NFS server
```

VM を日常の主開発環境にする必要はありません。
以下の項目を検証する必要が生じた段階で用意してください。

## 検証項目

### 1. Reboot と boot ID

| 項目 | 手順 | 期待される結果 |
| --- | --- | --- |
| boot ID の変化検出 | `compute01` を reboot | `host.boot` observation が記録され、`previous_boot_id` と `boot_id` が異なる |
| agent 再起動との区別 | `systemctl restart sentinel-agent` | reboot として記録され **ない** |
| reboot 後の復帰 | reboot 完了まで待つ | `ReturnToService=1` により Slurm へ自動復帰し、その遷移が記録される |

Docker で再現できない理由: container に boot semantics が無く、
`/proc/sys/kernel/random/boot_id` は host のものを見ています。

### 2. NFS hard mount と D-state

**最も重要な検証項目です。** Sentinel の NFS probe 設計全体が、
この挙動を前提にしています。

| 項目 | 手順 | 期待される結果 |
| --- | --- | --- |
| hard mount の停止 | `filesrv01` の NFS service を停止（client は hard mount） | `nfs.client.io` が `NFS_TIMEOUT` または `NFS_STUCK` を報告 |
| **thread が蓄積しないこと** | 停止状態を 10 分以上維持 | agent のスレッド数が増え続けない。`max_outstanding = 1` が効いている |
| agent の生存 | 同上 | agent が応答を続け、`nfs.client.mount` など他の probe は動作を続ける |
| D-state の観測 | `ps -eo stat,comm \| grep '^D'` | blocked task は **1 本のみ** |
| 復旧 | NFS service を再開 | probe が復帰し、state が recovery する |

Docker で再現できない理由: container 内の hang が host を巻き込み得るため、
`storage-service` は userspace の停止を模擬しているだけです。
`degrade-storage hang` は D-state ではありません。

### 3. 実 systemd

| 項目 | 手順 | 期待される結果 |
| --- | --- | --- |
| unit 状態の取得 | `systemctl stop slurmd` | `systemd.unit` probe が `ActiveState=inactive` を報告 |
| failed unit | unit を意図的に失敗させる | `Result=exit-code` を検出 |
| hardening 下での動作 | 生成された unit で起動 | `ProtectSystem=strict` 下で全 probe が動作する |
| 権限不足の扱い | 非特権ユーザーで SMART 等を試行 | `UNSUPPORTED` であり `FAILED` ではない |

Docker で再現できない理由: container では supervisord を使用しています。

### 4. 実 GPU

| 項目 | 期待される結果 |
| --- | --- |
| `nvidia-smi` の実出力 | parser が全 field を正しく取得する |
| GRES との突合 | `slurm.conf` の GPU 数と一致すれば診断なし |
| GPU 数の不一致 | `GPU_CONFIGURATION_MISMATCH` |
| driver 異常時 | `nvidia-smi` が失敗しても agent が生存する |

### 5. ハードウェア障害

| 項目 | 備考 |
| --- | --- |
| SMART / NVMe の異常 | v1 では probe 未実装。将来 capability 追加で対応 |
| NIC のハードウェア故障 | `HOST_UNREACHABLE` になるが `POWER_OFF` とは言わないこと |
| 物理電源断 | 同上。BMC 無しでは電源状態を主張しないこと |

### 6. Kernel event

| 項目 | 手順 | 期待される結果 |
| --- | --- | --- |
| journal の読み取り | `journalctl` へアクセス | `journal.read` capability が検出される |
| OOM / I/O error | 実際に発生させる、または過去ログで確認 | raw event として保存され、それ自体が診断結果とされないこと |

### 7. Clock skew

| 項目 | 手順 | 期待される結果 |
| --- | --- | --- |
| skew の検出 | 1 台の時刻を意図的にずらす | heartbeat が `clock_skew_ms` を報告する |
| observation を捨てないこと | 同上 | skew があっても observation は保存される（`IMPLEMENTATION.md` §47）|

## 実クラスタでの障害注入について

`mizuno_cluster` は開発環境ではありません（`IMPLEMENTATION.md` §31, §32）。

**自動実行してはならないもの:**

* NFS server の停止
* network 全体に対する iptables 変更
* reboot
* filesystem 操作

これらは管理者の判断のもとでのみ、計画して実施してください。
Docker / VM で代替できるものはそちらで行ってください。

導入順は [OPERATIONS.md](OPERATIONS.md) の「段階的な導入」に従ってください。

## 現在の検証状況

| Level | 内容 | 状況 |
| --- | --- | --- |
| 1 | unit / mock | 自動化済み・CI 実行可能 |
| 2 | in-process simulation | 自動化済み・CI 実行可能 |
| 3 | Docker 疑似クラスタ | 自動化済み（`dev/compose/scripts/acceptance`、23 項目） |
| 4 | VM / 実機 | **未実施**。本書が要件一覧 |
