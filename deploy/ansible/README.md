# Ansibleによる展開

複数ノードへの配布は、このAnsibleロールで自動化できます。
このロールは [DEPLOYMENT.md](../../docs/DEPLOYMENT.md) の §3〜§6を自動化します。

Sentinel側にAnsible固有のものは一切ありません。ここにあるのは
「バイナリを置き、`sentinel install`を実行し、設定を配る」だけです。
別の構成管理ツールを使っているなら、同じ手順を移植してください。

## 前提

* controllerにAnsibleがインストールされている
* controllerから各ノードへSSHでログインでき、`sudo`が使える
* controller側で`sentinel install controller`が済んでおり、
  `/etc/sentinel/token`が存在する

## どのユーザーでSSHするか

ユーザー名は秘密情報ではないので、`ansible-vault`は要りません。
決まる順序は次のとおりです。

| 優先 | 指定方法 |
| --- | --- |
| 1 | Ansibleインベントリの`ansible_user = li` |
| 2 | `ansible-playbook -u li` |
| 3 | `ansible.cfg`の`remote_user` |
| 4 | `~/.ssh/config`の`User` |
| 5 | 実行している人のローカルユーザー名 |

Ansibleは既定で`ssh`コマンドを使い、`~/.ssh/config`の設定を適用します。
普段`ssh node01`で接続できる場合は、その設定を使えます。
Ansibleインベントリに同じユーザー名やポートを重ねて指定する必要はありません。

```
# ~/.ssh/config
Host node* filesrv*
    User li
    Port 22
```

疎通確認

```bash
ansible -i inventory.ini agents -m ping -K
```

## パスワードが必要な場合

SSHにも`sudo`にもパスワードが要る、という環境は珍しくありません。両方扱えます。

```bash
ansible-playbook -i inventory.ini site.yml --ask-pass --ask-become-pass
```

| オプション | 何のパスワードか | 備考 |
| --- | --- | --- |
| `--ask-pass` | SSHログイン | `sshpass`が必要（`apt install sshpass`） |
| `--ask-become-pass`（`-K`） | `sudo` | |

実行開始時に1回ずつ聞かれ、以降は全ノードで使い回されます。
全ノードで同じパスワードであることが前提です。

### SSHは鍵にすることを強く推奨します

パスワード認証は毎タスクで使われるうえ、`sshpass`は
Ansible公式が非推奨としており、`PasswordAuthentication no`の環境では
そもそも使えません。鍵を配るのは1回で済みます。

```bash
ssh-keygen -t ed25519 -C "ansible@controller"    # まだ無ければ
for n in node01 node02 node03; do ssh-copy-id "$n"; done
```

これで`--ask-pass`が不要になり、`sudo`のパスワードだけになります。

```bash
ansible-playbook -i inventory.ini site.yml -K
```

### ansible-vaultが必要なのはどこか

| 状況 | 必要なもの |
| --- | --- |
| SSH鍵、`sudo`パスワード共通 | `-K`だけ |
| SSHもパスワード、両方共通 | `--ask-pass -K`（+ `sshpass`） |
| ノードごとに`sudo`パスワードが違う | ansible-vault（下記。1ファイルで済みます） |
| ユーザー名がノードごとに違う | `~/.ssh/config`か`ansible_user`（vault不要） |

全ノードでSSHとsudoのパスワードが共通なら、`--ask-pass -K`で実行できます。
ノードごとにパスワードが異なる場合は、ansible-vaultで管理します。

### ノードごとにsudoパスワードが違う場合

`--ask-become-pass`は1つしか受け付けないので、異なるパスワードはvaultで管理します。
ノードごとにファイルを分ける必要はありません。
暗号化ファイル1つに全ノード分を辞書で持たせます。

`group_vars/agents/vars.yml`（平文）

```yaml
ansible_become_password: "{{ vault_become_passwords[inventory_hostname] }}"
```

この2つは両方作るか、両方作らないかです。`vars.yml`だけ置くと
`vault_become_passwords`が未定義になり、playbookは最初のタスクで止まります。
そのためリポジトリには`.example`として置いてあり、
既定ではどちらも存在しません（vaultを使わない環境はそのまま動きます）。

`group_vars/agents/vault.yml`（暗号化）

```yaml
vault_become_passwords:
  node01: "..."
  node02: "..."
  fileserver01: "..."
```

作り方

```bash
cp group_vars/agents/vars.yml.example  group_vars/agents/vars.yml
cp group_vars/agents/vault.yml.example group_vars/agents/vault.yml
$EDITOR group_vars/agents/vault.yml          # 実際のパスワードを書く
ansible-vault encrypt group_vars/agents/vault.yml
```

以降の編集は`ansible-vault edit group_vars/agents/vault.yml`で行います
（自動で復号し、保存時に再暗号化します）。

実行

