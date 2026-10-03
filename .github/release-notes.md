分散クラスタの監視・障害検知・原因診断システムです。
controller・agent・CLIを一つのバイナリで実行できます。

## v1.0.5の変更

### 復旧を確認するまで障害を維持

ノード停止後、観測の期限切れだけで復旧通知が送られる問題を修正しました。原因と考えられる対象と、影響を受けた対象すべての復旧を、新しい正常な観測で確認します。観測が途絶えたり、状態がUNKNOWNになったりしただけでは解決しません。

状態はprobeと監視元ごとに判定します。別のprobeの正常結果、再送された観測、過去の観測では異常を解消しません。判定途中の回数も保存し、controllerの再起動後に引き継ぎます。

### 通知の再送と復旧状態の表示

通知候補を障害の記録と同時に保存します。送信に失敗した通知はcontrollerの再起動後も再送し、配信済みの通知は繰り返しません。障害が再発した場合は、前回の未配信の復旧通知を取り消します。

原因が復旧しても、影響を受けた対象の復旧を確認できていなければ「復旧途中」と表示します。汎用webhookの`resolved`が`true`になるのは、復旧完了時だけです。

### 日本語の通知とSlackの確認手順

通知の見出し、異常の説明、確認手順を日本語にしました。各手順に、実行するホストと出力で確認する項目を記載しています。Sentinelの確認コマンドは、controllerで`sudo -u sentinel sentinel ...`として実行してください。

Slackでは、赤のCRITICAL通知の冒頭に`@channel`を付けます。復旧途中は黄、復旧完了は緑で表示し、復旧の通知にはメンションを付けません。長い手順はコマンドを途中で切らずに分割し、表示上限を超える場合は全文を確認するコマンドを表示します。

```toml
[[notification.webhooks]]
name = "ops"
url = "https://hooks.slack.com/services/..."
format = "slack"
```

通知経路を確認するには、controllerで次のコマンドを実行してください。`--severity critical`を付けると、テスト通知にも`@channel`が付きます。

```bash
sudo -u sentinel sentinel notify test --provider ops
```

Slurmの調査には登録されたNodeNameを使い、NFSの調査には観測したポートを使います。NFSのI/O検査が完了しない場合は、マウント情報、カーネルログ、待機中のプロセスの確認手順を表示します。

導入・運用ガイド、設計書、設定例も、検査条件と操作手順が分かる文章に書き直しました。

## ダウンロード

| ファイル | 対象 |
| --- | --- |
| `sentinel-x86_64-unknown-linux-musl` | x86_64 |
| `sentinel-aarch64-unknown-linux-musl` | ARM64 |

各バイナリにSHA-256の確認用ファイルを添付します。静的リンクでビルドするため、ホストのglibcのバージョンに依存しません。同じアーキテクチャのcontrollerとagentに、同じバイナリを配れます。

以下はx86_64の例です。ARM64ではファイル名を対応するものに置き換えてください。

```bash
sha256sum -c sentinel-x86_64-unknown-linux-musl.sha256
sudo install -m 0755 sentinel-x86_64-unknown-linux-musl /usr/local/bin/sentinel
sudo -u sentinel sentinel version
```

## 更新

controllerとagentをv1.0.5に更新してください。更新前に設定を確認し、controller、agentの順に再起動します。既存のdatabaseには起動時にマイグレーションを適用します。

```bash
sudo -u sentinel sentinel --config /etc/sentinel/config.toml config check
sudo systemctl restart sentinel-controller
# 各agentホストで実行
sudo systemctl restart sentinel-agent
```

Ansibleロールの既定の取得バージョンもv1.0.5です。詳しい手順は[運用ガイド](https://github.com/mizuno-group/cluster-sentinel/blob/v1.0.5/docs/OPERATIONS.md)を参照してください。

## 実環境での確認

実際のSlack宛への配信、BMCによる電源状態の確認、NFSの応答待ちが続く実機、実GPUでの動作は、利用環境で確認してください。自動復旧やリモートでの任意コマンド実行は行いません。
