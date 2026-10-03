分散クラスタの監視・障害検知・原因診断システムです。
controller・agent・CLIを一つのバイナリで実行できます。

## v1.0.6の変更

### Slack通知を一つの本文に整理

通知全文が通常の本文欄と色バー付きの本文欄に二重に表示される問題を修正しました。色バー付きの本文にまとめて表示します。赤のCRITICAL通知には、冒頭に`@channel`を付けます。

確認コマンドはコードブロックで表示します。実行先、確認する内容、コマンドを改行し、手順の間に空行を入れました。診断の確度、診断文、障害IDなどの情報も段落を分けています。

以前の診断に保存された`systemctl`や`journalctl`などのコマンドもコードで表示します。引用符、バッククォート、日本語を含むコマンドを、そのままコピーできます。長い通知でもコマンドを途中で切らず、表示上限を超えた場合は全文を確認するコマンドを表示します。

### 中央ノードと下流ノードの更新をスクリプトで実行

[`deploy/update.sh`](https://github.com/mizuno-group/cluster-sentinel/blob/v1.0.6/deploy/update.sh)を追加しました。中央ノードを更新し、サービスの起動と状態の読み取りを確認してから、Ansibleで下流ノードを更新します。バージョンを指定して繰り返し使えます。

中央ノードでは、取得したバイナリのSHA-256、バージョン、既存の設定を検証してから置き換えます。前のバイナリは`/usr/local/bin/sentinel.previous`に保存します。中央ノードでagentも起動中なら、そのサービスも再起動します。

中央ノードの設定・DB・認証情報を置き換える処理は行いません。下流ノードの設定と認証情報は、既存のAnsibleロールが通常どおり配布します。ノードの設定を変更するときは、インベントリやgroup_varsを編集してください。

### 監視の担当変更後も復旧を確認

障害時に根拠へ含めた正常な観測について、監視の担当変更後も元の監視元からの更新を待ち続け、障害記録が解決しない場合がある問題を修正しました。現在の監視元から正常な結果を続けて得られれば、その観測を確認できます。異常だった観測には、異常を報告した監視元からの復旧確認を引き続き求めます。観測が古くなっただけでは、障害記録を解決しません。

## 更新方法

中央ノードで、普段Ansibleを実行するユーザーから実行してください。スクリプト全体には`sudo`を付けません。

```bash
cd /path/to/cluster-sentinel
git pull --ff-only
./deploy/update.sh v1.0.6 -- -K
```

下流ノードでsudoを使う指定は、playbookの`become: true`に含まれています。`-K`でsudoのパスワードを入力します。

Vaultを使っている場合は、次のように実行します。

```bash
./deploy/update.sh v1.0.6 -- --ask-vault-pass -K
```

SSHのパスワードも必要な場合は、`-- -k -K`を付けます。別のインベントリは`--inventory /path/to/inventory.ini`で指定できます。

中央と下流を別々に更新する場合は、次のように実行します。

```bash
./deploy/update.sh v1.0.6 --parent-only
./deploy/update.sh v1.0.6 --agents-only -- -K
```

スクリプトは中央ノードの`/usr/local/bin/sentinel`と`/etc/sentinel/config.toml`を使います。配置を変えている場合は、[運用ガイド](https://github.com/mizuno-group/cluster-sentinel/blob/v1.0.6/docs/OPERATIONS.md#アップグレード)の手動手順を参照してください。

## 配布ファイル

| ファイル | 対象 |
| --- | --- |
| `sentinel-x86_64-unknown-linux-musl` | x86_64 |
| `sentinel-aarch64-unknown-linux-musl` | ARM64 |

各バイナリにSHA-256の確認用ファイルを添付します。静的リンクでビルドするため、ホストのglibcのバージョンに依存しません。

Slackの表示を確認するには、中央ノードで次のコマンドを実行してください。`--severity critical`を付けると、テスト通知にも`@channel`が付きます。

```bash
sudo -u sentinel sentinel notify test --provider ops
```
