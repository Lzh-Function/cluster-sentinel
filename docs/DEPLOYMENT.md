# 実クラスタ導入マニュアル

実際の計算クラスタへCluster Sentinelを導入する手順書です。
TLS、複数NIC、非標準ポートなど、本番構成で必要になる設定も扱います。

> はじめて触る場合は [GETTING_STARTED.md](GETTING_STARTED.md) から
> 読んでください。30分で動くところまで行けます。
> この文書は、そのあと本番構成へ広げるときに読むものです
> （TLS、非標準ポート、複数NIC、段階的導入など）。

Rustツールチェインは不要です。配布物は静的リンクされた単一バイナリで、
[Releases](https://github.com/Lzh-Function/cluster-sentinel/releases) から
取得したものをコピーするだけです。

前提として、Sentinelは監視対象を一切変更しません。
reboot・`systemctl restart`・mount操作・`scontrol update`を実行しません。
したがって導入自体がクラスタの動作を変えることはありませんが、
段階的に入れることを強く推奨します（[§7](#7-段階的導入)）。

日常運用は [OPERATIONS.md](OPERATIONS.md)、
設定項目の網羅的な説明は [CONFIGURATION.md](CONFIGURATION.md) を参照してください。

---

## 目次

| | 節 | 対象 |
| --- | --- | --- |
| 1 | [事前確認](#1-事前確認) | 該当なし |
| 2 | [構成の決定](#2-構成の決定) | 該当なし |
| 3 | [バイナリの配置](#3-バイナリの配置) | controllerとagentを置く全ホスト |
| 4 | [Controller の構築](#4-controllerの構築) | controllerのホスト |
| 5 | [cluster credential の配布](#5-cluster-credentialの配布) | 全ホスト |
| 6 | [Agent の展開](#6-agentの展開) | agentを置く各ホスト |
| 6.7 | [監視頻度を変える](#67-監視頻度を変える) | 任意 |
| 7 | [段階的導入](#7-段階的導入) | 該当なし |
| 8 | [SSH ポートが 22 でない場合](#8-sshポートが22でない場合) | 該当する場合 |
| 9 | [その他の非標準構成](#9-その他の非標準構成) | 該当する場合 |
| 9.7 | [NIC が複数ある場合](#97-nicが複数ある場合vlanbridge複数fabric) | VLAN環境は必読 |
| 9.9 | [ストレージ構成は書かなくてよい](#99-ストレージ構成は書かなくてよい例外は3つ) | NFSを使う場合 |
| 9.10 | [controller に agent を同居させる](#910-controllerにagentを同居させる) | 推奨 |
| 10 | [導入後の確認](#10-導入後の確認) | 該当なし |
| 11 | [設定ファイルテンプレート](#11-設定ファイルテンプレート) | 参考 |
| 12 | [チェックリスト](#12-チェックリスト) | 該当なし |

---

## 1. 事前確認

### 必要なもの

| 項目 | 内容 |
| --- | --- |
| OS | Linux（systemd前提） |
| 権限 | 各ホストのroot（導入時のみ。常駐は非特権ユーザー） |
| ネットワーク | agent → controllerへのTCP到達性（既定7443） |
| | controller / peer → 各ホストへのTCP到達性（SSHポート、agentポート7444） |

Rustツールチェインは不要です。配布されるのは静的リンクされた
単一バイナリで、ビルド済みのものをコピーするだけです。

SentinelはSlurmを変更しません。既存の`slurm.conf`を書き換える必要はありません。

### どのホストに何を置くか

導入前にこの表を埋めてください。以降の手順はこれに沿って進みます。

| ホスト | 役割 | 置くもの |
| --- | --- | --- |
| 1台 | controller | バイナリ + 設定 + unit + credential（生成元） |
| 監視したいホスト | agent | バイナリ + 設定 + unit + credential（controllerからコピー） |
| agentを置かないホスト | 外から観測されるだけ | 何も置きません（controllerの設定に宣言するだけ） |

agentを置かないホストにはバイナリも設定ファイルも要りません。
controllerとpeerが外からTCPで観測します。
ただし取得できるのは到達性・SSH・NFSポートまでで、
load・memory・GPU・カーネルイベントなどそのホストの内側は一切見えません。

### 確認しておく情報

```bash
# controllerにするホストの名前と、agentから到達できるアドレス
hostname -f
ip -o addr show

# アーキテクチャ（x86_64とARMが混在するクラスタではホストごとに確認）
uname -m

# Slurmのcontrollerとノード定義（あれば）
grep -E "^(SlurmctldHost|ControlMachine|NodeName)" /etc/slurm/slurm.conf

# SSHのポート（22以外なら §8を参照）
grep -iE "^\s*(Port|ListenAddress)" /etc/ssh/sshd_config

# Slurmの外にあるホスト（ファイルサーバー等）の一覧
#   NFSのマウント関係はagentの報告から自動で導出されるため、
#   調べておく必要はない（§9.9）
```

---

## 2. 構成の決定

| 決めること | 例 | 備考 |
| --- | --- | --- |
| environment名 | `example-lab` | 全ホストで一致させる |
| controllerを置くホスト | `head01` | source codeには現れない。設定だけの問題 |
| scheduler entity名 | `example_cluster` | SlurmのClusterNameに合わせると分かりやすい |
| observerにするホスト | controller / ファイルサーバー/ 一部compute | 3台以上を推奨（後述） |
| クラスタ内通信のNIC | `vlan102`など | NICが複数あるなら必須（§9.7） |
| ストレージの依存関係 | どのノードがどのファイルサーバーを使うか | 誤診断を避けるために重要 |

### observerを3台以上にする理由

observerが1台しかない場合、Sentinelは到達性の診断を行いません。
1観測元では「ホスト全体が到達不能になった」と「経路が切れた」を区別できないためです。

異なる障害ドメインのホストを選んでください。
同じストレージを使う3台は、そのストレージの障害で同時に影響を受けるためです。

### ストレージの依存関係を確認する理由

複数ノードが同じストレージを使っていることを把握できないと、共有ストレージの障害を
個別の`NFS_CLIENT_FAILURE`として報告する場合があります。
NFSの依存関係はagentのマウント表から自動検出します。
`sentinel dependency list`で実構成と一致するかを確認してください。
自動検出できない構成は[§9.9](#99-ストレージ構成は書かなくてよい例外は3つ)の手順で設定します。

---

## 3. バイナリの配置

controllerとagentを置く全ホストで行います。
agentを置かないホストには不要です。

### 3.1 ダウンロード

[Releases](https://github.com/Lzh-Function/cluster-sentinel/releases) から、
そのホストのアーキテクチャに合うものを取得します。

```bash
# x86_64
curl -fsSLO https://github.com/Lzh-Function/cluster-sentinel/releases/latest/download/sentinel-x86_64-unknown-linux-musl
curl -fsSLO https://github.com/Lzh-Function/cluster-sentinel/releases/latest/download/sentinel-x86_64-unknown-linux-musl.sha256
sha256sum -c sentinel-x86_64-unknown-linux-musl.sha256

# ARM64
curl -fsSLO https://github.com/Lzh-Function/cluster-sentinel/releases/latest/download/sentinel-aarch64-unknown-linux-musl
curl -fsSLO https://github.com/Lzh-Function/cluster-sentinel/releases/latest/download/sentinel-aarch64-unknown-linux-musl.sha256
sha256sum -c sentinel-aarch64-unknown-linux-musl.sha256
```

`uname -m`が`x86_64`なら前者、`aarch64`なら後者です。

### 3.2 配置

```bash
sudo install -m 0755 sentinel-x86_64-unknown-linux-musl /usr/local/bin/sentinel
sentinel version
```

```
sentinel 1.0.0
protocol version: 1
config version:   1
target:           x86_64-unknown-linux-musl
```

> 初回はこれで構いませんが、すでにSentinelが動いているホストを
> 更新するときは`install`ではなく`mv`を使ってください。
> 実行中のバイナリはtruncateできず`Text file busy`になります
> （[OPERATIONS.md のアップグレード](OPERATIONS.md#アップグレード)）。

必ず`/usr/local/bin`に置いてから次に進んでください。
ダウンロードしたディレクトリのまま`sentinel install`を実行すると、
生成されるsystemd unitがそのパスを指し、ディレクトリを片付けた時点で
サービスが起動しなくなります。`install`はこの状態を警告します。

静的リンクなので、glibcのバージョンや配布物の追加は不要です。

```bash
ldd /usr/local/bin/sentinel      # -> statically linked
```

同一アーキテクチャなら全ホストに同じファイルを配れます。

```bash
# 例: 各計算ノードへ配る
for n in node01 node02 node03; do
  scp sentinel-x86_64-unknown-linux-musl "$n":/tmp/sentinel
  ssh "$n" 'sudo install -m 0755 /tmp/sentinel /usr/local/bin/sentinel && rm /tmp/sentinel'
done
```

---

## 4. Controllerの構築

controllerにするホスト1台で行います。

### 4.1 サービスユーザーを作る

`install`より先に作ってください。生成されるファイルの所有者になります。

```bash
sudo useradd --system --no-create-home --shell /usr/sbin/nologin sentinel
```

`/etc/sentinel`も`/var/lib/sentinel`も、この時点では存在しなくて構いません。
前者は`install`が、後者はsystemdが起動時に作ります。

### 4.2 install

```bash
sudo sentinel install controller
```

これ1回で、ディレクトリごと以下が生成されます。

| 生成物 | 内容 |
| --- | --- |
| `/etc/sentinel/config.toml` | 全設定を既定値のまま書き出した設定ファイル（説明つき） |
| `/etc/systemd/system/sentinel-controller.service` | 権限とアクセスの制限済みsystemd unit |
| `/etc/sentinel/token` | cluster credential（32 byte乱数、mode 0400） |

既存のファイルは上書きしません。バージョンアップ後にもう一度実行しても、
調整済みの設定やcredentialはそのまま残ります
（credentialが入れ替わると全agentが一斉に締め出されるため）。
上書きしたい場合のみ`--force`を付けてください。

内容を先に確認したい場合

```bash
sentinel install controller --dry-run
```

credentialをsecret managerなどで別管理している場合

```bash
sudo sentinel install controller --no-credential
```

### 4.3 所有者を合わせる

`install`はrootとして書くので、サービスユーザーに渡します。

```bash
sudo chown -R sentinel:sentinel /etc/sentinel
```

`/var/lib/sentinel`（databaseの置き場）はunitの`StateDirectory=`により
systemdが初回起動時に作成し、所有者も設定します。手で作る必要はありません。

### 4.4 設定の仕上げ

生成された`/etc/sentinel/config.toml`のうち、
書き換えが必要なのは`CHANGE-ME`を含む行だけです。
controllerの場合は`environment`の1行です。

それ以外はすべて既定値がそのまま書き出されており、
変更したい行のコメントを外すか値を書き換えます。
Slurmの外にあるファイルサーバーや依存関係は、
ファイル内のコメント例を参考にしてください（§11にも同じものがあります）。

### 4.5 検証

起動前に必ず実行してください。

```bash
sudo -u sentinel sentinel config check
```

検出できる問題をすべて報告します（最初の1件で止まりません）。
`error`が1つでもあれば起動しません。

`warning`は許容されます。特に
「宣言されていないentityへの依存」は、
Slurm自動検出やagent registrationから到着する予定のものであれば正常です。

### 4.6 起動

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now sentinel-controller
systemctl status sentinel-controller
journalctl -u sentinel-controller -f
```

### 4.7 動作確認

CLIはrootか`sentinel`ユーザーで実行してください。

```bash
sudo -u sentinel sentinel status     # 推奨
sudo sentinel status
```

一般ユーザーで実行すると設定ファイルを読めません。
設定ファイルは全ユーザーから読み取り可能にしていません
（`[[notification.webhooks]]`のURL自体がcredentialを含みうるため）。

```
$ sentinel status
error: cannot read config file /etc/sentinel/config.toml: permission denied.
The configuration belongs to the service user, so run one of:
  sudo -u sentinel sentinel <command>
  sudo sentinel <command>
```


```bash
curl -fsS http://localhost:7443/v1/health
sudo -u sentinel sentinel status
sudo -u sentinel sentinel dependency list
```

この時点でentityは0件です。これは正常です。

```
ENVIRONMENT: example-lab

No entities known yet.
```

entityが現れる経路は2つあり、どちらもまだ動いていないためです。

1. Slurm discovery。`install`は`scontrol`の有無を見て
`[discovery.slurm] enabled`を設定します。このホストに`scontrol`が
無かった場合は`false`になっているので、Slurmクラスタなら手で`true`にします。

```bash
sudo -u sentinel grep -A3 "discovery.slurm" /etc/sentinel/config.toml
```

```toml
[discovery.slurm]
enabled = true
```

有効にしたら、次の監視対象一覧周期（既定5分）を待たずに実行できます。

```bash
sudo systemctl restart sentinel-controller
sudo -u sentinel sentinel discover
sudo -u sentinel sentinel status
```

2. agentの登録。§6で展開すると現れます。

Slurmを使っていない、あるいはSlurmの外にあるホストは
`[[entities]]`で宣言します（§11.1のテンプレート参照）。

agentがいないentityが`UNKNOWN`と出るのも正常です。
観測していないものをhealthyとは呼びません。

---

## 5. cluster credentialの配布

environment内の全ホストで同一の値を使用します。
controllerの`install`が生成したものを、各ホストに配布してください。

```bash
sudo scp /etc/sentinel/token <host>:/etc/sentinel/token
# 配布先で
sudo chown sentinel:sentinel /etc/sentinel/token
sudo chmod 0400 /etc/sentinel/token
```

> credential無しではcontrollerもagentも起動を拒否します。
> 未認証で動作するモードはありません。

配布にscpを使う場合、経由地にファイルを残さないよう注意してください。

`sentinel install agent`はcredentialを生成しません（§6.1）。

---

## 6. Agentの展開

agentを置く各ホストで行います。以下はすべてそのホスト上での作業です。

前提は「§3でバイナリを`/usr/local/bin/sentinel`に置いた」ことだけです。
`/etc/sentinel`は存在しなくて構いません。`install`が作ります。

### 6.1 サービスユーザーとinstall

```bash
sudo useradd --system --no-create-home --shell /usr/sbin/nologin sentinel
sudo sentinel install agent
sudo chown -R sentinel:sentinel /etc/sentinel
```

生成物は2つです。

| 生成物 | 内容 |
| --- | --- |
| `/etc/sentinel/config.toml` | 全設定を既定値のまま書き出した設定ファイル |
| `/etc/systemd/system/sentinel-agent.service` | 権限とアクセスの制限済みsystemd unit |

credentialは生成されません。controllerのものを配ります（§5）。
agentが自前で生成すれば、クラスタの誰も知らないcredentialができてしまい、
「設定の問題」が「認証の失敗」として現れることになるためです。

`/var/lib/sentinel`（spoolの置き場）はsystemdが初回起動時に作ります。

### 6.2 credentialを置く

§5でcontrollerから配ったものを配置します。

```bash
sudo install -o sentinel -g sentinel -m 0400 /path/to/token /etc/sentinel/token
```

### 6.3 設定を書き換える

`CHANGE-ME`を含む2行だけです。

```toml
environment = "CHANGE-ME-environment"                 # controller と一致させる
controller_address = "CHANGE-ME-controller-host:7443" # controller のアドレス
```

capabilityはagentが自動検出するため、列挙する必要はありません。

NICが複数あるホストでは、もう1行必要です（§9.7）。
まず候補を確認します。

```bash
sudo sentinel doctor
```

候補が複数あると警告が出るので、`[agent]`セクションに追記します。

```toml
[agent]
interface = "vlan102"
```

### 6.4 検証と起動

```bash
sudo -u sentinel sentinel config check
sudo systemctl daemon-reload
sudo systemctl enable --now sentinel-agent
systemctl status sentinel-agent
```

### 6.5 確認

```bash
# このホストがSentinelからどう見えるか、capabilityと報告アドレスの判定理由つき
sudo sentinel doctor

# controller側から
sentinel entity show <hostname>
sentinel status
```

`sentinel doctor`はcapabilityごとに
「detected on this host」「not present on this host」
「forced on by configuration」「suggested by a role」
のいずれかを表示します。想定と違う場合はここで分かります。

報告アドレスの行に`!`の警告が残っていないことも確認してください。

### 6.6 まとめて展開する場合

ノードが10台を超えるなら [Ansible ロール](../deploy/ansible/) を使ってください。
§3〜§6をそのまま自動化してあり、アーキテクチャ別のバイナリ取得・
チェックサム検証・credential配布・`config check`・起動まで行います。

```bash
cd deploy/ansible
cp inventory.example.ini inventory.ini
$EDITOR inventory.ini
ansible-playbook -i inventory.ini site.yml --limit node01   # まず 1 台
ansible-playbook -i inventory.ini site.yml
```

SSHと`sudo`にパスワードが必要な場合

```bash
ansible-playbook -i inventory.ini site.yml --ask-pass --ask-become-pass
```

`--ask-pass`には`sshpass`が必要です。SSHは鍵にしておくことを推奨します
（`ssh-copy-id`を1回。パスワード認証は毎タスクで使われ、
`PasswordAuthentication no`の環境では使えません）。
詳細は [deploy/ansible/README.md](../deploy/ansible/README.md) を参照してください。

Sentinel側にAnsible固有のものはありません。別の構成管理ツールなら、
同じ手順（バイナリを置く → `sentinel install agent` → 設定とcredentialを配る）
を移植してください。

#### 手作業で配る場合

```bash
for n in node01 node02 node03; do
  scp sentinel-x86_64-unknown-linux-musl "$n":/tmp/sentinel
  scp /etc/sentinel/token "$n":/tmp/token
  ssh "$n" '
    sudo install -m 0755 /tmp/sentinel /usr/local/bin/sentinel
    sudo useradd --system --no-create-home --shell /usr/sbin/nologin sentinel 2>/dev/null || true
    sudo sentinel install agent
    sudo install -o sentinel -g sentinel -m 0400 /tmp/token /etc/sentinel/token
    sudo chown -R sentinel:sentinel /etc/sentinel
    rm -f /tmp/sentinel /tmp/token
  '
done
```

このあと各ホストの`config.toml`を書き換えます
（`environment`、`controller_address`、必要なら`interface`）。
3行とも全ホストで同じ値になるなら、書き換えた1つを配って構いません。

```bash
for n in node01 node02 node03; do
  scp /etc/sentinel/config.toml "$n":/tmp/config.toml
  ssh "$n" '
    sudo install -o sentinel -g sentinel -m 0640 /tmp/config.toml /etc/sentinel/config.toml
    sudo -u sentinel sentinel config check
    sudo systemctl daemon-reload && sudo systemctl enable --now sentinel-agent
    rm -f /tmp/config.toml
  '
done
```

> `scp`の経由地にファイルを残さないでください。credentialも設定も
> `/tmp`を通ります。

---

## 6.7 監視頻度を変える

生成された設定ファイルには全probeの既定値が
コメントアウトされた状態で書き出されています。
変えたい行のコメントを外してください。

```toml
# 大規模クラスタで負荷を下げる
[probes."network.tcp"]
interval = "15s"

# この環境ではGPUを別系統で見ている
[probes."gpu.nvidia"]
enabled = false
```

書かれていないprobeは既定のまま動きます。
存在しないprobe idを書くと`config check`がerrorにします
（警告なしに無視されると「変更したつもりで変わっていない」状態になるため）。

`max_outstanding`は引き下げしかできません。
`nfs.client.io`と`journal.events`は同時実行1に固定されており、
blockingシステムコールを積み上げないための制約は設定で覆せません。

一覧は [CONFIGURATION.md](CONFIGURATION.md) の`[probes]`にあります。

---

## 7. 段階的導入

実クラスタは開発環境ではありません。以下の順で進めてください。

| 段階 | 作業 | 確認すること | 目安 |
| --- | --- | --- | --- |
| 1 | controllerのみ | `sentinel status`が既存構成を正しく表示 | 1日 |
| 2 | agent 1台（重要度の低いノード） | 登録される。`sentinel entity show`が妥当 | 1日 |
| 3 | agent数台 | 全台登録。誤検知が出ない | 2-3日 |
| 4 | `observer.peer`を付与 | `sentinel peers`でobserverが3台付く | 2-3日 |
| 5 | ストレージの依存関係を確認し、不足分を設定 | `sentinel dependency list`が実構成と一致 | |
| 6 | notificationを有効化 | まずテスト用の宛先へ | 1週間 |
| 7 | 全台展開 | | |

各段階で数日おき、誤検知が出ないことを確認してから次へ進んでください。
誤検知に慣れた運用者は、本物の警告も無視するようになります。

### 実クラスタでの障害注入について

自動実行してはなりません。

* NFSサーバーの停止
* ネットワーク全体へのiptables変更
* reboot
* ファイルシステム操作

Docker疑似クラスタ（`dev/compose/`）で代替できるものはそちらで行ってください。
実機でしか確認できない項目は [VM_VALIDATION.md](VM_VALIDATION.md) にまとめてあります。

---

## 8. SSHポートが22でない場合

SSHを22以外で運用する場合は、probeが実際のポートに接続することを確認してください。
agentがいるホストでは通常、自動検出します。agentがいないホストでは設定が必要です。
22番のまま接続すると、正常なSSHサービスも障害として報告されます。

### 8.1 agentがいるホスト

通常は何もしなくて構いません。
agentが`/etc/ssh/sshd_config`を読み、`Port` / `ListenAddress host:port`から
実際のポートを検出してcontrollerへ報告します。

確認

```bash
sentinel doctor --json | python3 -c 'import json,sys; print(json.load(sys.stdin).get("hostname"))'
# controller側で、報告されたポートを確認
sentinel entity show <hostname> --json | python3 -c 'import json,sys; print(json.load(sys.stdin))'
```

`sshd_config`が読めない、あるいはポートが別の場所で設定されている場合は
明示します。

```toml
[agent]
controller_address = "head01:7443"
ssh_port = 2222        # sshd_config から読めない場合のみ
```

優先順位は次のとおりです。

```text
[agent] ssh_port  >  sshd_config の Port  >  既定値 22
```

### 8.2 agentがいないホスト（設定で宣言するホスト）

自分で報告できないため、必ず明示してください。

```toml
[[entities]]
type = "host"
name = "filesrv01"
addresses = ["10.0.0.10"]
ports = { ssh = 2222 }
capabilities = ["storage.nfs.server", "observer.peer"]
```

### 8.3 ホストごとにポートが異なる場合

`ports`はentityごとの設定です。混在して構いません。

```toml
[[entities]]
type = "host"
name = "filesrv01"
ports = { ssh = 2222 }

[[entities]]
type = "host"
name = "filesrv02"
# 22のまま。portsを書かない
```

### 8.4 確認方法

```bash
# 期待どおりのポートを呼び出しているか
sentinel entity show <hostname>       # ssh component が HEALTHY か
sentinel diagnose                     # SSH_SERVICE_FAILURE が出ていないか
```

`SSH_SERVICE_FAILURE`が全ホストに出る場合、ポート設定を疑ってください。

---

## 9. その他の非標準構成

### 9.1 agentのポートを変える

既定は7444です。変更する場合

```toml
[agent]
listen = "0.0.0.0:9444"
```

agentは自分のポートをregistrationで報告するため、
controller側に追記する必要はありません。
peerも正しいポートに接続します。

agentがいないホストに対して指定する場合のみ

```toml
ports = { agent = 9444 }
```

### 9.2 controllerのポートを変える

```toml
# controller側
[controller]
listen = "0.0.0.0:8443"

# agent側
[agent]
controller_address = "head01:8443"
```

### 9.3 Slurm NodeNameとhostnameが異なる

設定は不要です。Sentinelは両者を別のものとして扱い、
`NodeHostName`で対応付けます。

### 9.4 Slurmの設定ファイルが標準の場所にない

```toml
[discovery.slurm]
enabled = true
scontrol_path = "/opt/slurm/bin/scontrol"
```

allowlistはファイル名で照合するため、
`/opt/slurm/bin/scontrol`は許可され、`/opt/scontrol/rm`は許可されません。

### 9.5 capabilityの自動検出が期待と違う

```bash
sentinel doctor    # 判定理由を確認
```

そのうえで上書きします。

```toml
[capabilities]
"storage.nfs.server" = "force"     # 検出結果によらず ON
"storage.smart"      = "disable"   # 検出結果によらず OFF
"storage.zfs"        = "enable"    # 検出が何も言わなかった場合に ON
```

優先順位

```text
disable  >  force  >  runtime discovery  >  enable / role hint
```

### 9.6 TLS

TLSは組み込みです。reverse proxyは不要です。

平文のままでも動作しますが、cluster credentialはbearer tokenなので、
wireを読める者は全agentになりすませます。
隔離された管理ネットワーク以外ではTLSを設定してください。

#### 9.6.1 証明書の準備

既存のPKIで発行してください。Sentinelは証明書を生成しません
（CAの管理と監査は、既存の証明書発行手順で行います）。

controllerの証明書には、agentが接続に使う名前またはアドレスを
SANに入れてください。

```bash
# 例: 手元のCAで発行する場合
openssl x509 -req -in controller.csr -CA ca.crt -CAkey ca.key \
  -extfile <(printf "subjectAltName=DNS:controller.example,IP:10.0.0.10") \
  -days 825 -out controller.crt
```

配置とパーミッション

```bash
install -d -m 0755 /etc/sentinel/tls
install -m 0644 ca.crt         /etc/sentinel/tls/ca.crt
install -m 0644 controller.crt /etc/sentinel/tls/controller.crt
install -m 0600 -o sentinel -g sentinel controller.key /etc/sentinel/tls/controller.key
```

#### 9.6.2 TLSのみ（サーバー認証）

```toml
# controller
[tls]
cert = "/etc/sentinel/tls/controller.crt"
key  = "/etc/sentinel/tls/controller.key"
```

```toml
# agent
[agent]
controller_address = "controller.example:7443"

[tls]
ca = "/etc/sentinel/tls/ca.crt"
```

`[tls]`にクライアント側の設定が1つでもあると、
`host:port`は`https://`として解釈されます。
`controller_address`にschemeを書いた場合はそちらが優先されます。

#### 9.6.3 mutual TLS（推奨）

tokenが漏れても耐えられる構成はこれだけです。
証明書を持たないクライアントはtokenを使った認証に進めません。

```toml
# controller
[tls]
cert      = "/etc/sentinel/tls/controller.crt"
key       = "/etc/sentinel/tls/controller.key"
client_ca = "/etc/sentinel/tls/ca.crt"     # これを書くと client 証明書は必須
```

```toml
# agent
[tls]
ca          = "/etc/sentinel/tls/ca.crt"
client_cert = "/etc/sentinel/tls/agent.crt"
client_key  = "/etc/sentinel/tls/agent.key"
```

agent用の証明書はホストごとに発行してください
（1枚を全ホストで共有すると、1台の侵害が全体の侵害になります）。

#### 9.6.4 IPアドレスで接続する場合

controllerの証明書が名前しか持たず、agentがIPで接続する場合

```toml
[agent]
controller_address = "10.0.0.10:7443"

[tls]
ca          = "/etc/sentinel/tls/ca.crt"
server_name = "controller.example"   # 証明書上の名前
```

`server_name`は「アドレスで接続するが、証明書上の名前で検証する」ための
設定です。`controller_address`が既に名前の場合は使えません（エラーになります）。

#### 9.6.5 PKIがまだ無い場合

```toml
[tls]
insecure_skip_verify = true
```

これはTLSで接続先の正当性を確認できなくなります。接続を横取りできる攻撃者は
任意の証明書を提示でき、credentialはそのまま読まれます。
起動のたびに警告が出ます。暫定措置としてのみ使ってください。

#### 9.6.6 確認

```bash
# 設定の検証。certだけ指定しkeyがない場合などはエラーになる
sentinel config check

# controllerがTLSでlistenしているか
journalctl -u sentinel-controller | grep "controller listening"
#   -> tls=true

# 証明書チェーンの確認
openssl s_client -connect controller.example:7443 \
  -CAfile /etc/sentinel/tls/ca.crt </dev/null
```

TLSの証明書や秘密鍵が読めない場合、controllerは起動に失敗します。
設定した暗号化が有効にならないまま運用されるのを防ぐためです。

詳細は [SECURITY.md](SECURITY.md) と
[CONFIGURATION.md](CONFIGURATION.md) の`[tls]`を参照してください。

### 9.6.7 firewallで開けるポート

peer同士が観測しあうため、ノード間で以下が通る必要があります。

| ポート | 用途 | 開けないとどうなるか |
| --- | --- | --- |
| SSHのポート（22とは限らない） | 到達性probeとSSH probe | ホストが到達不能に見える |
| 7444 | agentのhealth endpoint | `agent` componentがUNAVAILABLEのまま |
| 7443（→ controllerのみ） | agentからの報告 | agentが登録できない |

DROPではなくREJECTにするか、明示的に許可してください。
DROPされた場合、probeはタイムアウトと区別できません。

到達性probeは設定済みのSSHポートに接続します（22固定ではありません）。
agentが`sshd_config`から自動検出して報告するので、通常は設定不要です。

### 9.7 NICが複数ある場合（VLAN・bridge・複数fabric）

peerがこのホストをprobeするアドレスは、agentが自動検出します。
ただし 「どのNICがクラスタ内通信を担っているか」は自動では分かりません。
使用するNICはクラスタの構成で決まるため、ホストの情報だけでは判別できません。

```
$ ip -o addr show
1: lo       inet 127.0.0.1/8
2: eno1  inet6 fe80::5054:ff:fe12:3456/64
6: vlan103   inet 192.0.2.32/24
7: vlan102   inet 192.0.2.20/24     ← クラスタ内通信はこれ
8: vlan101   inet 192.0.2.10/24
9: wg0      inet 10.0.0.1/24
```

このホストを調べても、`vlan102`が答えだと分かる手がかりはありません。
そのためSentinelは候補が複数あることを報告し、選択を求めます。

```bash
sentinel doctor
```

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

指定してください。全ノードで同じ1行が使えます。

```toml
[agent]
interface = "vlan102"
```

NAT越しなどホスト自身から見えないアドレスの場合は直接指定します。

```toml
[agent]
address = "203.0.113.9"
```

agentがいないホストは、これまでどおり`[[entities]]`の`addresses`で宣言します。

```toml
[[entities]]
type = "host"
name = "filesrv01"
addresses = ["192.0.2.30"]
```

#### 指定しないとどうなるか

「物理NICに見えるもののうち名前順で最初」が選ばれます。
上の例では`vlan101`です。多くの場合これは間違いです。

除外されるものは決まっています（ここは自動で正しく処理されます）。

* loopbackアドレス（`127.0.0.0/8`、`::1`）
* link-local（`169.254.0.0/16`、`fe80::/10`）
* `lo`インターフェース上の全アドレス、WSLの`10.255.255.254/32`のように、
  loopbackアドレスではないが誰からも到達できないもの
* `docker*` / `br-*` / `veth*` / `virbr*` / `wg*` / `tailscale*`などは後順位

#### 指定したNICにアドレスが無い場合

アドレスを報告しません。別のNICにフォールバックしません。
運用者が選ばなかったネットワークにpeer全員を向けるのが、
この設定で防ぎたい障害そのものだからです。

controllerはホスト名にフォールバックし、`doctor`が理由と実在する候補を表示します。

### 9.8 databaseの保持期間

controllerのdatabaseは書き込み一方で、既定では
observation 14日 / transition 90日 / 解決済みincident 180日でpruneされます。
実測でホスト1台あたり1日約170 MB増えるため、
既定ならホストあたり約2.4 GBで頭打ちになります。

長期保存が必要な場合

```toml
[retention]
observations       = "60d"
resolved_incidents = "never"     # incident は消さない
```

ディスクが小さい場合

```toml
[retention]
observations = "3d"
interval     = "15m"
```

`sentinel prune --dry-run`で、実行前に削除量を確認できます。
運用手順は [OPERATIONS.md](OPERATIONS.md) を参照してください。

### 9.9 ストレージ構成は書かなくてよい（例外は3つ）

NFSの依存関係を設定ファイルに書く必要はありません。

agentは毎サイクル自分のマウント表を報告しています。controllerはそこから
ファイルサーバーのホストentity、ストレージentity、`provides`、`uses_storage`を
すべて組み立てます。ノードを増やしても、マウント先を変えても、
設定ファイルは触りません。

```bash
sentinel discover && sentinel dependency list
```

止めたい場合は`[discovery.nfs] enabled = false`です。

#### 手で書く必要がある3つの場合

1. マウントがIPで書かれていて、そのIPを持つホストをSentinelが知らない

`10.0.0.9:/data`のようなマウントは、そのアドレスを登録しているホストが
いれば自動で結び付きます。いなければentityは作られません。
アドレスはidentityではないため（[ADR 0001](adr/0001-deterministic-entity-identity.md)）、
`10.0.0.9`という名前のentityを捏造すると、そのマシンが後から自分の名前で
登録したときに同じマシンに2つのidentityができてしまうからです。

未解決のアドレスを省略せず、`sentinel discover`が報告します。

```
NFS mounts that could not be tied to a known host:
  10.0.0.9  mounted by node01, node02
```

そのホストを宣言すれば、以降は自動で解決されます。

```toml
[[entities]]
type = "host"
name = "the-fileserver"
addresses = ["10.0.0.9"]
```

2. NFS以外の共有ストレージ

Lustre、GPFS、オブジェクトストレージなど。導出はNFSのマウント表を
見ているだけなので、それ以外は従来どおり宣言します。

```toml
[[entities]]
type = "storage"
name = "lustre-scratch"

[[dependencies]]
from = "storage/lustre-scratch"
to   = "host/mds01"
type = "provides"

[[dependencies]]
from = "host/node01"
to   = "storage/lustre-scratch"
type = "uses_storage"
```

3. どのノードもマウントしていないが監視したいファイルサーバー

誰もマウントしていなければマウント表に現れないため、導出されません。

手で書いた宣言は導出結果と併存します。打ち消し合いません。

#### ストレージentityの健全性はどこから来るか

ストレージentityにはprobeを実行する相手がいません（それは「概念」であって
マシンではないため）。健全性は提供元ホストのexport検査から導かれます。

ここで使うのはサーバー側の検査だけです。1台のホストがファイルサーバーでも
NFSクライアントでもありうるので（scratchをexportしつつ他所のhomeを
マウントする計算ノード）、両者を混ぜるとクライアント側のマウント詰まりが
「このホストのNFS提供に障害がある」と報告され、調査対象を誤る原因になります。

### 9.10 controllerにagentを同居させる

推奨します。controllerのホスト（多くはヘッドノード）にagentを
入れていないと、そのホストは外から到達性を見られるだけになります。
CPUもメモリもNFSマウントもjournalも見えません。
`slurmctld`を実行するヘッドノードも、agentがいなければ内部の状態を確認できません。

同居させるときは設定ファイルのパスを分けます。既定のままだと
agentのinstallがcontrollerの`config.toml`を上書きします。

```bash
sudo sentinel install agent --config /etc/sentinel/agent.toml
```

これで衝突しません。

| | controller | agent |
| --- | --- | --- |
| 設定 | `/etc/sentinel/config.toml` | `/etc/sentinel/agent.toml` |
| unit | `sentinel-controller.service` | `sentinel-agent.service` |
| ポート | 7443 | 7444 |
| 状態ファイル | `sentinel.db` | `spool.db` |
| credential | `/etc/sentinel/token`（共用） | 同左 |

credentialは設定ファイルの隣を見に行くので、同じディレクトリに置く限り
自動的に共用されます。agentのinstallがcredentialを作ったり
入れ替えたりすることはありません。

書き換えるのは2行です。

```bash
sudo sed -i 's/^environment = .*/environment = "my-cluster"/; s/^controller_address = .*/controller_address = "127.0.0.1:7443"/' /etc/sentinel/agent.toml
```

```bash
sudo systemctl daemon-reload && sudo systemctl enable --now sentinel-agent
```

同居させると、そのホストについて次が自動的に解決します。

- SSHポートが22以外でも、agentが`sshd_config`から読んで報告する
  （§8の手作業が不要になる）
- そのホストのNFSマウントが依存グラフに入る（§9.9）
- CPU・メモリ・journal・systemdサービスが見えるようになる

> Ansibleの`agents`グループには入れないでください。
> ロールは`/etc/sentinel/config.toml`に書き込むため、
> controllerの設定が上書きされてcontrollerが止まります。
> このホストだけは上の手順で個別に入れてください。

---

## 10. 導入後の確認

```bash
# 全agentが登録されたか
curl -fsS http://localhost:7443/v1/health

# 全entityが想定どおりか
sentinel status

# 依存関係が実構成と一致するか
sentinel dependency list

# observerが3台付いているか。付いていないentityは明示される
sentinel peers

# 誤検知が出ていないか
sentinel diagnose
```

### 導入直後に期待される状態

| 状態 | 意味 |
| --- | --- |
| 多くが`HEALTHY` | 正常 |
| agent未導入のホストが`UNKNOWN` | 正常。観測していないものをhealthyとは呼びません |
| `sentinel diagnose`が空 | 正常 |
| `SLURM_ONLY_DEGRADATION` | 実際にDRAINされているノードがあれば正常 |

### 誤検知が出た場合

| 症状 | 疑うところ |
| --- | --- |
| 全ホストに`SSH_SERVICE_FAILURE` | SSHポート（[§8](#8-sshポートが22でない場合)） |
| ファイルサーバーに`SLURM_*` | 通常起きません。起きた場合は報告してください |
| 個別の`NFS_CLIENT_FAILURE`が多発 | ストレージの依存関係が検出・設定されているか |
| 到達性の診断が一切出ない | observer不足（`sentinel peers`） |

---

## 11. 設定ファイルテンプレート

通常は`sentinel install`または`sentinel config init`が生成するファイルを
使ってください。全設定が既定値のまま説明つきで書き出され、
書き換えが必要な行には`CHANGE-ME`が入っています。

```bash
sentinel config init --role controller --output /etc/sentinel/config.toml
sentinel config init --role agent      --output /etc/sentinel/config.toml
sentinel config init --role agent --dry-run   # 中身だけ見る
```

以下は、生成物を待たずに構成を先に検討したい場合の参考です。
同じものが [`docs/templates/`](templates/) にもあります。

### 11.1 Controller (`/etc/sentinel/config.toml`)

```toml
config_version = 1

# 全ホストで一致させること
environment = "example-lab"

[controller]
listen = "0.0.0.0:7443"
inventory_interval = "5m"
# controller自身も観測点として動作する。
# firewallなどでcontrollerからの通信経路が制限される場合のみfalseにする
observe = true

[database]
path = "/var/lib/sentinel/sentinel.db"

[peer_monitoring]
# 1 entityあたりのobserver数。
# 0にすると到達性の診断ができなくなる
degree = 3

[discovery.slurm]
enabled = true
# scontrolがPATHにない場合のみ
# scontrol_path = "/opt/slurm/bin/scontrol"

[notification]
# 導入初期は "critical" にして様子を見るのも可
min_severity = "warning"

# [[notification.webhooks]]
# name = "ntfy"
# url  = "https://ntfy.example.org/cluster-sentinel"

# ---------------------------------------------------------------------------
# Scheduler entity
#
# SlurmのClusterNameに合わせておくと分かりやすい。
# 書かない場合は "slurm" になる。
# ---------------------------------------------------------------------------
[[entities]]
type = "scheduler"
name = "example_cluster"

# ---------------------------------------------------------------------------
# Slurmの外にあるホスト
#
# Slurm自動検出では見つからないため、ここで宣言する。
# agentを入れる予定であっても、先に書いておいてよい（mergeされる）。
# ---------------------------------------------------------------------------
[[entities]]
type = "host"
name = "filesrv01"
addresses = ["10.0.0.10"]
capabilities = ["storage.nfs.server", "observer.peer"]
labels = { role = "fileserver", rack = "r01" }
# SSHが22以外の場合のみ
# ports = { ssh = 2222 }

[[entities]]
type = "host"
name = "filesrv02"
addresses = ["10.0.0.11"]
capabilities = ["storage.nfs.server", "observer.peer"]
labels = { role = "fileserver", rack = "r01" }

# ---------------------------------------------------------------------------
# Storage entityと依存関係
#
# NFSについては、書く必要がありません。
# agentが報告するマウント表から、ファイルサーバーのホストentity、ストレージentity、
# provides、uses_storageがすべて自動で導出される。
# ノードがマウント先を変えても、この設定ファイルを触る必要はない。
#
#   確認:  sentinel dependency list
#   停止:  [discovery.nfs] enabled = false
#
# 手で書く必要があるのは §9.9に挙げた3つの例外だけ。
# 手で書いたものは導出結果と併存する（打ち消し合わない）。
# ---------------------------------------------------------------------------

# ---------------------------------------------------------------------------
# capabilityの上書き（必要な場合のみ）
# ---------------------------------------------------------------------------
# [capabilities]
# "storage.nfs.server" = "force"
```

### 11.2 Agent (`/etc/sentinel/config.toml`)

全agentホストで同じ内容で構いません。
capabilityは自動検出されます。

```toml
config_version = 1

# controllerと一致させること
environment = "example-lab"

[agent]
controller_address = "head01:7443"
spool_path = "/var/lib/sentinel/spool.db"

# health endpoint。peerがここを見て
# agentのみの停止とホスト全体の停止を区別する
listen = "0.0.0.0:7444"

# SSHが22以外で、かつsshd_configから読めない場合のみ
# ssh_port = 2222

# UI上のグループ分けにのみ使う。probeを有効化しない
roles = ["compute"]

# ---------------------------------------------------------------------------
# このホストをpeer observerにする場合
#
# observerは互いに異なる障害ドメインから選ぶこと。
# ---------------------------------------------------------------------------
[capabilities]
"observer.peer" = "force"
```

### 11.3 最小構成（動作確認用）

```toml
# controller
config_version = 1
environment = "example-lab"

[controller]
listen = "0.0.0.0:7443"

[database]
path = "/var/lib/sentinel/sentinel.db"

[discovery.slurm]
enabled = true
```

```toml
# agent
config_version = 1
environment = "example-lab"

[agent]
controller_address = "head01:7443"
```

---

## 12. チェックリスト

### 導入前

- [ ] environment名を決めた
- [ ] controllerを置くホストを決めた
- [ ] agentを置くホストと、置かないホストを決めた
- [ ] observerにするホストを3台以上決めた（異なる障害ドメイン）
- [ ] 各ホストのアーキテクチャを確認した（`uname -m`。x86_64 / ARM混在ならホストごと）
- [ ] クラスタ内通信のNICを確認した（複数あるなら [§9.7](#97-nicが複数ある場合vlanbridge複数fabric)）
- [ ] ストレージの依存関係を把握した
- [ ] SSHポートを確認した（22以外なら [§8](#8-sshポートが22でない場合)）
- [ ] agent → controllerのネットワーク到達性を確認した

### Controllerのホスト

- [ ] releaseからバイナリを取得し、`sha256sum -c`が通った
- [ ] `/usr/local/bin/sentinel`に配置し、`sentinel version`が動いた
- [ ] `sentinel`ユーザーを作成した（`install`より先に）
- [ ] `sudo sentinel install controller`を実行した
- [ ] `chown -R sentinel:sentinel /etc/sentinel`した
- [ ] `config.toml`の`CHANGE-ME`を書き換えた（`environment`）
- [ ] 自動検出されないホストとストレージの依存関係を宣言した
- [ ] `sentinel config check`が通った
- [ ] サービスが起動し、`systemctl status`が正常
- [ ] `curl .../v1/health`が応答した

### Agentを置く各ホスト

- [ ] そのアーキテクチャ用のバイナリを`/usr/local/bin/sentinel`に配置した
- [ ] `sentinel`ユーザーを作成した（`install`より先に）
- [ ] `sudo sentinel install agent`を実行した（警告が出ていないこと)
- [ ] controllerの`/etc/sentinel/token`を配置した（0400、`sentinel`所有）
- [ ] `chown -R sentinel:sentinel /etc/sentinel`した
- [ ] `config.toml`の`CHANGE-ME` 2行を書き換えた
- [ ] `sentinel doctor`の報告アドレスに`!`の警告がない（あれば`interface`を指定）
- [ ] `sentinel doctor`のcapabilityが想定どおり
- [ ] `sentinel config check`が通った
- [ ] サービスが起動し、`systemctl status`が正常

### agentを置かないホスト

- [ ] controllerの`config.toml`に`[[entities]]`として宣言した
- [ ] SSHポートが22以外なら`ports = { ssh = ... }`を書いた
- [ ] IPを`addresses`で明示した

### 全体

- [ ] `curl .../v1/health`で全agentが登録されている
- [ ] `sentinel status`が実構成と一致する
- [ ] `sentinel dependency list`が実ストレージ構成と一致する
- [ ] `sentinel peers`でobserverの付いていないentityがない
- [ ] `sentinel diagnose`に誤検知がない
- [ ] 数日おいて誤検知が出ないことを確認した
- [ ] notificationをテスト宛先で確認した
