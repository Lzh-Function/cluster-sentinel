# コマンドリファレンス

`sentinel`はバイナリ1つです。controllerもagentも調査用のCLIも、
すべてこのバイナリのサブコマンドとして提供されます。

---

## 早見表

やりたいことから引く場合。

| やりたいこと | コマンド |
| --- | --- |
| 今どうなっているか知りたい | `sentinel status` |
| 障害の原因と判断の根拠 | `sentinel diagnose` |
| 対応が必要な障害の一覧 | `sentinel incident list` |
| その障害の根拠と経緯を全部見る | `sentinel incident show <id>` |
| 動いているはずの検査が動いているか | `sentinel audit` |
| そもそもちゃんと監視できているのか | `sentinel explain paths` |
| 何をどんなコマンドで監視しているのか | `sentinel explain` |
| あるホストの詳細を見る | `sentinel entity show <name>` |
| その判断の元になった生データ | `sentinel entity observations <name>` |
| 誰が誰を監視しているか | `sentinel peers` |
| 依存関係のグラフ | `sentinel dependency list` |
| このホストがSentinelからどう見えるか | `sentinel doctor` |
| 通知が実際に届くか試す | `sentinel notify test` |
| 計画作業中の通知を止める | `sentinel maintenance start <name> --reason <理由>` |
| 止めているものを確認する / 解除する | `sentinel maintenance list` / `end <id>` |
| 設定が正しいか確かめる | `sentinel config check` |
| 設定の実効値と、その出どころ | `sentinel config show` |
| 導入する（設定・unit・credentialを生成） | `sentinel install controller` / `agent` |
| 自動検出を手動実行する | `sentinel discover` |
| 古い記録を消す | `sentinel prune --dry-run` |

---

## 共通のこと

### 実行ユーザー

`sentinel`ユーザーかrootで実行してください。

```bash
sudo -u sentinel sentinel status
```

設定ファイルは全ユーザーから読み取り可能にしていません。webhookのURL自体が
credentialを含みうるためです。

### 全コマンド共通のオプション

| オプション | 意味 |
| --- | --- |
| `-c`, `--config <PATH>` | 設定ファイルの場所。既定`/etc/sentinel/config.toml`。環境変数`SENTINEL_CONFIG`でも指定可 |
| `--json` | 機械可読出力（`controller` / `agent` / `install` / `explain`を除く） |
| `-v`, `--verbose` | ログを詳しく。重ねるとさらに詳しく（`-vv`） |
| `--log-json` | ログをJSONで出す |

`--json`はほぼすべての調査系コマンドにあります。`jq`と組み合わせる前提の
安定した形なので、スクリプトからはこちらを使ってください。
（`explain`は人が読むためのものなので`--json`はありません。）

### 終了コード

そのままhealth checkに使えます。

| コマンド | `0` | `1` | `2` |
| --- | --- | --- | --- |
| `status` | 正常 | 該当なし | 異常あり（openなincidentを含む） |
| `diagnose` | 何も問題なし | 該当なし | 診断結果あり |
| `incident list` | activeなincidentなし | 該当なし | activeなincidentあり |
| `config check` | 妥当 | errorあり | 該当なし |
| `discover` | 全provider成功 | 失敗したproviderあり | 該当なし |
| `notify test` | 全宛先に到達 | 該当なし | 失敗した宛先あり |
| `entity show` / `observations` | 成功 | 見つからない／曖昧 | 該当なし |
| `audit` | 観測結果が届いていないprobeなし | 該当なし | 観測結果が届いていないprobeあり |

---

## 状態を見る

### `sentinel status`

環境全体の状態を一覧表示します。障害の報告を受けたら、最初に確認します。

```bash
sentinel status
```

openなincidentがあれば、entity一覧の前に表示されます。
その場合exit codeは2になります。

末尾には、必要なときだけ次の行が出ます。

- agentのバージョンが混在しているとき（更新中は正常、更新後なら見落とし）
- 観測結果が届いていないprobeがあるとき（`sentinel audit`へ誘導）

