# セキュリティ

本書はSentinelが想定する攻撃、v1の対策と制限を説明します。

## 監視対象への操作

Sentinelは監視・診断のみを行います。
v1では、ホストの再起動、`systemctl restart`、mount/remount、
`scontrol update`など、監視対象を変更する操作を実行しません。
controllerが侵害されても、SentinelのRPCを使って任意の操作は実行できません。

## 脅威モデル

| 攻撃者 | 想定される行為 | 対策 |
| --- | --- | --- |
| クラスタ内の通信を盗聴できる者 | agentとcontrollerの通信を読み取る | TLS。導入時に設定する |
| クラスタ内の通信に介入できる者 | 偽の観測結果を送る、controllerになりすます | クラスタ共有credentialによる認証とTLSの証明書検証 |
| 監視対象ホストのローカルユーザー | agentを使って権限を取得する | 専用ユーザー、読み取り専用の検査、遠隔コマンド実行の禁止 |
| controllerを侵害した者 | agentへ任意のコマンドを送る | RPCにコマンド実行機能を設けない |

クラスタのroot権限を既に取得した攻撃者に対する防御は、対象外です。

## 遠隔コマンド実行

通信プロトコルには、実行コマンドを指定する型がありません。
probeはバイナリにコンパイルされており、controllerがagentへ送れる指示は再登録だけです。
次のテストで、コマンドの指定と実行経路がないことを確認します。

* `protocol::tests::the_protocol_cannot_express_a_command_to_run`は、通信形式に`command`、`exec`、`script`、`shell`、`argv`が含まれないことを検査します。
* `api::tests::there_is_no_route_that_executes_anything`は、`/v1/exec`などの経路が404を返すことを検査します。

CLIにも同様のテストがあり、`sentinel exec`、`run`、`restart`、`reboot`、
`drain`、`resume`といったサブコマンドがないことを検査します。

## 認証

credentialなしでは`sentinel controller`と`sentinel agent`は起動しません。
v1では、クラスタ単位で共有するbearer credentialを使います。

```bash
# 推奨。権限を設定でき、プロセス一覧に値が表示されない
export SENTINEL_TOKEN_FILE=/etc/sentinel/token

# 環境変数で指定する場合
export SENTINEL_TOKEN=...
```

`SENTINEL_TOKEN_FILE`を`SENTINEL_TOKEN`より優先します。
指定したファイルが読めない場合は、設定ミスを検出するため、環境変数へ切り替えません。

credentialの生成例

```bash
head -c 32 /dev/urandom | base64
```

32文字未満のcredentialには警告を出します。
credentialの比較は一定時間で行い、処理時間から値を推測されるのを防ぎます。
`Debug`出力では値を伏せます。バイナリにsecretを埋め込みません。

### 共有credentialの制限

共有credentialを取得した攻撃者は、任意のホストになりすまして偽の観測結果を送信できます。
mutual TLSを設定すると、tokenに加えてクライアント証明書が必要になります。
ノード別credentialは未実装です。`IMPLEMENTATION.md` §65に従い、
認証を使う側の処理を変えずに方式を追加できる設計です。

## 通信の保護

TLSは組み込みです。`[tls]`に`cert`と`key`を設定すると、controllerはTLSで待ち受けます。
設定項目は[設定リファレンス](CONFIGURATION.md#tls)を参照してください。

| 構成 | 設定 | 保護の範囲 |
| --- | --- | --- |
| 平文HTTP | なし | 通信の暗号化なし。隔離された管理ネットワークを前提とする |
| TLS | `cert`と`key` | 通信を暗号化し、credentialの盗聴を防ぐ |
| mutual TLS | 上記に`client_ca`を追加 | tokenだけでは認証できず、クライアント証明書も必要になる |

`client_ca`を設定すると、クライアント証明書の提示は必須です。
提示を任意にすると、証明書を持たない攻撃者も接続できるためです。
証明書や秘密鍵が読めない場合は起動に失敗し、平文HTTPへ切り替えません。
平文HTTPで待ち受ける場合は、起動時に警告を出します。

### `insecure_skip_verify`

証明書を検証せずに接続する設定です。
通信に介入できる攻撃者が任意の証明書を提示し、credentialを取得できてしまいます。
PKIの準備前に接続を試すために用意しています。有効な間は警告を出します。

### 証明書の発行

Sentinelは証明書を自動生成しません。
CAの管理と監査をSentinelとは別に行うため、既存の証明書発行手順を使ってください。

### agentのhealth endpoint

agentのhealth endpointにはTLSを適用していません。
credentialを送信せず、agentが応答しているかを確認する情報だけを返します。
実装は`src/agent/rpc.rs`にあります。

## 外部コマンド

probeが起動できるプログラムは、`src/command/allowlist.rs`の許可リストに限定します。
照合するのは実行ファイル名です。`/opt/slurm/bin/scontrol`は許可しますが、
`/opt/scontrol/rm`は許可しません。
`sh`、`bash`、`rm`、`dd`、`mount`、`reboot`、`scancel`、`srun`を
許可リストに追加しないことを、テストで確認します。
外部コマンドにはタイムアウトと出力サイズの上限を設定し、出力を省略した場合は記録します。

## 権限

agentは専用の非特権ユーザーで実行することを推奨します。
次の機能では追加権限が必要になる場合があります。

| 機能 | 必要な権限 |
| --- | --- |
| SMART / NVMe | デバイスの読み取り権限 |
| 一部のjournal読み取り | `systemd-journal`グループ |
| BMC / IPMI。v1対象外 | 別系統の管理ネットワークへのアクセス |

権限が不足する検査は`UNSUPPORTED`を返します。
読み取れないことだけを理由に、対象を`FAILED`とは判定しません。

## systemdの権限制限

生成するunitには、`IMPLEMENTATION.md` §85に従って次を設定します。

```ini
NoNewPrivileges=true
PrivateTmp=true
ProtectHome=true
ProtectSystem=strict
ProtectKernelTunables=true
ProtectControlGroups=true
RestrictSUIDSGID=true
ReadWritePaths=/var/lib/sentinel /run/sentinel
```

## ログへの出力

credentialやtokenの値はログに書きません。`Debug`出力では値を伏せます。
外部コマンドの大量出力もログに書かず、保持上限を設けて観測の根拠として保存します。

## 脆弱性の報告

セキュリティ上の問題は公開issueに記載せず、リポジトリ管理者へ直接連絡してください。
