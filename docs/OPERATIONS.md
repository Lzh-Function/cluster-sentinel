# 運用ガイド

導入が済んだあと、日々どう使うかのガイドです。
障害が出たときの読み方と、よくある症状への対処が中心です。

> 導入がまだの場合は [GETTING_STARTED.md](GETTING_STARTED.md)、
> 本番構成へ広げる場合は [DEPLOYMENT.md](DEPLOYMENT.md) を先に。

## 前提

* 本番環境はsystemd + nativeホスト + 単一`sentinel`バイナリ。
* Sentinelは監視対象を変更しません。
  reboot・`systemctl restart`・remount・`scontrol update`を実行しません。
  診断と、読み取り専用な調査コマンドの提示のみを行います。

## 導入

Rustツールチェインは不要です。
[Releases](https://github.com/Lzh-Function/cluster-sentinel/releases) の
静的リンクバイナリ（x86_64 / aarch64）を配置します。

```bash
sha256sum -c sentinel-x86_64-unknown-linux-musl.sha256
sudo install -m 0755 sentinel-x86_64-unknown-linux-musl /usr/local/bin/sentinel

sudo useradd --system --no-create-home --shell /usr/sbin/nologin sentinel
sudo sentinel install controller     # または agent
sudo chown -R sentinel:sentinel /etc/sentinel
```

先に`/usr/local/bin`へ配置してから`install`を実行してください。
ダウンロード先のまま実行すると、unitがそのパスを指してしまいます
（`install`は警告します）。

`install`は`/etc/sentinel`を作り、設定ファイル・systemd unit・
（controllerのみ）cluster credentialを生成して、残りの手順を表示します。
既存のファイルは上書きしません（`--force`を付けた場合のみ）。

`/var/lib/sentinel`はsystemdが初回起動時に作ります。

生成された設定ファイルには全設定が既定値のまま書き出されています。
書き換えが必要なのは`CHANGE-ME`を含む行だけです。

```bash
sudo -u sentinel sentinel config check
sudo systemctl daemon-reload
sudo systemctl enable --now sentinel-controller
```

ノードが多い場合は [Ansible ロール](../deploy/ansible/) を使ってください。

手順の詳細は [DEPLOYMENT.md](DEPLOYMENT.md) を参照してください。

### 設定ファイルだけ生成する

```bash
sentinel config init --role agent --dry-run          # 中身を見る
sentinel config init --role agent --output ./a.toml  # 書き出す
```

## 監視頻度の変更

probeごとに`[probes]`で変更します。書かれていないprobeは既定のままです。

```toml
[probes."network.tcp"]
interval = "15s"
```

生成された設定ファイルに全probeの既定値がコメントアウトされて入っているので、
変えたい行のコメントを外してください。
一覧は [CONFIGURATION.md](CONFIGURATION.md) の`[probes]`にあります。

## 段階的な導入

実クラスタは開発環境ではありません（`IMPLEMENTATION.md` §31）。
以下の順で導入してください。

| 段階 | 内容 | 確認すること |
| --- | --- | --- |
| 1 | controllerのみ（読み取り専用、Slurm自動検出） | `sentinel status`が既存構成を正しく表示する |
| 2 | agent 1台 | 登録される。`sentinel entity show <host>`が妥当 |
| 3 | agent複数台 | 全台登録。`sentinel peers`でobserverが付く |
| 4 | peer monitoring有効化 | 誤検知が出ないことを数日観察 |
| 5 | notification有効化 | まずテスト用の宛先へ |
| 6 | 全台展開 | |

実クラスタで危険な障害注入を自動実行しないでください
（`IMPLEMENTATION.md` §32）。
NFSサーバーの停止、ネットワーク全体に影響するiptablesの変更、再起動、ファイルシステム操作は
管理者判断のもとでのみ行います。
Docker / VMで代替できるものはそちらで行ってください。

## 日常の確認

CLIはrootか`sentinel`ユーザーで実行します。
設定ファイルは全ユーザーから読み取り可能にしていません
（webhook URL自体がcredentialを含みうるため）。

```bash
sudo -u sentinel sentinel status
```

```bash
sentinel status              # クラスタ全体。異常があれば exit 2
sentinel diagnose            # 障害の原因と判断の根拠
sentinel incident list       # 対応が必要な incident
sentinel incident show <id>  # 根拠・timeline・evidence
sentinel peers               # observer の割り当て
sentinel doctor              # この host から見た自分自身
sentinel prune --dry-run     # 保持期間を過ぎた記録の量
```

### この仕組みを他人に説明する

```bash
sentinel explain                 # 3 つすべて
sentinel explain capabilities    # 何が probe を有効にし、どう判定されるか
sentinel explain probes          # 各 probe が実際に何を実行するか
sentinel explain paths           # どの host を誰が、何で監視しているか
```

`status`は監視対象の状態と対応中の障害を表示します。
`explain`は、検査内容、有効化の条件、監視元の割り当てを説明します。

### HEALTHY表示と観測結果を確認する

HEALTHYという表示だけで、必要な検査がすべて動いているとは判断できません。
監視元の割り当てと最新の観測結果も確認してください。
過去には、次の不具合がありました。

- ある検査がどこからも実行されていなかった。全ノードHEALTHYのまま、
  数か月気づかれなかった
- openなCRITICALの通知が一度も飛んでいなかった。`status`は
  「31 healthy」と表示していた

導入直後と、構成を変えたあとには、次の2つを見てください。

```bash
sentinel explain paths
```

各ホストの`watched by`を見ます。ここが空、または1台しかないホストは、
到達性の診断に必要な観測元が足りません。最低2つの独立した観測元が
必要で、1台ではSentinelは判断を保留します。

```bash
sentinel entity observations <host>
```

`WHEN`列が現在時刻の近くで更新され続けているかを見ます。
特定の検査だけを追うこともできます。

```bash
sentinel entity observations <host> --probe nfs.server.exports
```

「観測がありません」と出たら、検査が実行され、controllerへ結果が届いているか確認します。
必要なcapabilityが付いていない場合もあるので、`sentinel entity show <host>`の
Capabilitiesを確認してください。

### `sentinel audit`を定期実行する

監視漏れを継続的に確認するには、手動の確認に加えて`sentinel audit`を定期実行します。
`sentinel audit`は「有効なのに観測を出していないprobe」を挙げ、
あればexit code 2を返します。cronに置いてください。

```bash
sudo -u sentinel sentinel audit > /dev/null || echo "cluster-sentinel: 監視に穴があります"
```

観測結果が届いていないprobeがあると`status`の末尾にも1行出ます。

```
⚠ 1 probe(s) have never reported at all; run `sentinel audit` for which
```

構成を変えた直後は必ず確認してください。capabilityの判定が変わると、
必要なprobeが停止していても見落とす場合があります。

### 名前が重複する場合

ストレージentityは提供元ホストと同じ名前になります
（`host/filesrv02`と`storage/filesrv02`）。型を付けない名前で指すと
どちらか分からないため、その場合は候補が表示されます。

```bash
sentinel entity show host/filesrv02      # ホストそのもの
sentinel entity show storage/filesrv02   # そのホストが提供するストレージ
```

```
systemd
  meaning    systemd units can be inspected here
  detected   the directory /run/systemd/system exists
  enables    systemd.unit

systemd.unit
  runs       systemctl show <unit> --property=ActiveState,SubState,Result
  needs      systemd
  where      on the host itself, by its agent
  cadence    every 10s, timeout 5s
```

```
compute02
  reached at   192.0.2.22
  from itself  host.metrics, systemd.unit
  from others  network.tcp, sentinel.agent, ssh.service
  watched by   node03 (SameDomain), fs02 (Independent), node01 (Filler)
```

`explain probes`のcadenceは設定を反映した値です。`[probes]`で
変更していれば、その値が出ます。停止しているprobeは`[DISABLED]`と表示されます。

### 診断の根拠となる観測結果を見る

stateやdiagnosisが実際の状況と一致しないときは、元の観測結果を確認します。

```bash
sudo -u sentinel sentinel entity observations <name>
sudo -u sentinel sentinel entity observations <name> --probe network.tcp --limit 100
```

```
WHEN                 PROBE               OBSERVER        STATUS        DETAIL
────────────────────────────────────────────────────────────────────────────
09-08 10:35:43       network.tcp         node02          ok            192.0.2.22:22 connected
09-08 10:35:42       network.tcp         head01          failed        no answer within 3s
```

observer列で観測元を確認します。「SSHは通るのに到達不能」のような一見矛盾した状態は、
たいてい観測者ごとに結果が違うだけで、この列を見れば矛盾ではなくなります。
`(itself)`はそのホストのagent自身による観測です。

すべて`--json`に対応しているため、スクリプトから利用できます。

`sentinel status`は異常があればexit code 2を返します。

## 診断結果の読み方

| 診断 | 意味 | 最初に見るもの |
| --- | --- | --- |
| `HOST_UNREACHABLE` | 複数の独立したobserverから到達不能 | console / BMC |
| `PATH_SPECIFIC_NETWORK_FAILURE` | 一部経路のみ異常。ホストは応答する | 両端のネットワーク |
| `SSH_SERVICE_FAILURE` | SSHのみ停止 | `systemctl status sshd` |
| `SENTINEL_AGENT_FAILURE` | 監視のみ停止。ホストは正常 | `systemctl status sentinel-agent` |
| `SLURMD_SERVICE_FAILURE` | ホストは正常、`slurmd`のみ | `journalctl -u slurmd` |
| `SLURM_ONLY_DEGRADATION` | ホストは完全に正常。Slurm上のみDRAIN | `scontrol show node <n>`のReason |
| `SLURM_CONTROL_PLANE_FAILURE` | 制御系自体 | `scontrol ping` |
| `NFS_SERVICE_FAILURE` | ファイルサーバーは稼働、exportサービスのみ | `exportfs -v` |
| `NFS_CLIENT_FAILURE` | 1クライアントのみ。サーバーは正常 | 当該クライアントのmount |
| `SHARED_STORAGE_FAILURE` | 同一ストレージの複数クライアント | ストレージを提供するホスト |
| `RESOURCE_CONFIGURATION_MISMATCH` | 設定と実ハードウェアの不一致 | `slurm.conf` |

### Sentinelが言わないこと

`POWER_OFF`は出力しません。ネットワーク応答がないことは電源状態の証拠ではありません。
電源断・NIC故障・switch障害はネットワークから見れば同じです。
`HOST_UNREACHABLE`は「複数の観測元から到達できない」と判定し、
その先はconsole / BMCを確認するよう促します。

observerが1台の場合、到達性の診断を行いません。
1観測元ではホスト全体の障害と経路障害を区別できないためです。
`sentinel peers`でobserverが付いているか確認してください。

## Notification

```toml
[notification]
min_severity = "warning"

[[notification.webhooks]]
name = "ntfy"
url = "https://ntfy.example.org/cluster-sentinel"
```

送信されるのは変化があったときだけです。

| 送信する | 送信しない |
| --- | --- |
| incidentの発生 | 継続中のincident（何度pollingしても） |
| severityの上昇 | 変化のない状態 |
| 診断内容の変化 | `min_severity`未満（ただし復旧は常に送る） |
| 復旧 / 解決 | maintenance中の対象 |

重複排除は宛先ごとに行われるため、
Slackを止めてもpagerは止まりません。
controllerとfallback notifierが同じincidentを検知しても、通知は1回です。

`RESOLVED`は、原因と影響を受けた対象の復旧を、新しい正常な観測で確認したときに送信します。観測の期限切れ、監視元の減少、`unknown`への遷移だけでは解決しません。原因の復旧を確認できても依存先が復旧していなければ、incidentは`recovering`になります。

controllerより5秒を超えて先の時刻を持つ観測は履歴に残しますが、状態判定には使いません。監視先と監視元の時計が同期していることも確認してください。

probeと監視元ごとに状態を判定するため、別のprobeの正常結果で異常が消えることはありません。観測の再送は判定回数に加算されず、判定途中の回数もcontrollerの再起動後に引き継がれます。

通知処理は観測の取り込みや診断とは別に実行し、遅い送信先が監視を止めないようにしています。通知候補はincidentと同時に保存します。送信に失敗した通知は再起動後も再送し、同じ宛先へ配信済みの通知は繰り返しません。障害が再発した場合は再発と再復旧をそれぞれ通知し、前回の未配信の復旧通知は取り消します。

### 実機で通知経路を確かめる

宛先を設定したあと、障害を待たずに届くかどうかを確認できます。
Slack宛は、webhookの設定を`format = "slack"`にしてください。

```bash
sudo -u sentinel sentinel notify test
sudo -u sentinel sentinel notify test --provider ops        # 宛先を絞る
sudo -u sentinel sentinel notify test --severity critical   # 重大度を変えて経路を試す
```

```
ops                  送信成功
broken               送信失敗　通知先http://...に接続できません。詳細　...

2か所の通知先のうち、1か所で送信に失敗しました。
```

incident、database、重複排除の記録は変更しません。
controllerから設定済みの宛先へ送信できるかを確認します。
障害が起きる前に実行し、URLや到達性の問題を確認してください。

送られる内容は、人が見てもフィルタが見てもテストと分かるようにしてあります。
`fingerprint`は`sentinel-test-notification`固定で、実際のincidentと
衝突しません（衝突すれば通常の障害通知を抑止してしまいます）。

テスト通知は`min_severity`未満でも送ります。
障害の重大度に関係なく、宛先への疎通を確認するためです。
Slackへ`--severity critical`でテスト通知を送ると、赤の表示と冒頭の`@channel`も確認できます。このテストでもチャンネル全体へのメンションが付きます。

### Slackの通知を受けたら

通知は色バー付きの本文にまとめて表示します。確認手順は、実行先、確認する内容、コマンドを改行して表示します。
診断の説明と障害IDなどの情報は段落を分けています。確認コマンドはコードブロックからコピーできます。
赤のCRITICAL通知には、冒頭に`@channel`が付きます。

通知の冒頭で、障害発生・診断更新・復旧途中・復旧確認のどれかを確認してください。復旧途中は、影響を受けた対象すべての復旧をまだ確認できていない状態です。

確認手順の最初にあるコマンドをSentinel controllerで実行し、原因の候補、影響を受けた対象、観測時刻を確認してください。その後は、手順に書かれたホストで、サービスやログを調べてください。通知には状態や設定を変更するコマンドを載せていません。

```bash
sudo -u sentinel sentinel incident show <障害ID>
```

Slurmのノードを調べるコマンドには、ホスト名と異なる場合も登録されたNodeNameを使います。NFSのI/O検査が完了しない場合は、マウント先へのアクセスを繰り返さず、マウント情報、カーネルログ、待機中のプロセスを確認してください。復旧確認の通知では、発生時の診断と復旧履歴を確認できます。

> 宛先が受け取ったことと、人が気づくことは別です。
> 実際にメッセージが届いているかは受信側で確認してください。

### 全経路を確かめる

通知経路だけでなく、probe → 診断 → incident → 通知の全体を試すなら、
最も影響の小さい実障害を起こします。

```bash
ssh <node> sudo systemctl stop sentinel-agent
# 数十秒待つ -> SENTINEL_AGENT_FAILUREとして通知されるはず
ssh <node> sudo systemctl start sentinel-agent
# 復旧通知（resolved: true）が届くはず
```

agentを止めてもジョブには影響しません。その間そのノードのローカル観測が
止まるだけです。本番で試せる障害はこれが上限だと考えてください。

## 計画作業中に通知を止める（maintenance window）

ディスク換装、OS入れ替え、電源工事などの計画停止に使います。
maintenance windowを設定せずに停止すると、通常の障害として通知されます。

### 作業を始めるとき

```bash
sudo -u sentinel sentinel maintenance start filesrv01 --reason "HDD 換装" --for 6h
```

これだけです。対象ノードについての通知が止まります。

`--for`は省略できます。省略すると期限なしになり、
`sentinel maintenance end`を打つまで続きます。
作業がいつ終わるか分からないなら省略して構いませんが、
解除を忘れると通常の障害通知も止まります。
不安なら長めの`--for 12h`を付けておくのが安全です。

クラスタ全体を止める場合（全体電源工事など）は、対象を省略します。

```bash
sudo -u sentinel sentinel maintenance start --reason "電源設備の点検"
```

### 作業が終わったら

```bash
sudo -u sentinel sentinel maintenance end <id>
```

`id`は`start`が表示したものです。全部打つ必要はなく、先頭数文字で足ります。
忘れたら`sentinel maintenance list`で出ます。

解除した次の診断サイクルから通知が再開されます。
作業後も障害が続いていれば、通知が届きます。作業中も検査と判定は継続しています。

### 今抑止されているものを確認する

```bash
sudo -u sentinel sentinel maintenance list
```

`sentinel status`の末尾にも出ます。

```
⚠ notifications suppressed by maintenance: filesrv01
  probing and diagnosis continue; end it with: sentinel maintenance end <id>
```

この行が出ているかを確認してください。
CRITICALのincidentがあるのに通知が届かない場合、送信失敗のほかに
maintenance windowの解除忘れも考えられます。この表示で通知の抑止を確認できます。

### 抑止されるのは通知だけ

probeは動き続け、stateもdiagnosisも更新され続けます。
異常な状態がhealthyに書き換えられることはありません。
`status`にも`incident list`にも、作業中の異常はそのまま出ます。

作業中に別の障害が起きた場合も、発生時刻や観測結果を後から確認できるようにするためです。
通知を止めるためにcontrollerを停止すると、停止中の診断を記録できません。

### 通知を止める範囲

incidentが抑止されるのは、
影響を受けているentityがすべてmaintenance対象である場合のみです。

filesrv01の作業中に、無関係な計算ノード4台が停止した場合、その障害は通知されます。
共有ストレージの障害がfilesrv01以外にも影響する場合は、
filesrv01だけのmaintenanceを設定しても通知を止めません。
作業対象外のホストに起きた異常を知らせるためです。

## トラブルシューティング

### agentが登録されない

```bash
sentinel doctor                          # credential が設定されているか
systemctl status sentinel-agent
journalctl -u sentinel-agent -n 50
curl -fsS http://<controller>:7443/v1/health   # controller は生きているか
```

`Credential: NOT CONFIGURED`と出る場合、`/etc/sentinel/token`が
サービスユーザーから読めていません。

### 監視されていないホストがある

capabilityが付いていない可能性があります。

```bash
sentinel entity show <host>   # Capabilities を確認
sentinel doctor               # その host で、各 capability の判定理由を表示
```

`sentinel doctor`はcapabilityごとに
「検出された / このホストには無い / 設定で有効化 / 役割による推定」
のいずれかを表示します。

必要であれば設定で明示的に上書きできます。

```toml
[capabilities]
"storage.nfs.server" = "force"
```

### 到達性の診断が出ない

observerが不足しています。

```bash
sentinel peers    # observer の付いていない entity を表示する
```

`observer.peer` capabilityを持つホストを増やしてください。

### controllerが停止した場合

agentは観測とspoolを継続します。
controller復旧後、spoolは自動で再送されます（重複挿入は起きません）。

```bash
sentinel doctor   # spool の深さを確認
```

### agentが起動直後に終了を繰り返す

`systemctl status`が再起動ループだけを示し、理由が書かれていない場合、
設定ファイルを`sentinel`ユーザーが読めていないことが多いです。
`install`は設定をmode 0640で書くため、root所有のままだと
サービスから開けません。

```bash
sudo chown sentinel:sentinel /etc/sentinel/*.toml && sudo systemctl restart sentinel-agent
```

（サービスユーザーが既にあるホストでは`install`が自動で所有者を
設定するようになっています。古いバージョンで作られたファイルだけ
手当てが必要です。）

### 通知が来ない

まずmaintenance windowを疑ってください。`sentinel status`の末尾に
`notifications suppressed by maintenance`が出ていれば原因はそれで、
`sentinel maintenance list`で誰がいつ何のために宣言したかが分かります。
期限なしで宣言されたものは、解除するまで残り続けます。

まず宛先そのものを試します。障害を待つ必要はありません。

```bash
sentinel notify test
```

宛先ごとに1通だけ送られます。届かない場合はURLか到達性の問題です。

届くのに障害通知が来ない場合、確認する順に

1. `sentinel incident list`、そもそもincidentになっているか。
   診断は出ていても、severityが`min_severity`を下回っていれば送られません
2. `[notification] min_severity`、導入初期に`"critical"`にしていないか
3. maintenance windowに入っていないか

通知は状態が変わったときだけ飛びます。続いているincidentを
繰り返し通知することはありません。これは仕様です。

### 通知が一度に大量に来る

incident 1件につき1通です。多数届いたのなら、多数のincidentが
同時に開いたということです。webhookを初めて設定した直後は、
それまでに開いていたincidentがまとめて送られます（一度だけ）。

送信間隔は既定で1秒空きます。間引きではなく間隔をあけるので、
通知が捨てられることはありません。webhook側がより厳しいなら伸ばせます。

```toml
[notification]
min_interval = "3s"
```

### ストレージの健全性がUNKNOWNのまま

そのストレージを提供しているホストのexport検査が動いていません。

```bash
sentinel entity observations <fileserver> --probe nfs.server.exports
```

観測が無い場合、そのホストに`storage.nfs.server` capabilityが
付いていない可能性があります。

```bash
sentinel entity show host/<fileserver>
```

capabilityは`/etc/exports`の存在か`exportfs`の有無で判定されます。
ZFSの`sharenfs`でexportしている場合も、exportの実体は
`/etc/exports.d/*.exports`にあり、そちらも読まれます。

### NFSを提供していないホストにNFS障害が出る

exportしていないホストは、exportゼロでも障害になりません。
障害として報告されるのは、2049番で何かがlistenしているのに
exportが空の場合だけです（クライアントが接続できて拒否される状態）。

これに該当しないのに報告されるなら、その判定はv1.0より前の挙動です。

## ディスク容量と保持期間

controllerのdatabaseは書き込み一方です。何も消さなければ埋まります。
実測値はホスト1台あたり1日約170 MB（5ホストのテストベッドで約10 KB/s）。
100ノードなら1日17 GB、1か月500 GBです。

既定でpruneは有効（`[retention]`、observation 14日）なので、
ホストあたり約2.4 GBで頭打ちになります。設定は
[`CONFIGURATION.md`](CONFIGURATION.md) の`[retention]`を参照してください。

pruneはcontroller内で1時間ごと、および起動時に実行されます。
手動でも実行できます。

```bash
sentinel prune --dry-run                  # 何が消えるかだけ見る
sentinel prune                            # 実行
sentinel prune --vacuum                   # 実行し、領域を filesystem に返す
sentinel prune --observations 3d --vacuum # この 1 回だけ保持期間を短くする
```

`--dry-run`の件数は実際のDELETEを実行してrollbackしたものです。
別のCOUNTクエリではないため、本番と食い違うことはありません。

pruneはファイルサイズを縮めません。空きページは新しい記録に再利用されるため
増加は止まりますが、領域をOSに返すには`--vacuum`が必要です
（database全体を書き直すため自動では実行しません）。

数か月分が溜まったdatabaseに対する最初の1回は時間がかかり、
その間write lockを保持します。agentはspoolで耐えますが、
`sentinel prune`を使って任意のタイミングで実施することを推奨します。

削除されないもの: openなincident（年齢に関わらず）、
保存されているincident / diagnosisが参照しているobservation、
各entityの直近`keep_per_entity`件。

## バックアップ

```bash
systemctl stop sentinel-controller
cp /var/lib/sentinel/sentinel.db /path/to/backup/
systemctl start sentinel-controller
```

WAL modeのため、稼働中のコピーは`sqlite3 .backup`を使用してください。

```bash
sqlite3 /var/lib/sentinel/sentinel.db ".backup /path/to/backup/sentinel.db"
```

## アップグレード

controllerを先に、agentを後に。バイナリversionとprotocol versionは
分離されているので、同一protocol versionであれば混在状態でも動きます。

新しいバイナリを置いて再起動する、それだけです。設定・database・credential
はそのまま残ります。databaseのマイグレーションはcontrollerの起動時に自動で
適用されます。

### 更新スクリプトを使う

中央ノードで、普段Ansibleを実行するユーザーから
[`deploy/update.sh`](../deploy/update.sh)を実行してください。
中央ノードを更新し、サービスの起動と状態の読み取りを確認してから、
既存のAnsibleインベントリにある下流ノードを更新します。

```bash
cd /path/to/cluster-sentinel
git pull --ff-only
./deploy/update.sh v1.0.6 -- -K
```

次のリリースへ更新するときは、`v1.0.6`を更新先のバージョンに変えます。
スクリプトはリポジトリを自動では更新しません。
バイナリの取得元は`mizuno-group/cluster-sentinel`です。
中央ノードでは`/usr/local/bin/sentinel`と`/etc/sentinel/config.toml`を使います。
配置を変えている場合は、後述の手動手順を使ってください。

Vaultを使っている場合やSSHのパスワードが必要な場合は、
`--`の後に普段のAnsibleオプションを付けます。
スクリプト全体には`sudo`を付けないでください。SSH接続には実行したユーザーの設定を使います。

```bash
./deploy/update.sh v1.0.6 -- --ask-vault-pass -K
./deploy/update.sh v1.0.6 -- -k -K
```

中央と下流を別々に更新することもできます。
`--agents-only`は、中央ノードに指定したバージョンが入っており、
サービスが起動していることを確認してから実行します。

```bash
./deploy/update.sh v1.0.6 --parent-only
./deploy/update.sh v1.0.6 --agents-only -- -K
./deploy/update.sh v1.0.6 --inventory /path/to/inventory.ini -- -K --limit node01
```

中央ノードでは、ダウンロードしたファイルのSHA-256、バージョン、
既存の設定を検証してからバイナリを置き換えます。
前のバイナリは`/usr/local/bin/sentinel.previous`に保存します。
設定・DB・認証情報を置き換える処理は行いません。
中央ノードで`sentinel-agent`も起動中なら、そのサービスも再起動します。
同じバイナリが入っている場合も、途中で中断した更新をやり直せるようにサービスを再起動します。

下流ノードの設定と認証情報は、既存のAnsibleロールが通常どおり配布します。
ノードの設定を変更するときは、Ansibleインベントリやgroup_varsを編集してください。
中央ノードの確認に失敗した場合は、下流の更新を開始しません。
下流の更新中に失敗した場合は、原因を直して`--agents-only`でやり直せます。

`sentinel status`の終了コード2は、監視対象の異常を示します。
スクリプトはその内容を表示して更新を続けます。
状態を読み取れない場合は、更新を中止します。

### 1. controller

```bash
ARCH=$(uname -m)
curl -fsSLO "https://github.com/mizuno-group/cluster-sentinel/releases/latest/download/sentinel-${ARCH}-unknown-linux-musl"
curl -fsSL "https://github.com/mizuno-group/cluster-sentinel/releases/latest/download/sentinel-${ARCH}-unknown-linux-musl.sha256" | sha256sum -c
```

置き換える前に、今の設定が新しいバイナリで通ることを確認します。

```bash
chmod +x sentinel-*-unknown-linux-musl && sudo -u sentinel ./sentinel-*-unknown-linux-musl --config /etc/sentinel/config.toml config check
```

```bash
sudo install -o root -g root -m 0755 sentinel-*-unknown-linux-musl /usr/local/bin/sentinel.new
sudo mv -fT /usr/local/bin/sentinel.new /usr/local/bin/sentinel
sudo systemctl restart sentinel-controller
```

`install`は別名のファイルへ書き込みます。
その後、同じディレクトリ内で`mv`してバイナリを置き換えます。
実行中のファイルに直接書き込むと、`Text file busy`で失敗します。

```bash
sudo -u sentinel sentinel version && systemctl status sentinel-controller --no-pager
```

controllerとagentが同居しているホスト（[DEPLOYMENT.md §9.10](DEPLOYMENT.md#910-controllerにagentを同居させる)）
では、バイナリは1つなので両方を再起動します。

```bash
sudo systemctl restart sentinel-controller sentinel-agent
```

### 2. agent（各ノード）

controllerが動いていることを確認してから、同じ手順を各ノードで行います。
ノードが多い場合は [Ansible ロール](../deploy/ansible/)
の`sentinel_version`を変えて再実行してください。

ロールの`sentinel_version`はリリースごとに更新されているので、
まず`git pull`してから実行するのが確実です。指定が古いままだと
「もう入っている」と判断されて`changed=0`で終わります。

```bash
git -C <このリポジトリ> pull && ansible-playbook -i inventory.ini site.yml -K
```

一度きり別のバージョンにしたい場合は、`v`を付けて指定します
（タグ名がそのままURLに入るので、`v`を落とすと404になります）。

```bash
ansible-playbook -i inventory.ini site.yml -K -e sentinel_version=v1.0.0
```

agentの停止中は新しいローカル観測を収集しません。停止前にspoolに保存した未送信の観測は、再起動後に送られます。

### systemd unitが更新された場合

リリースノートにunitの変更が書かれている場合のみ必要です。

```bash
sudo sentinel install controller --force    # または agent
sudo systemctl daemon-reload
sudo systemctl restart sentinel-controller
```

`--force`は設定ファイルとunitを上書きします。設定を手で調整している
場合は、先に控えを取ってください。

```bash
sudo cp /etc/sentinel/config.toml /etc/sentinel/config.toml.bak
```

credentialは`--force`でも上書きされません。入れ替えると、既存のagentが認証できなくなるためです。
意図的に更新する場合は、
ファイルを削除してから`install`を実行し、全ホストに配り直してください。

### 切り戻し

前のバイナリに戻して再起動するだけです。databaseスキーマは後方互換であり、
新しいバージョンのマイグレーションを適用したDBも、古いバイナリで読み取れます。

```bash
sudo install -o root -g root -m 0755 /path/to/previous/sentinel /usr/local/bin/sentinel.new
sudo mv -fT /usr/local/bin/sentinel.new /usr/local/bin/sentinel
sudo systemctl restart sentinel-controller
```

更新スクリプトで保存したバイナリを使う場合は、
`/path/to/previous/sentinel`を`/usr/local/bin/sentinel.previous`に置き換えます。
中央ノードでagentも動かしている場合は、`sentinel-agent`も再起動してください。

## v1で行わないこと

* 自動復旧（再起動 / restart / remount / `scontrol update`）
* LLMによる診断
* controllerのHA
* Prometheus / Grafanaの置き換え
* per-node credential（cluster共有token + mutual TLSまでがv1）
* 遠隔からの読み取りAPI（参照CLIはcontroller上で実行する必要があります）

いずれもcore architectureを変更せずに追加できる設計にしてありますが、
v1の範囲外です。
