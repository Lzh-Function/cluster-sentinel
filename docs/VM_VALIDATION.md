# VM / 実クラスタ検証要件 (Level 4)

Docker疑似クラスタでは再現できず、VMまたは実機での検証が必要な項目の一覧です
（`docs/IMPLEMENTATION.md` §28-§32, M11）。

コンテナで検証した項目と、実機での確認が必要な項目を区別します。
カーネルやハードウェアの挙動は、コンテナの試験だけでは確認できません。

## 検証環境

最小構成で十分です。

```text
ctrl01      controller + slurmctld
compute01   agent + slurmd
filesrv01   agent + 実 NFS server
```

VMを日常の主開発環境にする必要はありません。
以下の項目を検証する必要が生じた段階で用意してください。

## 検証項目

### 1. Rebootとboot ID

| 項目 | 手順 | 期待される結果 |
| --- | --- | --- |
| boot IDの変化検出 | `compute01`を再起動 | `host.boot` observationが記録され、`previous_boot_id`と`boot_id`が異なる |
| agent再起動との区別 | `systemctl restart sentinel-agent` | 再起動として記録されない |
| 再起動後の復帰 | 再起動完了まで待つ | `ReturnToService=1`によりSlurmへ自動復帰し、その遷移が記録される |

コンテナはホストと同じカーネルを使い、
`/proc/sys/kernel/random/boot_id`もホストの値を返します。
そのため、コンテナの再起動では実機のboot ID変更を再現できません。

### 2. NFS hard mountとD-state

最も重要な検証項目です。SentinelのNFS probe設計全体が、
この挙動を前提にしています。

| 項目 | 手順 | 期待される結果 |
| --- | --- | --- |
| hard mountの停止 | `filesrv01`のNFSサービスを停止（クライアントはhard mount） | `nfs.client.io`が`NFS_TIMEOUT`または`NFS_STUCK`を報告 |
| スレッドが蓄積しないこと | 停止状態を10分以上維持 | agentのスレッド数が増え続けない。`max_outstanding = 1`が適用されている |
| agentの生存 | 同上 | agentが応答を続け、`nfs.client.mount`など他のprobeは動作を続ける |
| D-stateの観測 | `ps -eo stat,comm \| grep '^D'` | blocked taskは1本のみ |
| 復旧 | NFSサービスを再開 | probeが復帰し、stateがrecoveryする |

コンテナ内で応答が戻らないNFS処理は、ホストにも影響するため、Dockerでは検証しません。
`storage-service`では、ユーザー空間のサービス停止だけを模擬します。
`degrade-storage hang`はD-stateではありません。

### 3. 実systemd

疑似クラスタはcgroup v2を実階層で使用していますが、
scopeを作成するのはsystemdではなくslurmd自身です
（`IgnoreSystemd=yes`）。コンテナにsystemdもdbusも無いためです。
本番環境の経路は未検証であり、ここで確認します。

| 項目 | 手順 | 期待される結果 |
| --- | --- | --- |
| systemd管理下のcgroup scope | 通常どおり`slurmd`を起動 | dbus経由でscopeが作成され、cgroupエラーが出ない |
| unit状態の取得 | `systemctl stop slurmd` | `systemd.unit` probeが`ActiveState=inactive`を報告 |
| failed unit | unitを意図的に失敗させる | `Result=exit-code`を検出 |
| 権限とアクセスの制限下での動作 | 生成されたunitで起動 | `ProtectSystem=strict`下で全probeが動作する |
| 権限不足の扱い | 非特権ユーザーでSMART等を試行 | `UNSUPPORTED`であり`FAILED`ではない |

Dockerで再現できない理由は、 コンテナではsupervisordを使用しています。

### 4. 実GPU

| 項目 | 期待される結果 |
| --- | --- |
| `nvidia-smi`の実出力 | パーサーが全フィールドを正しく取得する |
| GRESとの突合 | `slurm.conf`のGPU数と一致すれば診断なし |
| GPU数の不一致 | `GPU_CONFIGURATION_MISMATCH` |
| driver異常時 | `nvidia-smi`が失敗してもagentが生存する |

### 5. ハードウェア障害

| 項目 | 備考 |
| --- | --- |
| SMART / NVMeの異常 | v1ではprobe未実装。将来capability追加で対応 |
| NICのハードウェア故障 | `HOST_UNREACHABLE`になるが`POWER_OFF`とは言わないこと |
| 物理電源断 | 同上。BMCがない場合は電源断と判定しないこと |

### 6. Kernel event

コンテナにはsystemdが無いため、疑似クラスタでは`journal.read`が
検出されず、`journal.events` probeは動作しません。実機での検証が必須です。

| 項目 | 手順 | 期待される結果 |
| --- | --- | --- |
| journalの読み取り | `sentinel doctor` | `journal.read`がdetected |
| eventの収集 | `logger -p kern.err "test I/O error on dev sda1"`等 | observationの根拠となる観測に記録される |
| OOM | 実際に発生させる、または過去ログで確認 | `oom`として記録されるが、それ自体でホストをdegradedにしない |
| I/O error / hung task | 同上 | `serious`として記録され、ホストcomponentがdegradedになる |
| 再起動をまたいだ保存 | event発生後に再起動 | controller側にeventが残っており、再起動後も参照できる |
| 出力上限 | 大量にログが出る状態で確認 | truncateされ、その事実が記録される |

