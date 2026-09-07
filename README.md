# Cluster Sentinel

汎用クラスタインフラ監視・障害検知・インシデント診断基盤。

Sentinel は Slurm 監視ツールではありません。研究用計算クラスタを
`ManagedEntity` の集合（capability と dependency を持つグラフ）としてモデル化し、
複数地点から観測することで、「応答しなくなった」ではなく **「なぜ壊れているのか」**
を導出します。

区別できることを目標とする状況:

| 状況 | 診断結果 |
| --- | --- |
| どこからも host へ到達できない | `HOST_UNREACHABLE` |
| 一部の observer からのみ到達できない | `PATH_SPECIFIC_NETWORK_FAILURE` |
| Host は応答するが SSH のみ異常 | `SSH_SERVICE_FAILURE` |
| Host は応答するが Sentinel agent のみ異常 | `SENTINEL_AGENT_FAILURE` |
| Host は正常で `slurmd` のみ停止 | `SLURMD_SERVICE_FAILURE` |
| Host は正常だが Slurm 上は DRAIN | `SLURM_ONLY_DEGRADATION` |
| control plane 自体が異常 | `SLURM_CONTROL_PLANE_FAILURE` |
| fileserver の export service 障害 | `NFS_SERVICE_FAILURE` |
| server は正常で 1 client の mount のみ異常 | `NFS_CLIENT_FAILURE` |
| 同一 storage に依存する複数 client が同時異常 | `SHARED_STORAGE_FAILURE` |
| 実ハードウェアと scheduler 設定の不一致 | `RESOURCE_CONFIGURATION_MISMATCH` |

意図的に存在しない診断結果: **`POWER_OFF`**。
network が沈黙していることは電源状態の証拠ではないため、
out-of-band な証拠なしにこれを主張しません。

## 現在の状況

[docs/IMPLEMENTATION.md](docs/IMPLEMENTATION.md) §94 の milestone 順に構築中です。

| Milestone | 範囲 | 状態 |
| --- | --- | --- |
| M0 | Repository / core domain / config / migration / CLI skeleton | 完了 |
| M1 | Passive controller、Slurm discovery、`sentinel status` | 完了 |
| M2 | Agent、protocol、spool | 完了 |
| M3 | Docker Compose 疑似クラスタ | 完了 |
| M4 | Host / network / SSH / agent 監視 | 完了 |
| M5 | Slurm 診断 | 完了 |
| M6 | Storage / NFS | 完了 |
| M7 | GPU | 完了 |
| M8 | Peer monitoring | 完了 |
| M9 | Diagnosis / incident correlation | 未着手 |
| M10 | Notification / operations | 未着手 |

## ビルド

```bash
cargo build --release
```

成果物は単一バイナリです。controller、agent、各 CLI 動詞はすべて
そのサブコマンドとして提供されます。

```bash
sentinel version
sentinel config check
sentinel discover
sentinel status
sentinel entity list
sentinel entity show <name>
sentinel dependency list
sentinel diagnose
sentinel peers
```

いずれも `--json` を付ければ機械可読出力になります。
`sentinel status` は異常があれば exit code 2 を返すため、
health check やスクリプトから利用できます。

## ドキュメント

| ドキュメント | 内容 |
| --- | --- |
| [docs/SPEC.md](docs/SPEC.md) | アーキテクチャ仕様（source of truth） |
| [docs/IMPLEMENTATION.md](docs/IMPLEMENTATION.md) | 実装契約・milestone・テスト要件 |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | コードの構成と、その理由 |
| [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) | 開発環境・テスト・疑似クラスタ |
| [docs/CONFIGURATION.md](docs/CONFIGURATION.md) | 設定リファレンス |
| [docs/adr/](docs/adr/) | 自明でなかった設計判断の記録 |

## ライセンス

MIT.
