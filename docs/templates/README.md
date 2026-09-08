# 設定ファイルテンプレート

そのままコピーして使えるテンプレートです。
`CHANGE-ME` を実際の値に置き換えてください。

| ファイル | 配置先 | 用途 |
| --- | --- | --- |
| `controller.toml` | controller host の `/etc/sentinel/config.toml` | controller |
| `agent.toml` | 各 agent host の `/etc/sentinel/config.toml` | agent |
| `agent-nonstandard-ssh.toml` | 同上 | SSH が 22 以外の場合 |
| `controller-mtls.toml` | controller host の `/etc/sentinel/config.toml` | mutual TLS 構成の controller |
| `agent-mtls.toml` | 各 agent host の `/etc/sentinel/config.toml` | mutual TLS 構成の agent |
| `minimal-controller.toml` | 同上 | 動作確認用の最小構成 |

`*-mtls.toml` は TLS 部分だけを示した最小構成です。
entity / dependency の宣言は `controller.toml` から持ってきてください。
証明書の準備手順は [../DEPLOYMENT.md](../DEPLOYMENT.md) §9.6 にあります。

配置後、**起動前に必ず**検証してください。

```bash
sudo -u sentinel sentinel config check
```

手順の全体は [../DEPLOYMENT.md](../DEPLOYMENT.md) を参照してください。
