分散クラスタの監視・障害検知・原因診断システムです。
成果物は単一バイナリ1つで、controller・agent・CLIをすべて兼ねます。

## ダウンロード

| ファイル | 対象 |
| --- | --- |
| `sentinel-x86_64-unknown-linux-musl` | x86_64 |
| `sentinel-aarch64-unknown-linux-musl` | ARM64 |

静的リンクなので、glibcのバージョンに関係なくどのディストリビューションでも動きます。
同一アーキテクチャなら全ホストに同じファイルを配れます。

```bash
sha256sum -c sentinel-x86_64-unknown-linux-musl.sha256
sudo install -m 0755 sentinel-x86_64-unknown-linux-musl /usr/local/bin/sentinel
sentinel version
```

## v0.3.13での追加

`sentinel explain`、この仕組みを作っていない人が読むためのコマンドです。

```bash
sentinel explain capabilities    # 何が probe を有効にし、どう判定されるか
sentinel explain probes          # 各 probe が実際に何を実行するか
sentinel explain paths           # どの host を誰が、何で監視しているか
```

`status`は「何を結論したか」を言いますが、「それがどうやって分かるのか」は
これまでどこにも出ていませんでした。根拠を確かめられない監視は、
運用者が診断の正誤を確認できません。

```
systemd
  detected   the directory /run/systemd/system exists
  enables    systemd.unit

systemd.unit
  runs       systemctl show <unit> --property=ActiveState,SubState,Result
  cadence    every 10s, timeout 5s

compute02
  from itself  host.metrics, systemd.unit
  from others  network.tcp, sentinel.agent, ssh.service
  watched by   node03 (SameDomain), fs02 (Independent), node01 (Filler)
```

## v0.3.12での追加

`[notification] min_interval`（既定`1s`）。同一宛先への送信間隔の下限です。

1つの障害が複数の依存先に影響すると、1回の診断で複数の通知が発生します。
webhookは共有されたrate-limitedな資源で、Slackは概ね毎秒1通、
超えると429を返します。通知は削減せず、間隔を空けて送信します。
送信数を減らすと、必要な障害通知を失う可能性があるためです。

```toml
[notification]
min_interval = "1s"
```

## v0.3.11での追加

Slack向けの整形（`format = "slack"`）。

```toml
[[notification.webhooks]]
name   = "ops"
url    = "https://hooks.slack.com/services/..."
format = "slack"
```

色つきの帯・見出し・太字・整形済みの詳細で届きます。
復旧は緑（重大度に関わらず）、criticalは赤、warningは黄。
復旧通知を障害発生の通知と区別できるようにしています。

既定は`generic`のままなので、既存の宛先の挙動は変わりません。

## v0.3.10での修正

Slackへの通知が400で弾かれていました。

Slackのincoming webhookは`text`（または`blocks` / `attachments`）を要求し、
無ければ`missing_text_or_fallback_or_attachments`を返します。
汎用JSONをそのまま送っていたため、Slackを宛先にしている場合、
通知は1件も届いていません。

ペイロードに`text`（Slack / Teams）と`content`（Discord）を追加しました。
URLを書くだけで動きます。構造化フィールドはそのまま残っています。

## v0.3.9での修正

incidentが開いても通知されないことがありました。

correlationが自動検出ループと診断ループの両方で走っており、
通知を送るのは後者だけでした。先に走ったほうが「開いた」という事実を
消費するため、自動検出ループで開かれたincidentは記録されるだけで
一度も通知されません。

自動検出は既定で5分ごと、診断は15秒ごとなので、両者が重なった
タイミングの障害だけが通知されないままになります。
両ループの実行順に依存するため、常に再現するわけではありませんでした。

incidentを開く場所を診断ループの1箇所に統一しました。

## v0.3.8での追加

`sentinel notify test`、設定した通知先に届くかを、障害を待たずに確認できます。

```bash
sudo -u sentinel sentinel notify test
```

```
ops                  sent
broken               FAILED: cannot reach http://... : error sending request

1 of 2 destination(s) failed.
```

incidentもdatabaseも重複排除も触りません。「このcontrollerが設定した通知先に
届くか」を確認します。障害が起きる前に、URLや通信上の問題を検出できます。

