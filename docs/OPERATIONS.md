# 運用ガイド

Cluster Sentinel を実際に運用するためのガイドです。

## 前提

* production は systemd + native host + 単一 `sentinel` バイナリ。
* Sentinel は **監視対象を変更しません**。
  reboot・`systemctl restart`・remount・`scontrol update` を実行しません。
  診断と、read-only な調査コマンドの提示のみを行います。

## 導入

### 1. バイナリの配置

```bash
sudo install -m 0755 sentinel /usr/local/bin/sentinel
sentinel version
```

アーキテクチャが同じであれば、全 host に同じバイナリを配布できます。

### 2. systemd unit の生成

```bash
sudo sentinel install controller     # controller host で
sudo sentinel install agent          # 各 agent host で
```

`--dry-run` を付けると unit を出力するだけで書き込みません。

生成される unit は既定で hardening 済みです
（`NoNewPrivileges` / `ProtectSystem=strict` / `CapabilityBoundingSet=` 等）。
Sentinel は read-only な監視 daemon であり、unit もそう宣言します。

### 3. サービスユーザーと credential

```bash
sudo useradd --system --no-create-home --shell /usr/sbin/nologin sentinel
sudo install -d -o sentinel -g sentinel -m 0750 /var/lib/sentinel
sudo install -d -m 0755 /etc/sentinel

head -c 32 /dev/urandom | base64 | sudo tee /etc/sentinel/token > /dev/null
sudo chown sentinel:sentinel /etc/sentinel/token
sudo chmod 0400 /etc/sentinel/token
```

**同一 credential を environment 内の全 host へ配布します。**
credential 無しでは controller も agent も起動を拒否します。

### 4. 設定と検証

`/etc/sentinel/config.toml` を作成し、起動前に検証します
（[CONFIGURATION.md](CONFIGURATION.md) 参照）。

```bash
sentinel config check
```

### 5. 起動

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now sentinel-controller   # または sentinel-agent
systemctl status sentinel-controller
```

## 段階的な導入

実クラスタは開発環境ではありません（`IMPLEMENTATION.md` §31）。
以下の順で導入してください。

| 段階 | 内容 | 確認すること |
| --- | --- | --- |
| 1 | controller のみ（read-only、Slurm discovery） | `sentinel status` が既存構成を正しく表示する |
| 2 | agent 1 台 | 登録される。`sentinel entity show <host>` が妥当 |
| 3 | agent 複数台 | 全台登録。`sentinel peers` で observer が付く |
| 4 | peer monitoring 有効化 | 誤検知が出ないことを数日観察 |
| 5 | notification 有効化 | まずテスト用の宛先へ |
| 6 | 全台展開 | |

**実クラスタで危険な障害注入を自動実行しないでください**
（`IMPLEMENTATION.md` §32）。
NFS server の停止、network-wide な iptables 変更、reboot、filesystem 操作は
管理者判断のもとでのみ行います。
Docker / VM で代替できるものはそちらで行ってください。

## 日常の確認

```bash
sentinel status              # クラスタ全体。異常があれば exit 2
sentinel diagnose            # 何が壊れていて、なぜか
sentinel incident list       # 対応が必要な incident
sentinel incident show <id>  # 根拠・timeline・evidence
sentinel peers               # observer の割り当て
sentinel doctor              # この host から見た自分自身
```

すべて `--json` に対応しているため、スクリプトから利用できます。

`sentinel status` は異常があれば exit code 2 を返します。

## 診断結果の読み方

| 診断 | 意味 | 最初に見るもの |
| --- | --- | --- |
| `HOST_UNREACHABLE` | 複数の独立した observer から到達不能 | console / BMC |
| `PATH_SPECIFIC_NETWORK_FAILURE` | 一部経路のみ異常。**host は生きている** | 両端の network |
| `SSH_SERVICE_FAILURE` | SSH のみ停止 | `systemctl status sshd` |
| `SENTINEL_AGENT_FAILURE` | 監視のみ停止。host は正常 | `systemctl status sentinel-agent` |
| `SLURMD_SERVICE_FAILURE` | host は正常、`slurmd` のみ | `journalctl -u slurmd` |
| `SLURM_ONLY_DEGRADATION` | host は完全に正常。Slurm 上のみ DRAIN | `scontrol show node <n>` の Reason |
| `SLURM_CONTROL_PLANE_FAILURE` | control plane 自体 | `scontrol ping` |
| `NFS_SERVICE_FAILURE` | fileserver は稼働、export service のみ | `exportfs -v` |
| `NFS_CLIENT_FAILURE` | 1 client のみ。**server は正常** | 当該 client の mount |
| `SHARED_STORAGE_FAILURE` | 同一 storage の複数 client | storage を提供する host |
| `RESOURCE_CONFIGURATION_MISMATCH` | 設定と実ハードウェアの不一致 | `slurm.conf` |

### Sentinel が言わないこと

**`POWER_OFF` は出力しません。** network の沈黙は電源状態の証拠ではありません。
電源断・NIC 故障・switch 障害は network から見れば同じです。
`HOST_UNREACHABLE` は「誰も到達できない」までしか主張せず、
その先は console / BMC を確認するよう促します。

**observer が 1 台の場合、到達性の診断を行いません。**
1 視点では host の死と経路障害を区別できないためです。
`sentinel peers` で observer が付いているか確認してください。

## Notification

```toml
[notification]
min_severity = "warning"

