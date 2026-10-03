# 設定リファレンス

SentinelはTOMLファイルを1つ読み込みます。既定値は
`/etc/sentinel/config.toml`（`--config`、または環境変数`SENTINEL_CONFIG`で変更可）。

設定を変更したら、サービスを再起動する前に検証してください。

```bash
sentinel config check
```

最初の1件で止まらず、検出できた問題をすべて報告します。
設定が使用不能な場合は非0で終了します。
`sentinel config show`は実効設定を出力します。

## 設定ファイルの作り方

手で書き起こす必要はありません。

```bash
sentinel config init --role controller --output /etc/sentinel/config.toml
sentinel config init --role agent      --output /etc/sentinel/config.toml
```

全設定が既定値のまま、説明つきで書き出されます。
書き換えが必要なのは`CHANGE-ME`を含む行だけです。

`sentinel install`は設定ファイル・systemd unit・credentialをまとめて生成します。
詳細は [DEPLOYMENT.md](DEPLOYMENT.md) を参照してください。

## 優先順位

```text
CLI  >  環境変数  >  設定ファイル  >  runtime discovery  >  built-in default
```

`sentinel config show`では、適用された値と、その値を指定した場所を確認できます。

## `config_version`

必須。本ビルドが理解するのはversion `1`です。

未対応のversionを指定した設定ファイルは拒否します。
一部の設定だけを適用すると、指定したとおりに監視できなくなるためです。

```toml
config_version = 1
```

## 最小のagent設定

```toml
config_version = 1
environment = "example-lab"

[agent]
controller_address = "controller.example:7443"
```

controllerアドレスに既定値はありません。
バイナリへホスト名を埋め込むことはしません。

## 最小のcontroller設定

```toml
config_version = 1
environment = "example-lab"

[controller]
listen = "0.0.0.0:7443"

[database]
path = "/var/lib/sentinel/sentinel.db"

[peer_monitoring]
degree = 3

[discovery.slurm]
enabled = true
```

## セクション

### `[controller]`

| キー | 型 | 既定値 | 意味 |
| --- | --- | --- | --- |
| `listen` | `host:port` | `0.0.0.0:7443` | controllerの待受アドレス |
| `inventory_interval` | duration | `5m` | 監視対象一覧自動検出の実行間隔（`scontrol`実行・全ホストprobeを伴う） |
| `diagnosis_interval` | duration | `15s` | 診断・相関・通知の実行間隔 |
| `observe` | bool | `true` | controller自身も観測点として動作するか |

`diagnosis_interval`は障害発生から通知までの遅延を決める値です。
保存済みデータを使う診断と、外部コマンドやprobeを実行する監視対象の自動検出を分けています。

### `[agent]`

| キー | 型 | 既定値 | 意味 |
| --- | --- | --- | --- |
| `controller_address` | `host:port` | *(なし)* | 報告先controller |
| `spool_path` | path | `<state dir>/spool.db` | ローカルobservation spool |
| `roles` | 文字列配列 | `[]` | グループ分けと既定値提示のみに使うラベル |
| `listen` | `host:port` | `0.0.0.0:7444` | health endpointの待受。peerがここを見る |
| `ssh_port` | 整数 | *(sshd_configから検出)* | SSHポート。自動検出できない場合のみ |

`ssh_port`の優先順位

```text
[agent] ssh_port  >  /etc/ssh/sshd_config の Port  >  既定値 22
```

通常はagentが`sshd_config`を読んで検出し、controllerへ報告するため、
指定は不要です。`listen`を変更した場合もagentが自分で報告するため、
controller側への追記は必要ありません。

`roles`はprobeを有効化しません。後述の「Capability」を参照してください。

#### アドレスの決まり方

peerがこのホストをprobeするアドレスは、次の順で決まります。

| 優先 | 設定 | 用途 |
| --- | --- | --- |
| 1 | `[agent] address` | アドレスを直接指定。NAT越しなど、ホスト自身から見えない場合 |
| 2 | `[agent] interface` | NIC名で指定。全ノードで同じ1行が使えるので推奨 |
| 3 | 自動検出 | 下記の規則で順位付け |

自動検出は次を行います。