### 7. Clock skew

| 項目 | 手順 | 期待される結果 |
| --- | --- | --- |
| skewの検出 | 1台の時刻を意図的にずらす | heartbeatが`clock_skew_ms`を報告する |
| observationを捨てないこと | 同上 | skewがあってもobservationは保存される（`IMPLEMENTATION.md` §47）|

## 実クラスタでの障害注入について

`example_cluster`は開発環境ではありません（`IMPLEMENTATION.md` §31, §32）。

自動実行してはならないもの

* NFSサーバーの停止
* ネットワーク全体に対するiptables変更
* reboot
* ファイルシステム操作

これらは管理者の判断のもとでのみ、計画して実施してください。
Docker / VMで代替できるものはそちらで行ってください。

導入順は [OPERATIONS.md](OPERATIONS.md) の「段階的な導入」に従ってください。

## 現在の検証状況

| Level | 内容 | 状況 |
| --- | --- | --- |
| 1 | unit / mock | 自動化済み・CI実行可能 |
| 2 | in-process simulation | 自動化済み・CI実行可能 |
| 3 | Docker疑似クラスタ | 自動化済み（`dev/compose/scripts/acceptance`、33項目）。実クラスタの構成を再現。controllerホストがagentを持ち、計算ノードの1台がストレージも提供し、ヘッドノードはexportゼロでexportポートに応答する。cgroupはv2実階層だがscope作成はsystemd経由ではない |
| 4 | VM / 実機 | 一部実施。下記の通り |

### 実機で確認できたこと

16ホスト（x86_64とARM64の混在）、Slurm、ZFS `sharenfs`によるNFS、
非標準SSHポートという構成で運用し、次を確認しました。

| 項目 | 結果 |
| --- | --- |
| Ansibleによるagentの一括配布 | 15ノードへ配布、冪等性を確認 |
| 異なるアーキテクチャの混在 | 各ホストが自分のarchのバイナリを取得 |
| 非標準SSHポート | agentが`sshd_config`から読んで報告。手動宣言なしで到達性判定が成立 |
| 実NFS | ZFS `sharenfs`（`root_squash`）。依存グラフが実構成と一致 |
| 通知の全経路 | 障害 → incident → 通知 → 復旧 → RESOLVEDまで実測 |
| controller再起動 | 対応中のincidentが再通知されないこと |
| 複数NICのホスト | agentが報告するアドレスの選定 |
| controllerとagentの同居 | 設定パスを分けて共存 |
| probe audit | 監視漏れを1件検出し、修正後に観測結果が届くことを確認 |

### 実機でまだ確認していないこと

| 項目 | なぜ未確認か |
| --- | --- |
| TLS | 当該クラスタは平文HTTPで運用中 |
| 長期運用後の保持期間 / prune | 数週間分の蓄積がまだない |
| 本物のホスト障害（`HOST_UNREACHABLE`） | 稼働中のクラスタでノードを停止できない。疑似クラスタでは検証済み |
| BMC / 電源状態 | v1の対象外 |

## TLS

疑似クラスタは平文HTTPのまま動作します（TLSは追加的な設定であり、
既定の経路を変えないことを確認するため）。

protocol部分は`tests/tls.rs`が実際のTLS listenerと実際のクライアント、
その場で生成した証明書で検証しています（mutual TLSの受理・拒否を含む）。

デーモンの設定読み込み経路は、本repositoryの開発環境で
`sentinel controller`をmutual TLS設定で起動し、
`curl`で外部から確認済みです（平文接続・CA無し・クライアント証明書無しの
3通りがすべて拒否され、クライアント証明書ありのみ成功）。

実機で確認すべきこと

| 項目 | 手順 | 期待される結果 |
| --- | --- | --- |
| 起動 | controllerのログ | `controller listening ... tls=true` |
| 証明書チェーン | `openssl s_client -connect <host>:7443 -CAfile <ca>` | `Verify return code: 0` |
| agentの接続 | agentのログ | 登録が成功し、`insecure_skip_verify`の警告が出ないこと |
| 証明書の期限切れ | 期限切れ証明書で起動 | agentが接続を拒否し、ログに理由が出ること |
| 材料の欠損 | `key`を読めない状態にする | controllerが起動に失敗する（平文で起動しないこと） |

## Retention

`sentinel prune`は疑似クラスタの実データに対して確認済みです
（10,350 observationを削除、`--vacuum`で10.8 MiB → 2.3 MiB、
削除後も診断は正常）。

実機で確認すべきこと

| 項目 | 手順 | 期待される結果 |
| --- | --- | --- |
| 数か月分に対する初回prune | 実際に実行し、所要時間を測る | 完了すること。その間agentがspoolで耐えること |
| 定期実行 | controllerを1時間以上動かす | ログにpruneの記録が出ること |
| 容量の頭打ち | 数週間の運用後にdatabaseサイズを確認 | 保持期間に対応する値で安定すること |
| 長期停止からの復帰 | controllerを数日停止して起動 | 起動時のpruneが滞留分を処理すること |