## v0.3.7での修正

サービスentityを誰も観測していませんでした。

`slurmd@<node>`は監視対象一覧に存在するのにprobeが1つも向いておらず、
実クラスタで31 entity中13が永久にUNKNOWNでした。

unitの状態をsystemdに訊くのはそのマシン上でしかできない問いなので、
controllerは肩代わりできません。そしてagentは自分のホストについての
probeしかスケジュールしていませんでした。

agentが、自分のcapabilityが示すunitを監視するようにしました。
観測はサービスentityに帰属します（ホストに混ぜると、
「デーモンが停止した」と「ホスト全体が到達不能になった」の区別が消えるため）。

## v0.3.6での修正

agentが自分のsandboxのマウント表を読んでいました。

生成されるsystemd unitの`ProtectSystem=strict`により、サービスは
ファイルシステム全体が読み取り専用に再マウントされた専用namespaceで動きます。
probeが読んでいた`/proc/self/mounts`はそのnamespace内のマウント状態なので、
書き込み可能なNFS共有が全ノードで読み取り専用と報告されます。

ホストのマウント表（`/proc/1/mounts`）を読むよう修正しました。
v0.3.5以前を使っている場合、NFS mountの状態は信用できません。

## v0.3.5での修正

実クラスタで見つかった誤検知2件です。バイナリの更新が必要です。

* 到達性probeがSSHポートを無視して22番に固定されていました。
  SSHを22以外に移し、22番をfirewallでDROPしている環境では、
  健全なホストが到達不能と報告されます。
  22番がREJECTを返すホストだけが到達可能と判定され、
  同じクラスタ内で結果が割れていました。
  agentが`sshd_config`から検出して報告する`ports.ssh`を優先します。
* 読み取り専用なNFS mountを劣化と判定していました。
  `ro`は「読み取り専用である」ことしか示さず、意図的なro共有を
  常時DEGRADEDと報告していました。事実として記録するだけにしました
  （カーネルが読み取り専用へ変更した場合はjournal probeが捉えます）。
* `sentinel`ユーザーを`systemd-journal` groupに入れる案内を追加。
  無いと`journal.events`がUNSUPPORTEDになり、カーネルイベントが
  一切収集されません。
* firewallで開けるポート（SSHのポート、7444、7443）を文書化。

## v0.3.4での変更

Ansibleロールのみの変更です。バイナリに変更はありません。
`deploy/ansible/`を使っている場合は`git pull`してください。

* 各ノードへ配る共通設定を、controllerの`config.toml`から取得するようになりました。
  `environment`・`[probes]`（監視頻度）・`[tls]`のクライアント側・待ち受けポートを
  ロールがcontrollerから読み取って各ノードへ配ります。
  監視頻度の変更はcontroller 1箇所の編集で済みます。
* credentialが0750になっていました（修正）。
  再帰的な`file`タスクがディレクトリ用のモードをファイルにも適用し、
  0400で書いたtokenをgroup読み取り可能にしていました。
* 毎回バイナリを再ダウンロードしていました（修正）。
* `-e sentinel_observer=false`がobserverを有効にしていました（修正）。
* controllerに対して実行すると設定を上書きしていました（実行を拒否するよう修正）。

## v0.3.3での変更

切り分け用のコマンドが増えました。

```bash
sentinel entity observations <name>            # 誰が何を見たか
sentinel entity observations <name> --probe network.tcp --limit 100
```

stateやdiagnosisではなく、生の観測を表示します。
時刻 / probe / observer / status / アドレスと失敗理由。
「SSHは通るのに到達不能」のような一見矛盾した状態は、
観測者ごとに結果が違うだけであることが多く、observer列で判別できます。

* `entity show`がprobe先アドレスと、その決まり方
  （agentの報告か、entity名からの解決か）を表示します。
* `doctor`のcredential判定が環境変数しか見ておらず、
  正常なホストでも`NOT CONFIGURED`と表示されていました。
* Ansibleロールがv0.3.0を配っていました（`sentinel_version`の更新漏れ）。
  リポジトリの版と一致していなければ、テストが失敗するようにしました。

## v0.3.2での修正

