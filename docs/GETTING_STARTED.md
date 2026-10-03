# はじめてのCluster Sentinel

初めて導入する方向けに、controllerとagentの起動から状態確認までを説明します。
作業時間の目安は30分です。詳細な設定や運用方法は、末尾のリンクを参照してください。

---

## 1. これは何をするものか

ノードが応答しない原因によって、確認すべき対象は変わります。
Sentinelは複数のノードからの観測結果を比較し、次のような障害を区別します。

| 見た目 | 実際に起きていること | やること |
| --- | --- | --- |
| ノードに繋がらない | 複数の観測元から到達不能 | 現地を見に行く |
| ノードに繋がらない | 経路の一部だけが切れている | ネットワークを見る |
| ノードに繋がらない | SSHだけが停止している（ホストは応答している） | sshdを直す |
| ノードが使えない | Slurm上でdrainされているだけ | 誰がdrainしたか調べる |
| 何台も同時に不調 | 共有ストレージ1台が原因 | その1台を見る |

障害の原因を区別し、確認先を絞ることが目的です。
1台からの到達失敗だけでは、ホスト全体が到達不能になったとは判定しません。

---

## 2. 最低限の用語

読み進めるのに必要なのはこれだけです。

### controller

観測結果を集め、診断と通知を行うプロセスです。通常はヘッドノードで動かします。

### agent

各ノードで動く常駐プロセスです。自分のホストの状態を報告し、
割り当てられた他のノードへの到達性も検査します。

### entity（エンティティ）

ホスト、サービス、ストレージなど、一つの監視対象です。
ホストfilesrv02と、そのホストが提供するストレージを分けて扱います。
これにより、ホストは応答していてもNFSの提供だけが停止した状態を表せます。

### capability（ケーパビリティ）

GPUがある、NFSを使っているなど、ノードの機能を示す情報です。
実際に検出した機能に基づいて、実行する検査を決めます。
たとえばGPU検査は「計算ノード」という役割では有効にせず、
`nvidia-smi`の有無などから実行できるかを判断します。

### probe（プローブ）

TCP接続の確認や`systemctl`による状態取得など、一つの検査です。

### incident（インシデント）

診断結果を対応すべき障害としてまとめた記録です。通知はこの記録に基づいて送ります。

---

## 3. 準備するもの

- controllerにする1台（ヘッドノードで構いません）
- agentを入れるノード（3台以上を推奨。理由は後述）
- 全ノードからcontrollerのTCP 7443に届くこと
- ノード同士がTCP 7444で届くこと（相互監視に使います）

> なぜ3台以上か
> AからBに到達できないだけでは、Bの障害かAとBの経路障害かを
> 区別できません。到達性の診断には、独立した観測元が2台以上必要です。
> 観測元が1台の場合は、診断を保留します。

コンパイルは不要です。バイナリ1つでcontrollerもagentもCLIも兼ねます。

```bash
ARCH=$(uname -m) && curl -fsSL -o sentinel "https://github.com/mizuno-group/cluster-sentinel/releases/latest/download/sentinel-${ARCH}-unknown-linux-musl" && chmod +x sentinel && sudo mv sentinel /usr/local/bin/
```

x86_64とARMが混在していても、各ノードで上を実行すれば正しいものが入ります。

```bash
sentinel version
```

---

## 4. controllerを構築する

### 4.1 一式を生成する

```bash
sudo sentinel install controller
```

これだけで、設定ファイル・systemd unit・クラスタcredentialが作られます。
手で書くファイルはありません。設定ファイルには全項目が既定値つきで
書き出され、変えるべき行にだけ`CHANGE-ME`が入っています。

コマンドの最後に「次にやること」が表示されます。その通りに進めれば済みます
（すでに済んでいる手順は表示されません）。以下はその補足です。

### 4.2 サービスユーザーを作る

Sentinelのサービスは、専用の非特権ユーザーで動かします。
監視対象を変更せず、状態を取得するためです。

