# 設定ファイルテンプレート

通常はこれらを使う必要はありません。
`sentinel install <role>`または`sentinel config init --role <role>`が、
全設定を既定値のまま説明つきで書き出した設定ファイルを生成します。

```bash
sentinel config init --role agent --dry-run
```

ここにあるのは、バイナリを持ち込む前に構成を検討したい場合や、
特定の構成（mutual TLSなど）を先に確認したい場合の参考です。
`CHANGE-ME`を実際の値に置き換えてください。

| ファイル | 配置先 | 用途 |
| --- | --- | --- |
| `controller.toml` | controllerホストの`/etc/sentinel/config.toml` | controller |
| `agent.toml` | 各agentホストの`/etc/sentinel/config.toml` | agent |
| `agent-nonstandard-ssh.toml` | 同上 | SSHが22以外の場合 |
| `controller-mtls.toml` | controllerホストの`/etc/sentinel/config.toml` | mutual TLS構成のcontroller |
| `agent-mtls.toml` | 各agentホストの`/etc/sentinel/config.toml` | mutual TLS構成のagent |
| `minimal-controller.toml` | 同上 | 動作確認用の最小構成 |

`*-mtls.toml`はTLS部分だけを示した最小構成です。
entity / dependencyの宣言は`controller.toml`から持ってきてください。
証明書の準備手順は [../DEPLOYMENT.md](../DEPLOYMENT.md) §9.6にあります。

配置後、起動前に必ず検証してください。

```bash
sudo -u sentinel sentinel config check
```

手順の全体は [../DEPLOYMENT.md](../DEPLOYMENT.md) を参照してください。