```bash
ansible-playbook -i inventory.ini site.yml --ask-vault-pass -K
```

2ファイルに分けるのは慣習です。暗号化ファイルは`git diff`でも
`grep`でも中身が見えないので、「どの変数がどこから来るのか」を
平文側に残しておくと後から読めます。

#### `-K`も併せて必要です

credentialを読むタスクはcontroller上でrootとして実行します
（`delegate_to: localhost` + `become: true`）。
localhostはAnsibleインベントリに居ないので`vault_become_passwords`が効かず、
ここだけ`--ask-become-pass`の値が使われます。

`ansible_become_password`変数が設定されているホストではそちらが優先されるため、
`-K`で入力した値はcontroller用、vaultの値は各ノード用、と自然に分かれます。

controllerのsudoパスワードを入力したくない場合は、
credentialの複製を自分で読める場所に置き、そちらを指してください。

```bash
sudo cp /etc/sentinel/token ~/sentinel-token
sudo chown "$USER" ~/sentinel-token && chmod 600 ~/sentinel-token
ansible-playbook -i inventory.ini site.yml --ask-vault-pass \
  -e sentinel_token_source=~/sentinel-token
```

この場合`-K`は不要になります。使い終わったら消してください。

#### vaultパスワードを毎回入力したくない場合

```bash
echo "vault のパスワード" > ~/.ansible-vault-pass
chmod 600 ~/.ansible-vault-pass
ansible-playbook -i inventory.ini site.yml \
  --vault-password-file ~/.ansible-vault-pass -K
```

vaultの中身を守っているのはこのファイルだけになります。
ホームディレクトリが他人から読めない、暗号化されている、
といった前提が置ける場合にのみ使ってください。

### sudoをパスワード無しにするという選択

各ノードで1回ずつsudoできるなら、そちらのほうが恒久的に楽です。
ただしAnsibleはpythonモジュールをrootで実行するため、
「sentinelコマンドだけNOPASSWD」では足りず、実質的に
その運用ユーザーの`NOPASSWD: ALL`が必要になります。

サイトのセキュリティ方針として許容できるかどうかで判断してください。
許容できない場合は、ansible-vaultでパスワードを管理してください。

### credentialの読み取りについて

`/etc/sentinel/token`はmode 0400、`sentinel`ユーザー所有です。
ロールはcontroller上でrootとして読み取り、各ノードへ配ります
（`slurp` + `become: true` + `delegate_to: localhost`）。
`-K`を渡していれば、この読み取りにもそのパスワードが使われます。

## 使い方

```bash
cd deploy/ansible
cp inventory.example.ini inventory.ini
$EDITOR inventory.ini          # ノード名と変数を書く
ansible-playbook -i inventory.ini site.yml
```

まず1台で試してください。

```bash
ansible-playbook -i inventory.ini site.yml --limit node02
```

`--check`を付けると、何も変更せずに差分だけ確認できます。

## このロールがすること

| 手順 | 対応する節 |
| --- | --- |
| releaseからバイナリを取得（アーキテクチャ別） | §3.1 |
| `/usr/local/bin/sentinel`に配置 | §3.2 |
| `sentinel`サービスユーザーを作成 | §6.1 |
| `sentinel install agent`を実行 | §6.1 |
| cluster credentialを配置（0400） | §6.2 |
| 設定ファイルを配置 | §6.3 |
| `sentinel config check`で検証 | §6.4 |
| サービスを起動 | §6.4 |

しないこと

* controllerの構築（1台なので手で行ってください）
* `[[entities]]`や依存関係の宣言（controller側の設定）
* NICの自動選択（`sentinel_interface`で指定してください）

## 変数

| 変数 | 既定 | 意味 |
| --- | --- | --- |
| `sentinel_version` | このリポジトリの版 | 取得するrelease |
| `sentinel_minimum_version` | `0.3.2` | このロールが必要とする最小版 |
| `sentinel_environment` | *(必須)* | controllerと一致させる |
| `sentinel_controller_address` | *(必須)* | `host:port` |
| `sentinel_interface` | *(未設定)* | クラスタ内通信のNIC名 |
| `sentinel_roles` | `[]` | UI上のグループ分け |
| `sentinel_observer` | `false` | このホストをpeer observerにするか |
| `sentinel_token_source` | `/etc/sentinel/token` | controller上のcredential |
| `sentinel_download_dir` | `/tmp` | 一時ファイルの置き場 |

## 何度実行しても同じ結果になります

新規のホストから3回連続で実行して確認しています。

```
1回目   changed=9
2回目   changed=0
3回目   changed=0
```

設定ファイルは毎回テンプレートから全体を書き直します（追記ではありません）。
そのため

* 何度実行しても内容は増えません
* 手で編集した内容は次回の実行で消えます。変更は
  `inventory.ini`か`group_vars/`に書いてください

credentialはcontrollerのものをそのまま配るだけで、生成も再生成もしません。

