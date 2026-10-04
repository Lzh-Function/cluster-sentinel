分散クラスタの監視・障害検知・原因診断システムです。
controller・agent・CLIを一つのバイナリで実行できます。

## v1.0.7の変更

### controllerの観測読み取りを軽量化

最新の観測を取得するSQLとインデックスを変更しました。診断のたびに過去の観測全体を並べ替えていた処理をなくし、必要な観測だけを読み取ります。DBの読み取りが長引いてagentからの送信が滞り、観測が期限切れになる原因を減らします。

### NVIDIA GPUを外したノードの監視対象を更新

`nvidia-smi`がないノードではGPUを監視対象から外します。過去のGPU観測が残っていても、現在の状態や診断には使いません。監視中にコマンドがなくなった場合も、次の検査でGPU監視を停止します。

コマンドがあるのにドライバのエラーやタイムアウトが発生した場合は、異常として扱います。コマンドを戻した場合はagentを再起動するとGPU監視を再開し、新しい観測から判定します。

### 更新時に監視状態と未解決の障害をすべて初期化

更新スクリプトはcontrollerを停止し、監視状態、未解決の障害、送信待ち通知をすべて初期化してから起動し直します。確認済み・復旧途中・通知を抑止中の障害も対象です。

更新前の観測は、遅れて届いた場合も現在の判定から外します。新しい観測がそろうまでUNKNOWNとなり、異常が続く場合は新しい障害IDで検出します。以前の障害は「初期化で終了」として履歴に残し、初期化による復旧通知は送りません。

観測履歴、設定、監視対象の登録情報は残します。通常のサービス再起動では、更新後の状態を引き継ぎます。手動で初期化する場合は、controllerを停止して`sentinel state reset`を実行してください。

中央ノードに`systemd-journal`グループがある場合は、更新スクリプトが`sentinel`ユーザーへログの閲覧権限を追加します。

## 更新方法

中央ノードで、普段Ansibleを実行するユーザーから実行してください。

```bash
cd /path/to/cluster-sentinel
git pull --ff-only
./deploy/update.sh v1.0.7 -- -K
```

下流ノードでsudoを使う指定は、playbookの`become: true`に含まれています。`-K`でsudoのパスワードを入力します。スクリプト全体には`sudo`を付けません。

Vaultを使う場合は、`-- --ask-vault-pass -K`を付けます。SSHのパスワードも必要な場合は、`-- -k -K`を付けてください。

中央と下流を別々に更新する場合は、次のように実行します。`--agents-only`の場合も、中央ノードの状態と未解決の障害を初期化するためcontrollerを再起動します。

```bash
./deploy/update.sh v1.0.7 --parent-only
./deploy/update.sh v1.0.7 --agents-only -- -K
```

スクリプトは中央ノードの`/usr/local/bin/sentinel`と`/etc/sentinel/config.toml`を使います。詳しい手順は[運用ガイド](https://github.com/mizuno-group/cluster-sentinel/blob/v1.0.7/docs/OPERATIONS.md#アップグレード)を参照してください。

## 配布ファイル

| ファイル | 対象 |
| --- | --- |
| `sentinel-x86_64-unknown-linux-musl` | x86_64 |
| `sentinel-aarch64-unknown-linux-musl` | ARM64 |

各バイナリにSHA-256の確認用ファイルを添付します。静的リンクでビルドするため、ホストのglibcのバージョンに依存しません。