[[notification.webhooks]]
name = "ntfy"
url = "https://ntfy.example.org/cluster-sentinel"
```

送信されるのは **変化があったとき** だけです。

| 送信する | 送信しない |
| --- | --- |
| incident の発生 | 継続中の incident（何度 polling しても） |
| severity の上昇 | 変化のない状態 |
| 診断内容の変化 | `min_severity` 未満（ただし復旧は常に送る） |
| 復旧 / 解決 | maintenance 中の対象 |

重複排除は宛先ごとに行われるため、
Slack を止めても pager は止まりません。
controller と fallback notifier が同じ incident を検知しても、通知は 1 回です。

## Maintenance window

計画作業中の通知を抑止します。

**抑止するのは通知だけです。**
observation・state・diagnosis は継続し、
異常な状態が healthy に書き換えられることはありません。
そうしなければ、作業中に発生した本物の障害が隠れ、
いつ始まったのかを後から再構成できなくなります。

incident が抑止されるのは、
**影響を受けている entity がすべて** maintenance 対象である場合のみです。
1 台の作業が、4 台を巻き込む障害を隠すことはありません。

## トラブルシューティング

### agent が登録されない

```bash
sentinel doctor                          # credential が設定されているか
systemctl status sentinel-agent
journalctl -u sentinel-agent -n 50
curl -fsS http://<controller>:7443/v1/health   # controller は生きているか
```

`Credential: NOT CONFIGURED` と出る場合、`/etc/sentinel/token` が
サービスユーザーから読めていません。

### 監視されていない host がある

capability が付いていない可能性があります。

```bash
sentinel entity show <host>   # Capabilities を確認
sentinel doctor               # その host で、各 capability の判定理由を表示
```

`sentinel doctor` は capability ごとに
「検出された / この host には無い / 設定で有効化 / role による推定」
のいずれかを表示します。

必要であれば設定で明示的に上書きできます。

```toml
[capabilities]
"storage.nfs.server" = "force"
```

### 到達性の診断が出ない

observer が不足しています。

```bash
sentinel peers    # observer の付いていない entity を表示する
```

`observer.peer` capability を持つ host を増やしてください。

### controller が停止した場合

agent は観測と spool を継続します。
controller 復旧後、spool は自動で再送されます（重複挿入は起きません）。

```bash
sentinel doctor   # spool の深さを確認
```

## バックアップ

```bash
systemctl stop sentinel-controller
cp /var/lib/sentinel/sentinel.db /path/to/backup/
systemctl start sentinel-controller
```

WAL mode のため、稼働中のコピーは `sqlite3 .backup` を使用してください。

```bash
sqlite3 /var/lib/sentinel/sentinel.db ".backup /path/to/backup/sentinel.db"
```

## アップグレード

1. `sentinel config check` を新バイナリで実行する
2. controller を先に更新する
3. agent を順次更新する

binary version と protocol version は分離されています。
同一 protocol version であれば、混在状態でも動作します。

## v1 で行わないこと

* 自動復旧（reboot / restart / remount / `scontrol update`）
* LLM による診断
* controller の HA
* Prometheus / Grafana の置き換え

いずれも core architecture を変更せずに追加できる設計にしてありますが、
v1 の範囲外です。