```bash
sudo useradd --system --no-create-home --shell /usr/sbin/nologin sentinel
```

```bash
sudo install -d -o sentinel -g sentinel -m 0750 /var/lib/sentinel && sudo chown -R sentinel:sentinel /etc/sentinel
```

```bash
sudo usermod -aG systemd-journal sentinel
```

最後の1行は、カーネルログを読めるようにするためのものです。
この設定がないと、ディスクI/Oエラーなどのカーネルイベントを収集できません。

### 4.3 環境名を決める

設定ファイルの`CHANGE-ME`を書き換えます。環境名は全ノードで一致させる
必要があります。クラスタの名前でも研究室名でも構いません。

```bash
sudo sed -i 's/^environment = .*/environment = "my-cluster"/' /etc/sentinel/config.toml
```

Slurmを使っているなら、次を有効にします（`scontrol`から自動でノード一覧を
取ってくるので、ノードを手で列挙する必要がなくなります）。

```toml
[discovery.slurm]
enabled = true
```

書けたら確認します。ここでエラーになる設定では、サービスも起動できません。

```bash
sudo -u sentinel sentinel config check
```

### 4.4 起動する

```bash
sudo systemctl daemon-reload && sudo systemctl enable --now sentinel-controller
```

```bash
sudo -u sentinel sentinel discover && sudo -u sentinel sentinel status
```

Slurmを有効にしたなら、この時点でノードが一覧に出ます。まだagentが
いないので、多くが`UNKNOWN`のはずです。それで正常です。

---

## 5. agentを配る

### 5.1 credentialを配る

全ノードが同じcredentialを持つ必要があります。controllerが作ったものを
そのまま配ります。

```bash
sudo scp /etc/sentinel/token <node>:/etc/sentinel/token
```

> 上書きに注意。すでに動いているクラスタで別のcredentialを置くと、
> 全agentが締め出されます。配るのは初回だけです。

### 5.2 各ノードで

```bash
sudo sentinel install agent
```

controllerと同じように、設定とunitが生成されます。書き換えるのは2行だけです。

```bash
sudo sed -i 's/^environment = .*/environment = "my-cluster"/; s/^controller_address = .*/controller_address = "head:7443"/' /etc/sentinel/config.toml
```

サービスユーザーの作成（4.2）を各ノードでも行ってから、起動します。

```bash
sudo systemctl daemon-reload && sudo systemctl enable --now sentinel-agent
```

### 5.3 台数が多い場合

複数ノードへの配布には、同梱のAnsibleロールを使えます。
controllerの設定を読み取って、揃えるべき項目を自動で配ります
（環境名や監視頻度を2か所で管理しなくて済みます）。

[`deploy/ansible/`](../deploy/ansible/) を参照してください。

> controllerのホストを`agents`グループに入れないでください。
> ロールは決まったパスに設定ファイルを書くため、controllerの設定が
> 上書きされてcontrollerが止まります。controllerにagentを同居させる
> 方法は [DEPLOYMENT.md](DEPLOYMENT.md) にあります。

---

## 6. 動いていることを確かめる

```bash
sudo -u sentinel sentinel status
```

しばらく待つと`UNKNOWN`が`HEALTHY`に変わっていきます。

HEALTHYという表示に加えて、必要な検査が実行されていることを確認します。
過去には、実行されていない検査があってもHEALTHYと表示される不具合がありました。
次の2つのコマンドで、監視元の割り当てと最新の観測結果を確認します。

```bash
sudo -u sentinel sentinel explain
```

何が検査を有効にしているか、各検査が実際にどんなコマンドを実行するか、
どのノードが誰を監視しているかを表示します。監視元が2台未満のノードは、
到達性の診断に必要な観測元が足りません。

```bash
sudo -u sentinel sentinel entity observations <node-name>
```

そのノードについて、いつ・誰が・何を観測したかの生データです。
観測時刻が現在時刻の近くで更新され続けていることを確認します。