> HEALTHYという表示に加えて、必要な検査の実行を確認してください。
> `explain paths`で監視元を、`entity observations`で最新の観測結果を確認します。
> 詳細は[運用ガイド](OPERATIONS.md#healthy表示と観測結果を確認する)を参照してください。

### `sentinel diagnose`

障害の原因、診断の根拠、
そして安全に実行できる調査コマンドの提案を出します。

```bash
sentinel diagnose
```

提案されるコマンドはすべて読み取り専用です。Sentinelが自分で
`restart`や`scontrol update`を実行することはありません。

### `sentinel incident list`

対応中のincidentを一覧表示します。通知はincidentに基づいて送信します。

| オプション | 意味 |
| --- | --- |
| `--all` | 解決済みも含める（既定はactiveのみ） |

### `sentinel incident show <id>`

incident1件の履歴、根拠となる観測、原因の候補、影響範囲を表示します。

```bash
sentinel incident show 7f2c0ef6-eaee-485a-bb18-e6612411bfe9
```

idは`incident list`や通知本文からコピーできます。

---

## 監視の中身を見る

### `sentinel explain [capabilities|probes|paths]`

検査の有効化条件、検査内容、監視元の割り当てを表示します。
引数を省略すると、3種類すべてを表示します。

| 引数 | 内容 |
| --- | --- |
| `capabilities` | 何が検査を有効にするのか、どう判定されるのか |
| `probes` | 各検査が実際に実行するコマンド、間隔、タイムアウト |
| `paths` | どのホストを誰が、何で監視しているか |

```bash
sentinel explain paths
```

出力の`watched by`が重要です。

```
compute02
  reached at   172.20.0.4
  from itself  host.metrics
  from others  network.tcp, sentinel.agent, ssh.service
  watched by   compute03 (SameDomain), filesrv02 (Independent), compute01 (Filler)
```

`watched by`が空か1台しかないホストは、到達性の診断に必要な観測元が足りません。
到達性の判断には独立した観測元が最低2つ必要で、1台では判断を保留します。

### `sentinel audit`

動いているはずの検査が、実際に動いているか。

```bash
sentinel audit
```

`explain probes`は有効な検査を、`entity observations`は実行結果を表示します。
`audit`は両者を比較し、有効なのに観測結果が届いていないprobeを報告します。
検査自体が実行されていなければ障害も診断できないため、監視漏れの確認に使います。

報告は2種類に分かれます。

| 種別 | 意味 |
| --- | --- |
| never | 一度も観測が無い。どこからも実行されていない可能性が高い |
| （時刻あり） | 以前は動いていたが止まった。agentの停止、capabilityの消失など |

probeごとにまとめて表示します。1台だけ観測結果が届かない場合は
そのホストを確認します。適用対象の全ホストで観測結果がない場合は、
probeが実行処理に登録されているかを確認します。

誤検知を避けるため、次は報告しません。

- 設定で明示的に無効化されているprobe
- agentが居ないホストのローカルprobe（実行する主体がいない）
- observerが付いていないホストのリモートprobe（実行する主体がいない。割り当ては`explain paths`で確認）
- 直近で1回取りこぼしただけのもの（probe間隔の10倍、最低5分は待つ）

観測結果が届いていなければexit code 2を返すので、cronやCIに置けます。

```bash
sudo -u sentinel sentinel audit > /dev/null || echo "監視に穴があります"
```

`audit`の実行忘れによる見落としを減らすため、`status`の末尾にも警告を表示します。

```
⚠ 1 probe(s) have never reported at all; run `sentinel audit` for which
```

### `sentinel entity list`

| オプション | 意味 |
| --- | --- |
| `--type <TYPE>` | `host` / `service` / `storage` / `scheduler`などで絞る |

### `sentinel entity show <name>`

entity一つの詳細を表示します。capability、各componentの状態、
probeの接続先アドレス、agentのバージョン、報告されたハードウェアを確認できます。

```bash
sentinel entity show compute02
```

名前が重複する場合は`type/name`で指定します。ストレージentityは
提供元ホストと同じ名前になるためです。

```bash
sentinel entity show host/filesrv02      # ホストそのもの
sentinel entity show storage/filesrv02   # そのホストが提供するストレージ
```

曖昧なまま指定した場合、勝手にどちらかを選ばずに候補を表示します。
entity idをそのまま渡すこともできます。

### `sentinel entity observations <name>`

診断の根拠となる観測記録を表示します。観測時刻、観測元、検査内容と結果を確認できます。

```bash
sentinel entity observations compute02
```

| オプション | 意味 |
| --- | --- |
| `--probe <ID>` | 特定の検査だけ（例`network.tcp`） |
| `--limit <N>` | 件数。既定40 |

`--probe`を付けるとその検査だけを絞って検索します。「観測がありません」と
出たら、その検査は本当に走っていません（他の検査に埋もれて見えないのでは
ありません）。capabilityが付いていない可能性が高いので、
`entity show`を確認してください。

### `sentinel peers`

observerの割り当て。observerが付いていないentityは明示されます。

### `sentinel dependency list`

依存関係の一覧。NFSのマウント関係はagentの報告から自動で導出されるので、
設定ファイルに書いた覚えがなくてもここに出てきます。

### `sentinel doctor`

そのホスト自身がSentinelからどう見えるか。capabilityの判定理由、
報告するアドレスとその選定理由、spoolの深さ。

```bash
sentinel doctor
```

「なぜこの検査が動かないのか」を調べるときは、対象ホストで
これを実行するのがいちばん速いです。

---

## 設定

### `sentinel config check`

起動前に必ず。ここでエラーになる設定では、サービスも起動できません。

```bash
sudo -u sentinel sentinel config check
```

errorとwarningを区別します。「宣言していないentityを参照している」は
warningです。Slurmの自動検出やagentの登録で、後から監視対象一覧へ追加される場合があるためです。

### `sentinel config show`

実効値と、その値がどこから来たのか。既定値なのか設定ファイルなのかが
分かるので、「設定したはずなのに反映されていない」の調査に使います。

### `sentinel config init`

全項目を既定値つきで書き出します。`install`が内部で使っているものと同じです。

| オプション | 意味 |
| --- | --- |
| `--role <ROLE>` | `controller`または`agent`（既定`agent`） |
| `--output <PATH>` | 出力先。省略時は設定パス |
| `--dry-run` | 書かずに表示する |
| `--force` | 既存ファイルを上書き |

---

## 導入・保守

### `sentinel install <controller|agent>`

設定ファイル・systemd unit・（controllerなら）cluster credentialを生成します。
手で書くファイルはありません。

```bash
sudo sentinel install controller
```

| オプション | 意味 |
| --- | --- |
| `--config <PATH>` | 設定ファイルの場所。同居させるときに使う（下記） |
| `--binary <PATH>` | unitが実行するバイナリのパス |
| `--output-dir <DIR>` | unitの出力先。既定`/etc/systemd/system` |
| `--dry-run` | 何も書かずに全部表示する |
| `--force` | 既存ファイルを上書き（credentialは決して上書きしません） |
| `--no-credential` | controllerでもcredentialを作らない |

実行後に「次にやること」が表示されます。すでに済んでいる手順は
表示されません。

既存ファイルは`--force`なしには触りません。credentialだけは`--force`を
付けても上書きされません。入れ替えると、既存のagentが認証できなくなるためです。

controllerとagentを同じホストで動かす場合は設定パスを分けます。
既定のままだとcontrollerの設定が上書きされます。

```bash
sudo sentinel install agent --config /etc/sentinel/agent.toml
```

詳細は [DEPLOYMENT.md §9.10](DEPLOYMENT.md#910-controllerにagentを同居させる)。

### `sentinel discover`

監視対象の自動検出を1回だけ手動実行します。controllerは定期実行しているため、通常は不要です。設定を変えた直後の確認に使います。

NFSのマウントで解決できなかったアドレスがあれば、ここで報告されます。

### `sentinel prune`

保持期間を過ぎた記録を削除します。controllerが自動で行うので通常は不要です。

| オプション | 意味 |
| --- | --- |
| `--dry-run` | 削除せずに量だけ報告 |
| `--observations <PERIOD>` | この実行に限り観測の保持期間を上書き |
| `--vacuum` | 空き領域をファイルシステムへ返す。database全体を書き直すので任意 |

```bash
sudo -u sentinel sentinel prune --dry-run
```

保持期間を短くする前に`--dry-run`で影響を確かめられます。
openなincidentと、保存されているincidentが参照している観測は
設定に関わらず削除されません。

---

## 通知

### `sentinel notify test`

設定した宛先に、テスト通知を送ります。障害を待つ必要はありません。

```bash
sudo -u sentinel sentinel notify test
```

| オプション | 意味 |
| --- | --- |
| `--provider <NAME>` | その宛先だけ |
| `--severity <LEVEL>` | 送るseverity。既定`warning` |

宛先ごとに1通だけ送ります。incident、database、重複排除の記録は変更しません。
疎通確認のための通知なので、`min_severity`未満でも送ります。
設定したら一度実行し、障害が起きる前に配信結果を確認してください。

---

## 計画作業

### `sentinel maintenance start [<name>] --reason <理由>`

計画作業中の通知を止めます。計画的に停止するときに使うものです。

```bash
sudo -u sentinel sentinel maintenance start filesrv01 --reason "HDD 換装" --for 6h
```

| 引数 / オプション | 意味 |
| --- | --- |
| `<name>` | 対象。`host/filesrv01`のように型を付けて曖昧さを消せます。省略すると環境全体 |
| `--reason <TEXT>` | 必須。`list`に表示されるので、後から見て分かる言葉で |
| `--for <DURATION>` | `6h`、`30m`、`2d`など。省略すると期限なし |

抑止されるのは通知だけです。probeは動き続け、stateもdiagnosisも
更新され続けます。`status`にも`incident list`にも、作業中の異常はそのまま
出ます。作業中に別の障害が始まったとき、その記録がなければ
いつ始まったのか後から分からなくなるためです。

incidentが抑止されるのは、影響を受けているentityがすべて
maintenance対象である場合のみです。1台の作業を宣言しても、4台に影響する障害の通知は止まりません。

> `--for`を省略すると、手動で解除するまで通知の抑止が続きます。
> 期限なしのwindowは`end`を打つまで残り、通常の障害通知も止まります。
> `sentinel status`の末尾に出るので、日々の確認で気付けるようにはなっています。

### `sentinel maintenance list`

今抑止されているものを表示します。

```bash
sudo -u sentinel sentinel maintenance list
```

| オプション | 意味 |
| --- | --- |
| `--all` | 終了済みのものも含める |

終了済みのwindowも記録として残ります（消しません）。
「あの日なぜ通知が来なかったのか」を後から説明できるようにするためです。

### `sentinel maintenance end <id>`

解除します。次の診断サイクルから通知が再開されます。

```bash
sudo -u sentinel sentinel maintenance end 2a464de5
```

`id`は`start`が表示したものです。先頭数文字で足ります。

作業後も障害が続いていれば、通知が届きます。作業中も検査と判定は継続しています。

---

## デーモン

### `sentinel controller` / `sentinel agent`

デーモン本体。通常はsystemdから起動されるので、手動で起動するのは
デバッグのときだけです。

```bash
sudo -u sentinel sentinel -vv controller
```

`-v`を重ねると各サイクルの詳細が出ます。

---

## `sentinel version`

```
sentinel 1.0.0
protocol version: 1
config version:   1
target:           x86_64-unknown-linux-musl
```

バイナリ、通信プロトコル、設定形式のバージョンは、それぞれ個別に管理します。

- バイナリversion、このバイナリのリリース
- protocol version、controllerとagentの通信規約。これが一致していれば、
  バイナリversionが混在していても動きます（アップグレード中はそうなります）
- config version、設定ファイルの形式。未知の値は起動時に拒否されます

---

## よく使う組み合わせ

障害の報告を受けたとき

```bash
sentinel status && sentinel diagnose && sentinel incident list
```

特定のホストがおかしいとき

```bash
sentinel entity show <host> && sentinel entity observations <host>
```

導入直後・構成変更後の確認

```bash
sentinel explain paths && sentinel dependency list
```

監視スクリプトから

```bash
sentinel status --json | jq -r '.entities[] | select(.health != "healthy") | "\(.entity_type)/\(.name) \(.health)"'
```

cronから健全性を見る（異常ならexit 2）

```bash
sudo -u sentinel sentinel status > /dev/null || echo "cluster-sentinel: 異常あり"
```