* `sentinel install --force`がcluster credentialを上書きしなくなりました。
  `--force`を実行する理由は、多くの場合systemd unitの更新を取り込むことです。
  それがcredentialを再生成していたため、アップグレードのつもりで実行すると
  全agentが一斉に締め出されていました。失敗は後から各ノードの認証エラーとして
  現れるため、原因に辿り着きにくい形でした。
  意図的な更新は、ファイルを削除して`install`を実行し、全ホストに配り直します。
* アップグレード手順を [OPERATIONS.md](../docs/OPERATIONS.md) に具体化しました
  （置き換え前の`config check`、controller → agentの順序、
  unitが変わった場合、切り戻し）。
* AnsibleロールがSSH / `sudo`のパスワード認証環境で動くようになりました。

## v0.3.1での修正

v0.3.0には、実機で問題になる不具合が含まれています。更新を推奨します。

* 到達性probeが1つも動いていませんでした。Slurm自動検出で見つかった
  ホストは`slurm.compute` capabilityしか持たないため、`network.tcp`を
  要求していたreachability probeが全てskipされていました。
  結果、誰も接触していないホストがHEALTHYと表示されます。
* 報告アドレスの選択。loopbackインターフェース上のアドレスを除外し、
  物理NICを仮想NICより優先。NICが複数ある場合は「曖昧である」と報告します
  （`[agent] interface`で指定してください）。
* systemd unitが`StateDirectory=`を持つため、`/var/lib/sentinel`の
  手動作成が不要になりました。
* ダウンロードしたディレクトリのまま`install`すると、unitが
  消えるパスを指してしまう問題を警告するようになりました。
* 設定ファイルの`Permission denied`が、正しい実行方法を案内します。
* `install`が`scontrol`の有無を見てSlurm自動検出を設定します。

ノードが多い場合は [Ansible ロール](../deploy/ansible/) を使ってください。

## 導入

```bash
sudo sentinel install controller   # または agent
```

設定ファイル・systemd unit・cluster credentialが生成されます。
書き換えが必要なのは`CHANGE-ME`を含む行だけです
（controllerは1行、agentは2行）。

NICが複数ある環境では、各ホストで`sentinel doctor`を1回確認してください。
どのNICでクラスタ内通信をしているかは自動判別できないため、
候補が複数ある場合はその旨が表示されます。`[agent] interface`で指定します。

手順の全体は [docs/DEPLOYMENT.md](../docs/DEPLOYMENT.md) にあります。

## 何ができるか

単一の観測点からは区別できない障害を、複数の観測者の合意と依存グラフから切り分けます。

* ホスト全体の障害 / 経路だけの障害 / SSHだけの障害 / agentだけの障害 / slurmdだけの障害
* SlurmのDRAIN・DOWNと、ホスト自体の障害
* Slurm制御系の障害
* NFSサーバーの障害 / クライアント側だけの障害 / 共有ストレージ起因の多ノード障害
* GPUのリソース設定不一致
* カーネルイベント（OOM・I/O error・hung task・NVMeタイムアウト・GPU Xidなど）の継続収集

## 設計上の約束

* 単一の観測者の失敗からホスト全体の障害を結論しません。独立した2つ以上の合意が必要です
* BMC/IPMIの証拠なしに電源断とは言いません
* 自動復旧を一切行いません。再起動もrestartも`scontrol update`もしません
* 任意のコマンドを遠隔実行する機能はありません。SSHはバナーを読むだけで、鍵もパスワードも持ちません
* 診断にLLMを使いません
* coreにhostname・IP・Slurmパーティション・NFS構成をハードコードしていません

## この版で検証していないもの

Docker疑似クラスタで実Slurmを動かした受け入れ23項目は通っていますが、
以下は実機での検証が必要です（[docs/VM_VALIDATION.md](../docs/VM_VALIDATION.md)）。

* 再起動をまたぐboot IDの変化
* NFS hard mount時のkernel D-state
* 実GPU
* `journal.events` probe（コンテナにsystemdが無いため疑似クラスタでは動きません）
* TLSの実運用（protocol部分は検証済み）

controllerのHA、リモート読み取りAPI、per-node credentialは範囲外です。
