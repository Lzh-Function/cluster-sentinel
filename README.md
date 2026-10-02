# Cluster Sentinel

計算クラスタの監視・障害検知・原因診断ツール。バイナリ1つで動きます。

複数のノードの観測結果を比較し、ホスト全体の障害、通信経路の障害、
特定サービスの停止を区別します。診断結果には原因の候補と根拠を示します。

| 症状 | 診断 |
| --- | --- |
| ノードに繋がらない | `HOST_UNREACHABLE`、複数の観測元から到達不能 |
| ノードに繋がらない | `PATH_SPECIFIC_NETWORK_FAILURE`、経路の一部だけが切れている |
| ノードに繋がらない | `SSH_SERVICE_FAILURE`、ホストは応答するがSSHが停止 |
| ノードに繋がらない | `SENTINEL_AGENT_FAILURE`、agentだけが停止 |
| ノードが使えない | `SLURMD_SERVICE_FAILURE`、`slurmd`だけ停止 |
| ノードが使えない | `SLURM_ONLY_DEGRADATION`、マシンは正常、Slurm上でdrain |
| 何台も同時に不調 | `SHARED_STORAGE_FAILURE`、共有ストレージ1台が原因 |
| ファイルサーバーが不調 | `NFS_SERVICE_FAILURE`、ホストは応答するがNFSの提供が停止 |

各ノードが互いを監視します。1台からの到達失敗だけでは、
ホスト全体が到達不能になったとは判定しません。

`POWER_OFF`という診断結果はありません。ネットワークに応答がないだけでは、
電源断、NIC故障、経路障害を区別できないためです。
電源状態を確認するには、BMCなどの別系統の情報が必要です。

## はじめに読むもの

[導入手順](docs/GETTING_STARTED.md)

初めて導入する方向けの手順です。作業時間の目安は30分です。

## ドキュメント

### 使う人向け

| 目的 | ドキュメント |
| --- | --- |
| はじめて触る | [GETTING_STARTED.md](docs/GETTING_STARTED.md) |
| コマンドの一覧と使い分け | [COMMANDS.md](docs/COMMANDS.md) |
| 本番クラスタへ本格導入する | [DEPLOYMENT.md](docs/DEPLOYMENT.md) |
| 日々の運用、障害時の読み方、アップグレード | [OPERATIONS.md](docs/OPERATIONS.md) |
| 設定項目のリファレンス | [CONFIGURATION.md](docs/CONFIGURATION.md) |
| 何をどこまで守るのか | [SECURITY.md](docs/SECURITY.md) |
| 多数のノードへ一括配布する | [deploy/ansible/](deploy/ansible/) |

### 中身を知りたい人向け

| 目的 | ドキュメント |
| --- | --- |
| 設計の考え方とコードの構成 | [ARCHITECTURE.md](docs/ARCHITECTURE.md) |
| 開発環境・テスト・疑似クラスタ | [DEVELOPMENT.md](docs/DEVELOPMENT.md) |
| アーキテクチャの要件と設計原則 | [SPEC.md](docs/SPEC.md) |
| 実装の要件・開発段階・テスト要件 | [IMPLEMENTATION.md](docs/IMPLEMENTATION.md) |
| 設計判断とその理由 | [adr/](docs/adr/) |
| Dockerでは検証できない項目 | [VM_VALIDATION.md](docs/VM_VALIDATION.md) |

## インストール

コンパイルは不要です。x86_64とARM64の静的バイナリを配布しています。

```bash
ARCH=$(uname -m) && curl -fsSL -o sentinel "https://github.com/mizuno-group/cluster-sentinel/releases/latest/download/sentinel-${ARCH}-unknown-linux-musl" && chmod +x sentinel && sudo mv sentinel /usr/local/bin/
```

ソースからビルドする場合

```bash
cargo build --release
```

## コマンド

controllerもagentもCLIも、すべて同じバイナリのサブコマンドです。

```bash
sentinel status                   # 今どうなっているか
sentinel diagnose                 # 障害の原因と判断の根拠
sentinel explain                  # 検査内容と監視元の割り当て
sentinel audit                    # 動いているはずの検査が動いているか
sentinel entity observations <n>  # その判断の元になった生の観測
sentinel notify test              # 通知先に実際に届くか
sentinel maintenance start <n>    # 計画作業中の通知を止める
```

一覧と使い分けは [docs/COMMANDS.md](docs/COMMANDS.md) にまとめてあります。

`explain`を除く上記のコマンドは、`--json`で機械可読出力になります。
`sentinel status`は異常があればexit code 2を返すので、
そのままhealth checkに使えます。

## 互換性について（v1.0以降）

1.xでは、次の互換性を維持します。

| | 約束 |
| --- | --- |
| `config_version` | `1`のまま。既存の設定ファイルはそのまま動きます |
| `protocol_version` | `1`のまま。controllerとagentのバージョンが混在していても動きます（アップグレード中は必ずそうなります） |
| `--json`の出力 | フィールドの追加はありますが、削除・改名はしません |
| 終了コード | 変えません（[COMMANDS.md](docs/COMMANDS.md#終了コード)） |

人が読む前提のテキスト出力は、読みやすさのために変わることがあります。
スクリプトからは`--json`を使ってください。

## 動作確認

Dockerの疑似クラスタに対して、受け入れ項目33件が自動で走ります。

```bash
cd dev/compose && ./scripts/acceptance
```

## ライセンス

MIT.