1. 到達不能なものを除外、loopbackアドレス、link-local（`169.254.0.0/16`、
   `fe80::/10`）、および`lo`インターフェース上の全アドレス。
   `lo`に付いた非loopbackアドレス（WSLの`10.255.255.254/32`など）は
   誰からも到達できません
2. 物理NICを仮想NICより優先、`docker*` / `br-*` / `veth*` / `virbr*` /
   `wg*` / `tailscale*`などは後ろに回します
3. IPv4をIPv6より優先し、以降は名前順（再起動しても順序が変わらないように）

自動検出は「どのNICがクラスタ内通信を担っているか」を答えられません。
使用するNICはクラスタの構成で決まるため、ホストの情報だけでは判別できません。
`vlan101` / `vlan102` / `vlan103`を持つホストでは、どれも同じくらい妥当に見えます。

そのため、物理NICの候補が2つ以上ある場合は「曖昧である」と報告します。
確認せずに一つ選ぶと、間違っていても気づけないためです。
`sentinel doctor`が候補一覧とともに表示します。

```
Address:     192.0.2.10
  -> vlan101           192.0.2.10
     vlan102           192.0.2.20
     vlan103           192.0.2.32
     wg0              10.0.0.1
  ! several interfaces could be the one peers reach this host on
    (vlan101, vlan102, vlan103); 192.0.2.10 was chosen by name order.
    Set [agent] interface to say which.
```

`interface`で指定したNICに使えるアドレスが無い場合、
アドレスを報告しません（別のNICにフォールバックしません）。
運用者が選ばなかったネットワークにpeerを向けるのが、この設定で防ぎたい障害だからです。
その場合controllerはホスト名にフォールバックし、`doctor`が理由を表示します。

### `[database]`

| キー | 型 | 既定値 |
| --- | --- | --- |
| `path` | path | `/var/lib/sentinel/sentinel.db` |

### `[probes]`

監視頻度をprobeごとに変更します。probe idをキーにしたテーブルです。

```toml
[probes."network.tcp"]
interval = "10s"

[probes."gpu.nvidia"]
enabled = false
```

| キー | 型 | 意味 |
| --- | --- | --- |
| `interval` | duration | 実行間隔 |
| `timeout` | duration | 1回あたりの上限時間 |
| `max_outstanding` | 整数 | 同一targetへの同時実行数（引き下げのみ可能） |
| `enabled` | bool | `false`でそのprobeを停止 |

書かれていないprobeは既定のまま動きます。1つだけ調整しても他には影響しません。
未設定のキーも同様に既定値のままです。

既定値（`sentinel config init`が生成する設定ファイルにも全件書き出されます）

| Probe | interval | タイムアウト | 備考 |
| --- | --- | --- | --- |
| `network.tcp` | 5s | 3s | ホストへの到達性を確認 |
| `sentinel.agent` | 5s | 3s | remoteのみ |
| `systemd.unit` | 10s | 5s | |
| `host.metrics` | 15s | 5s | |
| `ssh.service` | 15s | 5s | |
| `gpu.nvidia` | 15s | 10s | |
| `nfs.server.port` | 15s | 5s | |
| `nfs.client.mount` | 30s | 5s | `/proc`のみ |
| `nfs.client.io` | 30s | 10s | 同時実行1（固定） |
| `journal.events` | 30s | 10s | 同時実行1（固定） |
| `nfs.server.exports` | 60s | 5s | |

`max_outstanding`は引き下げしかできません。
`nfs.client.io`と`journal.events`は1に固定されています。
blockingシステムコールが積み上がらないようにするためであり
（`SPEC.md` §76、設計原則10）、設定ファイルで覆せません。

存在しないprobe idを書くとerrorになります。
警告なしに無視されると「変更したつもりで変わっていない」状態になるためです。

設定の上書きはcontrollerのremote probeとagentのpeer probeにも同じく適用されます。
観測者ごとに頻度が違うと、quorumが異なる頻度の観測を比較することになるためです。

### `[notification]`

| キー | 型 | 既定値 | 意味 |
| --- | --- | --- | --- |
| `min_severity` | 文字列 | `warning` | これ未満は送らない（復旧通知は常に送る） |
| `min_interval` | duration | `1s` | 同一宛先への送信間隔の下限 |

