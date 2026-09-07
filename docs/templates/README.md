# 設定ファイルテンプレート

そのままコピーして使えるテンプレートです。
`CHANGE-ME` を実際の値に置き換えてください。

| ファイル | 配置先 | 用途 |
| --- | --- | --- |
| `controller.toml` | controller host の `/etc/sentinel/config.toml` | controller |
| `agent.toml` | 各 agent host の `/etc/sentinel/config.toml` | agent |
| `agent-nonstandard-ssh.toml` | 同上 | SSH が 22 以外の場合 |
| `minimal-controller.toml` | 同上 | 動作確認用の最小構成 |

配置後、**起動前に必ず**検証してください。

```bash
sudo -u sentinel sentinel config check
```

手順の全体は [../DEPLOYMENT.md](../DEPLOYMENT.md) を参照してください。