## controllerから共通設定を取得する

同じ設定を2箇所で保守する必要はありません。ロールは実行時に
controllerの`config.toml`を読み、揃っていなければならない設定を各ノードへ配ります。

| 設定 | 出所 |
| --- | --- |
| `environment` | controllerのconfig.toml |
| `[probes]`（監視頻度） | controllerのconfig.toml |
| `[tls]`の`ca` / `server_name` / `insecure_skip_verify` | controllerのconfig.toml |
| controllerの待ち受けポート | controllerのconfig.toml |
| `controller_address`のホスト部 | Ansibleインベントリ |
| `interface` / `roles` / observer指定 | Ansibleインベントリ（ホストごとに違うため） |
| TLSのクライアント証明書と鍵 | Ansibleインベントリ（ホストごとに違うため） |

読み取りにはcontroller自身のバイナリ（`sentinel config show --json`）を
使います。TOMLを別途パースするのではなく、デーモンが解釈するのと同じ値が
配られます。

### 監視頻度を変える

controllerの設定を変えて、playbookを実行するだけです。

```bash
sudo -u sentinel $EDITOR /etc/sentinel/config.toml
```

```toml
[probes."network.tcp"]
interval = "15s"
```

```bash
sudo systemctl restart sentinel-controller
ansible-playbook -i inventory.ini site.yml --ask-vault-pass -K
```

`[probes]`はcontrollerとagentの両方で必要です（controllerもremote probeを
自分で実行するため）。この仕組みにより、書くのはcontrollerの1箇所だけです。

### 食い違いはエラーになります

Ansibleインベントリに`sentinel_environment`を書いていて、controllerの値と違う場合は
実行が止まります。値が同じなら何も起きません。

```
sentinel_environment is wrong-lab but the controller says mizuno-group.
Remove it from the inventory: with sentinel_sync_from_controller the
controller decides, and two places to change it is what that setting
exists to avoid.
```

### 同期を切る場合

```bash
ansible-playbook -i inventory.ini site.yml -e sentinel_sync_from_controller=false
```

このとき`sentinel_environment`と`sentinel_probes`はAnsibleインベントリに書きます。

### controller自身には実行しないでください

ロールは設定ファイルをテンプレートから全体を書き直すため、controllerに
対して実行すると、controllerの設定がagentの設定へ置き換わります。
`sentinel-controller.service`があるホストは実行を拒否します。

controllerにagentを同居させる場合は、
[導入マニュアルの手順](../../docs/DEPLOYMENT.md#910-controllerにagentを同居させる)に従い、
agent用の設定を`/etc/sentinel/agent.toml`に分けてください。
バイナリとunitだけ配りたい場合は`sentinel_manage_config=false`を使います。

## 検証

```bash
ansible -i inventory.ini agents -b -a "sentinel doctor"
sudo -u sentinel sentinel status
sudo -u sentinel sentinel peers
```

`sentinel doctor`の報告アドレスに`!`の警告が出ていないか確認してください。
出ていれば`sentinel_interface`を設定して再実行します。

## バージョンについて

`sentinel_version`の既定値は、このリポジトリがビルドする版と一致します
（`tests/ansible_role.rs`が強制します）。古いreleaseを指定すると、
ロールが使う`install --binary`や`doctor --json`のアドレス系フィールドが
無いため、バイナリを配り終えたあとの`install`で失敗します。

そのため、バイナリを配置した直後にバージョンを検査し、
古ければ「command-lineの引数エラー」ではなく理由の分かるメッセージで止まります。

## アップグレード

中央ノードと下流ノードをまとめて更新する場合は、中央ノードで
[`deploy/update.sh`](../update.sh)を実行してください。
既存のインベントリとSSH設定を使い、中央ノードを先に更新します。

```bash
cd /path/to/cluster-sentinel
git pull --ff-only
./deploy/update.sh v1.0.6 -- -K
```

Vaultを使っている場合は`-- --ask-vault-pass -K`、
SSHのパスワードも必要な場合は`-- -k -K`を付けます。
インベントリを別の場所に置いている場合は、`--inventory /path/to/inventory.ini`で指定します。
中央ノードの更新が済んでいる場合は、`--agents-only`で下流だけを更新できます。
詳しい動作と手動での更新方法は[運用ガイド](../../docs/OPERATIONS.md#更新スクリプトを使う)を参照してください。

Ansibleを直接実行する場合は、先に中央ノードを更新してから、
`sentinel_version`を変えて再実行します。credentialはcontrollerのものを配布します。
agentの設定ファイルはテンプレートから書き直すため、変更はAnsibleインベントリやgroup_varsに記載してください。

```bash
ansible-playbook -i inventory.ini site.yml -K -e sentinel_version=v1.0.6
```

controllerを先に更新してください。protocol versionが同じであれば
混在状態でも動作します。