severityは診断の種類と依存先の数に応じて決まります。
設定の不一致や時刻ずれは`info`のため、既定では通知しません。
ノードをdrainした場合は、依存する`slurmd`サービスへの影響があるため、
`warning`として通知します。

drainの通知を受け取らない場合は`min_severity = "critical"`にしてください。
`warning`以上の通知を受け取る場合は、既定値を使います。

`min_interval`は、通知を省略せずに送信間隔を空ける設定です。
一つの障害が複数の依存先へ影響すると、同時に複数の通知が必要になります。
webhookの送信頻度制限を守りながら、必要な通知をすべて送るために使います。

#### `[[notification.webhooks]]`

| キー | 型 | 既定値 | 意味 |
| --- | --- | --- | --- |
| `name` | 文字列 | *(必須)* | ログと重複排除に使う名前 |
| `url` | 文字列 | *(必須)* | POST先 |
| `format` | `generic` / `slack` | `generic` | ペイロードの形 |

#### `format = "slack"`

Slack Block Kitで送ります。日本語の見出しと異常の説明に続けて、確認手順を番号付きで表示します。各手順には、実行するホスト、確認する項目、コマンドを載せます。

```toml
[[notification.webhooks]]
name   = "ops"
url    = "https://hooks.slack.com/services/..."
format = "slack"
```

| 状態 | 色 | 見出しの表示 |
| --- | --- | --- |
| `resolved` | 緑 | 復旧確認 |
| `recovering` | 黄 | 復旧途中 |
| `critical` | 赤 | 重大 |
| `warning` | 黄 | 警告 |
| `info` | 灰 | 情報 |

赤のCRITICAL通知では、冒頭に`@channel`を付けます。障害の発生、重大度の上昇、診断の更新が対象です。Slackのメンションとして送るため、チャンネル全体への通知になります。復旧途中と復旧完了は、障害時の重大度に関係なく黄と緑で表示し、`@channel`は付けません。