そして、これを毎回手で確かめなくて済むようにするのが次のコマンドです。

```bash
sudo -u sentinel sentinel audit
```

「有効なのに観測を出していない検査」を挙げます。何も無ければ1行で終わります。
定期確認にはcronを使えます。監視漏れがあれば終了コード2を返します。

---

## 7. 通知を設定する

設定しないと、障害が起きても`status`を見に行くまで気づけません。

controllerの設定ファイルに追記します。

```toml
[notification]
min_severity = "warning"

[[notification.webhooks]]
name   = "ops"
url    = "https://hooks.slack.com/services/XXX/YYY/ZZZ"
format = "slack"
```

`format = "slack"`にすると、色つきの帯と太字で整形されます。
それ以外の宛先なら`"generic"`のままにしてください。

障害を待たずに、届くかどうかだけ先に試せます。

```bash
sudo -u sentinel sentinel notify test
```

宛先ごとに1通だけ送られます。障害が起きる前に、URLと配信結果を確認してください。

通知は状態が変わったときだけ飛びます。続いている障害を
繰り返し通知することはありません。復旧時にも届きます。

### 計画停止の前に通知を止める

ディスク換装や電源工事のように、自分で止めると分かっているときは、
作業前にmaintenance windowを設定すると、対象の通知を止められます。

```bash
sudo -u sentinel sentinel maintenance start <node> --reason "HDD 換装" --for 6h
```

作業が終わったら`sentinel maintenance end <id>`で解除します
（`id`は上のコマンドが表示します。先頭数文字で足ります）。

止めるのは通知だけです。検査と判定は続いているので、
作業中に別の障害が始まった場合も、後から記録を追えます。
詳しくは [OPERATIONS.md](OPERATIONS.md#計画作業中に通知を止めるmaintenance-window)。

---

## 8. 最初につまずきやすいところ

### SSHが22番ではない

agentが`/etc/ssh/sshd_config`を読んで自動的に検出します。何もしなくて
構いません。agentを入れていないホストだけ、設定ファイルで教える必要が
あります（[DEPLOYMENT.md](DEPLOYMENT.md) の該当節）。

### `config check`が通らない

メッセージがどの行の何が問題かを言います。`CHANGE-ME`の消し忘れが
いちばん多いです。

### agentが起動直後に終了を繰り返す

設定ファイルを`sentinel`ユーザーが読めていない可能性があります。
`sudo chown -R sentinel:sentinel /etc/sentinel`を実行してください。

### ストレージの依存関係を書かないといけない?

不要です。agentが報告するマウント表から自動的に組み立てられます。
ノードがマウント先を変えても設定ファイルを触る必要はありません。

### 通知が来ない

まず`sentinel status`の末尾を見てください。`notifications suppressed by maintenance`
が出ていれば、期限なしのmaintenance windowが残っています。
`sentinel maintenance list`で確認して`end`で解除します。

### あるノードだけ`UNKNOWN`のまま

agentが登録できているかと、必要な観測が届いているかを確認します。ノード側で
`systemctl status sentinel-agent`と`journalctl -u sentinel-agent -n 50`を
見てください。credentialの不一致か、controllerへの到達性が大半です。

---

## 9. 次に読むもの

| 目的 | ドキュメント |
| --- | --- |
| コマンドの一覧と使い分け | [COMMANDS.md](COMMANDS.md) |
| 本番クラスタへ本格導入する（TLS、非標準ポート、段階導入） | [DEPLOYMENT.md](DEPLOYMENT.md) |
| 日々の運用と、障害が出たときの読み方 | [OPERATIONS.md](OPERATIONS.md) |
| 設定項目を全部知りたい | [CONFIGURATION.md](CONFIGURATION.md) |
| 何をどこまで守るツールなのか | [SECURITY.md](SECURITY.md) |
| 設計の考え方 | [ARCHITECTURE.md](ARCHITECTURE.md) |
