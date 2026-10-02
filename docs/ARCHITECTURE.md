# アーキテクチャ

本書はコードの構成、実装上の制約、設計判断の理由を説明します。
監視対象や機能の要件、設計原則は[仕様書](SPEC.md)に記載しています。
個別の設計判断とその理由は[ADR](adr/)に記録します。

## 監視と診断の仕組み

Sentinelはホスト、サービス、ストレージなどをentityとして管理します。
各entityの機能をcapability、依存関係をdependencyとして記録し、
複数地点の観測結果と依存関係から障害の原因を判定します。

## パイプライン

```text
Probe ──▶ Observation ──▶ State ──▶ Diagnosis ──▶ Incident
```

観測から障害の管理までを5段階に分け、各段階の役割を限定します。

| 段階 | 役割 | 制約 |
| --- | --- | --- |
| Probe | 検査を実行し、測定結果を返す | `HOST_UNREACHABLE`などの診断を生成しない |
| Observation | 対象、観測元、時刻、測定結果を記録する | 保存済みの観測結果を書き換えない |
| State | 観測結果から各componentの状態を判定する | 原因の判定は行わない |
| Diagnosis | 障害の原因と根拠を示す | 通知先は決めない |
| Incident | 関連する診断を対応すべき障害としてまとめる | 観測されていない事実を根拠にしない |

この分離により、運用者が診断の根拠を検証できます。
たとえばファイルサーバーが原因という診断では、使用したルール、観測結果、
観測時刻、観測元まで確認できる必要があります。
probeは測定した事実を返し、原因の判定は診断ルールで行います。

## モジュール依存方向

```text
integrations (probes/, inventory/)
        │
        ▼
   observation ──▶ state ──▶ diagnosis ──▶ incident
```

coreから技術固有の実装へ依存させません。
たとえばcoreのモジュールに`use crate::probes::slurm::...`と書くことは禁止します。
Slurm・NFS・NVIDIAに関する情報は、capability文字列、JSONペイロードを持つ
observation、依存関係のedgeを通じて渡します。
SPEC.md §146のNFSからCephFSへの変更も、設定と連携機能の追加で対応し、
coreの変更を不要にする設計です。

## リポジトリ構成

```text
src/
  entity/         監視対象と識別子
  capability/     機能の検出結果と設定の優先順位
  dependency/     依存グラフと閉路を考慮した探索
  observation/    書き換えない観測記録
  state/          状態判定、連続失敗・成功回数の管理
  diagnosis/      診断エンジンとルール
  incident/       関連する診断の統合と障害の状態管理
  probes/         検査インターフェースと実装
  inventory/      監視対象の自動検出と統合
  integrations/   Slurmなどの技術固有の処理
  agent/          agent、自ホストとpeerの検査、spool、RPC
  controller/     controller、API、外部からの検査、peer割り当て
  protocol/       通信プロトコル
  command/        外部コマンドの実行時間と許可リストの管理
  persistence/    SQLiteへの保存と読み出し
  notification/   通知、重複排除、maintenance
  config/         設定、優先順位、検証
  cli/            サブコマンド
```

`SPEC.md` §183の推奨構成に`integrations/`を追加しています。
`scontrol`の出力解析を`integrations/slurm/`にまとめ、
`inventory/`と`probes/`から共通で使います。
この配置でも、coreから技術固有の実装には依存しません。

## Entityの識別方法

識別キーは`(environment, entity_type, canonical_name)`です。
内部IDはこのキーから計算するUUIDv5です。
詳しくは[ADR 0001](adr/0001-deterministic-entity-identity.md)を参照してください。

IPアドレスは識別キーに使いません。一つのホストが管理用、ストレージ用、
相互接続用など複数のアドレスを持つことがあり、アドレスは変更されるためです。
`entity_addresses`には、一つのentityに対して複数のアドレスを保存します。
Slurmの`NodeName`とホスト名も区別し、対応関係を保存します。

## Probeの有効化

probeはcapabilityに基づいて有効化します。
roleは運用上のグループ分けや既定値の提示に使うラベルで、単独ではprobeを起動しません。
roleの変更で必要なprobeが停止すると、障害が起きるまで監視漏れに気づけない場合があります。
capabilityの優先順位は`src/capability/resolve.rs`に実装し、各規則をテストしています。

## 依存グラフ

依存グラフは閉路を許容します。たとえばcontrollerがストレージに依存し、
そのストレージを提供するホストがcontroller上のschedulerに依存する構成です。
`src/dependency/graph.rs`の探索処理は訪問済みの対象を記録し、無限に探索しません。
閉路と自己ループの両方をテストしています。

同じ依存先を持つ対象のグループは、グラフから導出します。
`group_by_shared_upstream`は共通の依存先へ到達する対象を求める汎用処理です。
NFS診断では、閉路による誤った統合を防ぐため、直接の`uses_storage`関係を使います。
ストレージの共有範囲を追加しても、coreの変更は不要です。

## 診断の制約

一つの観測元での失敗だけでは、ホスト全体の障害と判定しません。
一部の観測元で失敗し、他の観測元から接続できる場合は、経路の障害として扱います。

ネットワークに応答がないだけでは、電源状態を判定できません。
ネットワークの観測から出す診断は`HOST_UNREACHABLE`までです。
`POWER_OFF`の判定にはBMCやPDUなど別系統の情報が必要ですが、v1では収集しません。

## Probeの安全制約

| 制約 | 理由 |
| --- | --- |
| 外部コマンドに必ずタイムアウトを設定する | 応答しない`scontrol`などが検査全体を停止させるのを防ぐ |
| 出力サイズを制限し、省略した事実を記録する | `journalctl`などが大量の出力を返すことがある |
| ファイルシステムへの検査はmountごとに同時1本まで | 応答しないNFSシステムコールを繰り返し起動しない |
| probeのpanicでデーモンを停止させない | 他の検査を継続する |
| RPCに遠隔コマンド実行を設けない | agentは事前に実装したprobeだけを実行する |

## 自動復旧

v1では、ホストの再起動、サービスの再起動、再マウント、`scontrol update`を実行しません。
診断結果と読み取り専用の調査コマンドを提示し、復旧作業は運用者が判断します。
推奨操作はdiagnosisのデータとして保持するため、将来、運用者が明示的に
有効化した場合に実行する機能を追加できる設計です。

## 診断ルール

coreの診断はRustで実装したルールによって行います。
同じ観測結果と依存関係からは同じ診断結果を生成し、適用したルールのIDを記録します。
将来、言語モデルを人向けの要約に使う可能性はありますが、障害の判定には使いません。

## 永続化

`src/persistence/`を通じて、WALモードのSQLiteを使います。
マイグレーションはバイナリに含め、起動時に適用します。

`controllers`の`environment`には一意制約を設けていません。
v1はcontroller 1台構成ですが、将来のHA対応でこの制約を解除する必要がないようにしています。
保存済みの`observations`は更新しません。観測IDはagentが生成し、spoolから
再送しても重複挿入しないようにしています。

## 実環境の設定値

実環境のホスト名、アドレス、partition名、ストレージ構成はcoreに埋め込みません。
設定ファイル、`fixtures/`、`dev/compose/`、テストで扱います。
実環境固有の名前が`src/`に混入していないことを、テストで確認します。