メンションは、Slackの[通知の書式](https://docs.slack.dev/messaging/formatting-message-text/#special-mentions)に従って送ります。ホスト名やSlurmのReasonにメンションの文字列が含まれていても、追加のメンションとして扱いません。

長い確認手順は、コマンドを途中で切らずに複数の表示欄へ分けます。表示上限を超える場合は、省略したことと全文を確認するコマンドを表示します。Sentinelのコマンドは、controllerで`sudo -u sentinel sentinel ...`として実行してください。

#### 送信される内容（`format = "generic"`）

`Content-Type: application/json`のPOSTです。

```json
{
  "source": "cluster-sentinel",
  "incident_id": "...",
  "fingerprint": "...",
  "trigger": "opened",
  "severity": "critical",
  "resolved": false,
  "title": "...",
  "body": "...",
  "recommended_actions": ["..."],
  "timestamp": "...",
  "text": "...",
  "content": "..."
}
```

`resolved`は復旧完了時だけ`true`です。復旧途中では`trigger`が`recovering`、`resolved`が`false`になります。

`text`と`content`には、全体を読める文章にしたものが入ります。
Slack / Microsoft Teamsは`text`を、Discordは`content`を要求するため、
URLを書くだけで動きます（無いとSlackは
`missing_text_or_fallback_or_attachments`で400を返します）。

構造化されたフィールドはそのまま残っているので、
severityで振り分けるような受け手はそちらを使ってください。

### `[retention]`

記録したデータをどれだけ保持するかです。

| キー | 型 | 既定値 | 意味 |
| --- | --- | --- | --- |
| `enabled` | bool | `true` | pruneを行うか |
| `interval` | duration | `1h` | pruneの実行間隔（起動時にも1回実行） |
| `observations` | period | `14d` | observationの保持期間 |
| `keep_per_entity` | 整数 | `64` | 期間に関わらずentityごとに残すobservation数 |
| `transitions` | duration | `90d` | state transitionの保持期間 |
| `resolved_incidents` | period | `180d` | 解決済みincidentの保持期間 |
| `diagnoses` | period | `30d` | どのincidentにも属さないdiagnosisの保持期間 |

periodにはduration（`"14d"`、`"6h"`）のほか、
`"never"`（`"forever"` / `"unlimited"` / `"keep"`も同義）を指定できます。

記録の用途に応じて保持期間を分けています。
容量の大半を占めるobservationは短期間で削除し、
障害の再発や過去の対応を調べるincidentは長期間保持します。

削除されないもの（SQLで強制、設定で緩められません）

* 未解決のincidentは、経過時間に関係なく残します
* 保存されているincident / diagnosisが参照するobservationは、保持期間を過ぎても残します。
  診断の根拠を後から確認できるようにするためです
* 各entityの直近`keep_per_entity`件のobservationを残します。
  保持期間より長い障害でも、最後に取得した観測結果を確認できるようにするためです

`keep_per_entity`は診断が読む件数（32）を下回れません。
下回る値を設定した場合は32に引き上げられ、`config check`が警告します。

容量の目安。実測で5ホストあたり約10 KB/s、ホスト1台あたり
1日約170 MBです。既定の14日保持ならホストあたり約2.4 GBで頭打ちになります。

pruneは空きページを再利用可能にしますが、ファイルサイズは縮みません。
保持期間を下げた直後に領域を返したい場合は`sentinel prune --vacuum`を使います。

### `[tls]`

controller APIとの通信をTLSで暗号化するための任意設定です。
何も設定しなければ従来どおり平文HTTPで動作します。

controller側（listener）

| キー | 型 | 意味 |
| --- | --- | --- |
| `cert` | path | サーバー証明書チェーン（PEM）。設定するとTLSが有効になる |
| `key` | path | サーバー秘密鍵（PEM: PKCS#8 / PKCS#1 / SEC1） |
| `client_ca` | path | クライアント証明書を検証するCA（PEM）。設定するとクライアント証明書は必須になります |

agent側（クライアント）

| キー | 型 | 意味 |
| --- | --- | --- |
| `ca` | path | controllerの証明書を検証するCA（PEM）。system rootに追加されます |
| `client_cert` | path | controllerに提示するクライアント証明書（PEM） |
| `client_key` | path | `client_cert`の秘密鍵（PEM） |
| `server_name` | 文字列 | 証明書の検証に使う名前。IPで接続する場合に使う |
| `insecure_skip_verify` | bool | 証明書を検証しない（既定`false`） |

3つの構成

1. 設定なしの場合は平文HTTP。隔離された管理ネットワークで使う構成です
2. `cert` + `key`、TLS。agentはsystem rootか`ca`で検証します
3. `cert` + `key` + `client_ca`、mutual TLS。agentは証明書を提示しなければ
   tokenを使った認証に進めません。tokenが漏れても耐えられる構成はこれだけです

`client_ca`を設定した時点でクライアント証明書は「任意」ではなく「必須」です。
証明書の提示を任意にすると、証明書を持たない攻撃者も接続できるためです。

`insecure_skip_verify = true`はTLSで接続先の正当性を確認できなくなります。接続を横取りできる
攻撃者は任意の証明書を提示でき、cluster credentialはそのまま読まれます。
証明書発行の準備が整う前にクラスタを起動する場合に使えますが、
起動のたびに警告が出ます。

証明書の自動生成機能はありません。CAの管理と監査はSentinelとは別に行い、
既存の証明書発行手順を使ってください。

### `[peer_monitoring]`

| キー | 型 | 既定値 | 意味 |
| --- | --- | --- | --- |
| `degree` | 整数 | `3` | 1 entityあたりに割り当てるobserver数 |

`degree = 0`はpeer monitoringを無効化し、警告されます。
第2の観測元が無い場合、controller自身の通信経路上の障害と
ホスト全体の障害を区別できなくなるためです。

### `[discovery.slurm]`

| キー | 型 | 既定値 | 意味 |
| --- | --- | --- | --- |
| `enabled` | bool | `false` | Slurm自動検出を実行するか |
| `scontrol_path` | path | *(`PATH`から解決)* | `scontrol`の場所 |

Slurmは、監視対象一覧を作るための情報源の一つです。
Slurmに登録されていないホストも、同じ仕組みで登録・監視できます。

### `[discovery.nfs]`

| キー | 型 | 既定値 | 意味 |
| --- | --- | --- | --- |
| `enabled` | bool | `true` | NFSの依存関係をagentの報告から導出するか |

既定で有効です。agentが報告するマウント表から、ファイルサーバーの
host entity、storage entity、`provides`、`uses_storage`が自動で作られます。
`[[dependencies]]`にNFSのマウント関係を書く必要はありません。

導出された内容は`sentinel dependency list`で確認できます。

アドレスでマウントされていて、そのアドレスを持つホストが未知の場合、
entityを作成せず、`sentinel discover`で報告します
（アドレスはidentityではないため。[ADR 0001](adr/0001-deterministic-entity-identity.md)）。

手で書いた宣言は導出結果と併存します。打ち消し合いません。
詳細と例外は [DEPLOYMENT.md §9.9](DEPLOYMENT.md#99-ストレージ構成は書かなくてよい例外は3つ)。

### `[capabilities]`

Capability名をキーとする運用者による上書き。

```toml
[capabilities]
"storage.nfs.server" = "force"    # discovery の結果によらず ON
"storage.smart"      = "disable"  # discovery の結果によらず OFF
"storage.zfs"        = "enable"   # discovery が何も言わなかった場合に ON
```

解決順序（強い順）

```text
disable  >  force  >  runtime discovery  >  enable / role hint
```

### `[[entities]]`

まだagentが入っていない、あるいはどの連携機能も発見しないentityを宣言します。

```toml
[[entities]]
type = "host"
name = "fileserver-a"
labels = { rack = "r01", location = "entrance" }
capabilities = ["storage.nfs.server", "observer.peer"]
addresses = ["10.0.0.10"]

[[entities]]
type = "storage"
name = "shared-a"
```

`type`は`host` / `service` / `storage` / `scheduler` / `external_dependency`のいずれか。

`name`は正式名であり、`(environment, type)`内で一意です。
アドレスとポートは接続先を示す情報で、entityの識別キーには使いません。
一つのentityが複数のアドレスを持てます。ポートを変更してもentityの識別子は変わりません。

`display_name`は表示にのみ使う別名です。省略すると`name`が使われます。
`cluster`は複数クラスタを1つのenvironmentで扱うときのグループ名で、
どちらもprobeを有効にしません（capabilityだけがprobeを決めます）。

`ports`はagentがいないホストに必要です。
agentがいるホストは自分でポートを報告します。

```toml
[[entities]]
type = "host"
name = "fileserver-a"
addresses = ["192.0.2.10"]
ports = { ssh = 2222, agent = 9444 }
```

指定しない場合は既定値（ssh 22 / agent 7444 / nfs 2049）が使われます。
SSHを22以外で運用しているクラスタでこれを書き忘れると、
probeが実際のSSHポートへ接続できず、正常なSSHサービスを障害として報告します。

### `[[dependencies]]`

> NFSについては書く必要がありません。マウント関係はagentの報告から
> 自動で導出されます（`[discovery.nfs]`）。ここに書くのは、NFS以外の
> 共有ストレージや、導出できなかったものだけです。

`from`が`to`に依存します。両者とも`type/name`形式で記述します。

```toml
[[dependencies]]
from = "host/compute-a"
to = "storage/shared-a"
type = "uses_storage"
criticality = "critical"

[[dependencies]]
from = "storage/shared-a"
to = "host/fileserver-a"
type = "provides"
```

`type`は`depends_on` / `hosted_on` / `provides` / `uses_storage` /
`uses_scheduler` / `network_reaches` / `observes`、
または連携機能固有の任意文字列。

`criticality`は`critical` / `important` / `optional`。
`optional`のedgeは障害を伝播しません。

ここで宣言していないentityを参照することはerrorではなくwarningです。
Slurm自動検出やagent registrationから正当に到着し得るためです。

閉路は許可されます。実際の依存グラフには閉路が存在します。

## 既定パス

| パス | 内容 |
| --- | --- |
| `/etc/sentinel/config.toml` | 設定 |
| `/var/lib/sentinel/` | databaseとspool |
| `/run/sentinel/` | runtime state |

ログはstderrへ出力されます。systemd配下ではjournalに入ります。
