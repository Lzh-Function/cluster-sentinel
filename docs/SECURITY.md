# セキュリティ

本書は Sentinel の脅威モデル、v1 で提供する保証、
そして **意図的に提供しない** ものを記述します。

## 設計上の立場

Sentinel は監視・診断のみを行います。
**v1 では監視対象を一切変更しません**
（reboot、`systemctl restart`、mount/remount、`scontrol update` を実行しない）。
これはセキュリティ上の性質でもあります —
controller を掌握しても、それが直ちに cluster の制御権にはなりません。

## 脅威モデル

想定する攻撃者:

| 攻撃者 | 想定される行為 | 対策 |
| --- | --- | --- |
| cluster network 上の受動的観測者 | agent–controller 間トラフィックの読み取り | TLS（デプロイ時に設定。後述） |
| cluster network 上の能動的攻撃者 | 偽 observation の注入、controller への成りすまし | cluster credential による認証 |
| 監視対象 host 上のローカルユーザー | agent 経由での特権昇格 | agent は read-only、専用ユーザー、remote 実行なし |
| controller を掌握した攻撃者 | agent への命令 | **RPC に命令が存在しない**（後述） |

想定 **しない** 攻撃者: cluster の root を既に取得している者。
その段階では Sentinel は防御境界ではありません。

## Remote 実行は存在しない

これは規約ではなく、**型で保証された性質** です。

Sentinel の wire protocol には、実行すべきコマンドを表現できる型が存在しません。
Probe はバイナリへ static compile されており、controller が agent へ送れるのは
「登録し直せ」という指示のみです。

以下 2 つのテストがこれを強制します。

* `protocol::tests::the_protocol_cannot_express_a_command_to_run`
  — wire 表現に `command` / `exec` / `script` / `shell` / `argv` が現れないこと。
* `api::tests::there_is_no_route_that_executes_anything`
  — `/v1/exec` 等の route が 404 であること。

CLI 側にも同様のテストがあり、`sentinel exec` / `run` / `restart` /
`reboot` / `drain` / `resume` といったサブコマンドが存在しないことを検査します。

## 認証

**未認証での動作は提供しません。** credential 無しで
`sentinel controller` / `sentinel agent` を起動すると、
デフォルトへフォールバックせずに起動を拒否します。

v1 は cluster 単位の bearer credential を用います。

```bash
# 推奨: ファイルから読む（パーミッションを設定でき、process table に出ない）
export SENTINEL_TOKEN_FILE=/etc/sentinel/token

# 代替: 環境変数
export SENTINEL_TOKEN=...
```

`SENTINEL_TOKEN_FILE` が `SENTINEL_TOKEN` より優先されます。
ファイルが読めない場合は環境変数へフォールバック **しません** —
設定ミスを隠すことになるためです。

credential の生成例:

```bash
head -c 32 /dev/urandom | base64
```

32 文字未満の credential は警告されます。
credential 比較は constant-time で行い、
prefix がタイミングから漏れないようにしています。
`Debug` 出力は常に redact されるため、ログへ漏れません。

**バイナリへ secret を埋め込むことはありません。**

### 既知の限界と将来の方向

cluster 単位の共有 credential は *下限* であり、目標ではありません。
これを掌握した攻撃者は、任意の host になりすまして偽 observation を注入できます。
`IMPLEMENTATION.md` §65 の要求どおり、
per-node credential および mTLS へ、呼び出し側を変えずに置換できる形にしてあります。

## 転送路の保護

v1 の agent は controller address をそのまま URL として解釈します。
`host:port` は `http://` として扱われます。

**production では TLS を終端してください。** 選択肢:

* `https://` URL を controller address に設定し、TLS を終端する reverse proxy を置く
* 監視トラフィックを信頼された管理 network に限定する

TLS 無しの場合、cluster network 上の受動的観測者に credential が露出します。

## 外部コマンドの実行

Probe が起動できるプログラムは allowlist に載ったものだけです
（`src/command/allowlist.rs`）。

* allowlist は **ファイル名** で照合します。
  `/opt/slurm/bin/scontrol` は許可されますが、`/opt/scontrol/rm` は許可されません
  — ディレクトリがプログラムを認可することはありません。
* `sh` / `bash` / `rm` / `dd` / `mount` / `reboot` / `scancel` / `srun` は
  **決して** allowlist に載りません。テストで検査しています。
* すべての外部コマンドは timeout と出力サイズ上限のもとで実行されます。
  truncate した場合はその事実を observation に記録します。

## 権限

agent は専用の非特権ユーザーで動作させることを推奨します。
以下は追加権限が必要になり得ますが、いずれも任意です。

| 機能 | 必要な権限 |
| --- | --- |
| SMART / NVMe | デバイスへの読み取り権限 |
| 一部の journal 読み取り | `systemd-journal` グループ |
| BMC / IPMI（v1 対象外） | out-of-band network アクセス |

これらが無い場合、該当 probe は `UNSUPPORTED` を返します。
`FAILED` ではありません — 見る権限が無いことは、壊れていることとは異なります。

## systemd hardening

生成する unit には以下を設定します（`IMPLEMENTATION.md` §85）。

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

## ログに書かないもの

* credential・token の類（`Debug` 実装が redact します）
* 外部コマンドの大量出力（evidence として保存し、保持上限を適用します）

## 脆弱性の報告

セキュリティ上の問題は public issue ではなく、
リポジトリ管理者へ直接連絡してください。
