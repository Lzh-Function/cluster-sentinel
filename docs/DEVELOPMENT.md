# 開発ガイド

## 前提環境

| 要件 | 備考 |
| --- | --- |
| Linux、またはWindows + WSL2 | 主開発環境として想定 |
| Rust toolchain | 1.82以降（`rustup`） |
| Docker Engine + Compose | M3以降、疑似クラスタに必要 |

実Slurmクラスタは開発環境ではありません。
日常の作業では、単体テスト、プロセス内のシミュレーション、Docker疑似クラスタで検証します。

## ビルドと検査

```bash
cargo build
```

次の3つの検査を通過することを確認します。CIでも同じ検査を実行します。

```bash
cargo fmt --check
```

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

```bash
cargo test --all
```

更新スクリプトを変更した場合は、次の検査も実行してください。CIでも実行します。
このテストは一時ディレクトリと模擬コマンドを使うため、稼働中のノードを変更しません。
Python 3.9以降が必要です。

```bash
bash -n deploy/update.sh
python3 tests/test_update_script.py
```

## テスト階層

| Level | 対象 | 必要なもの |
| --- | --- | --- |
| 1、unit / mock | domain model、config、パーサー、グラフ、state、rule、DB、protocol | なし |
| 2、in-process simulation | 人工的な観測から状態・診断・incidentを生成する | なし |
| 3、Docker疑似クラスタ | 実agent・実controller・実Slurm、障害注入 | Docker |
| 4、VM / 実クラスタ | 再起動、boot ID、NFS hard mount、D-state、実GPU | VMまたは実機 |

Level 1と2は、Rust toolchainさえあればDocker無しで必ず通ります。

## 疑似クラスタ

Docker Composeによる6コンテナの疑似クラスタが`dev/compose/`にあります。
controller 1、compute 3、ファイルサーバー2という構成です。

単純な構成では再現できなかった実機の不具合を検証するため、
次の3つの構成を含めています。

| 構成 | 検証する内容 |
| --- | --- |
| controllerホストがagentも動かす | 内側からも監視され、かつpeerの監視対象でもあるホスト。controllerは自分自身にprobeを実行しないので、この構成でのみ再現する監視漏れがある |
| 計算ノードの1台がストレージも提供する | 依存グラフに閉路ができる（ストレージ → そのホスト → scheduler → schedulerが乗るホスト → そのストレージ）。グラフを推移的に辿るルールは直接依存していない対象まで同じグループとして誤判定する |
| ヘッドノードがexportゼロでexportポートに応答する | NFSサーバのパッケージが入っているだけのホスト。誤検知の検証対象 |

いずれも実クラスタで使っている構成です。

```bash
cd dev/compose
./scripts/up
```

`up`はコンテナの起動だけでなく、controller APIの応答・Slurmのノード登録・
ストレージサービス・agent登録がすべて揃うまで待ってから戻ります。

```bash
./scripts/sentinel status          # クラスタ状態
./scenarios/stop-slurmd compute01  # 障害注入
./scripts/sentinel status          # 診断結果を確認
./scenarios/recover-all            # 復旧
./scripts/down                     # 停止
./scripts/reset                    # volume ごと作り直し
```

利用可能なシナリオ一覧と依存関係トポロジは
[`dev/compose/README.md`](../dev/compose/README.md) を参照してください。

Docker統合テストは、環境変数で有効にした場合だけ実行します（Dockerが無い環境でも
unit / simulationテストが通るようにするため）。実行するには

```bash
SENTINEL_DOCKER_TESTS=1 cargo test --test m3_pseudo_cluster -- --test-threads=1
```

## Dockerで保証できないもの

疑似クラスタはサービス / プロセス / ネットワーク障害を忠実に再現しますが、
以下は再現しません。

* 実機の再起動時の挙動とboot IDの変化
* カーネルのハードロックとD-state
* NFS hard-mountによるkernel stall
* 実systemdホストの挙動
* SMART / NVMe、GPU、NICのハードウェア障害
* IPMI / BMC、物理電源断

これらはLevel 4で検証し、
VMテスト要件として記録します。詳細は
[VM_VALIDATION.md](VM_VALIDATION.md) を参照してください。

## v1受け入れテスト

`docs/IMPLEMENTATION.md` §97の18段階を自動化してあります。

```bash
cd dev/compose
./scripts/acceptance
```

各段階で一つのサービスや経路に障害を起こし、正しい対象を診断することを確認します。
別の対象を誤診断しないことも、同じように検証します。
たとえばデーモンだけを停止した場合に、ホスト全体を`HOST_UNREACHABLE`と
判定していないことを確認します。

## リポジトリ構成

```text
src/
  entity/       ManagedEntity、identity
  capability/   Capability とその解決
  dependency/   有向グラフ、cycle-safe traversal
  observation/  immutable な probe 結果
  probes/       Probe interface（配下に integration）
  state/        導出された health、debounce
  diagnosis/    typed rule
  incident/     correlation と lifecycle
  persistence/  SQLite repository
  config/       設定、優先順位、検証
  cli/          サブコマンド
migrations/     SQL migration（順に適用）
fixtures/       parser test 用の実コマンド出力
tests/          integration / simulation / scenario test
dev/compose/    Docker 疑似クラスタ（M3 以降）
```

## 規約

* core / library層はtyped error（`thiserror`）、CLI境界は`anyhow`を使用します。
* `unwrap()` / `expect()`はテストと、失敗し得ないことが証明できる箇所に限ります。
* probeの失敗がデーモンをpanicさせてはなりません。
* 外部コマンドは必ずタイムアウトと出力サイズ制限のもとで実行します。
* `src/`にホスト名・アドレス・partition名・ストレージtopologyを書きません。
  テストデータ・設定・テストが正しい置き場所です。

## 実装済みのprobe

| Probe ID | 観測対象 | 必要capability | 実行場所 |
| --- | --- | --- | --- |
| `host.metrics` | load / memory / pressure / uptime / boot ID | `host.metrics` | local |
| `network.tcp` | ホストへの到達性 | 不要 | local / remote |
| `ssh.service` | SSH banner | `ssh.server` | local / remote |
| `sentinel.agent` | agentのhealth endpoint | `sentinel.agent` | remoteのみ |
| `systemd.unit` | systemd unitの状態 | `systemd` | local |
| `slurm.node` | schedulerから見たノード状態 | （Slurm自動検出） | controller |
| `slurm.controller` | 制御系の到達性 | （Slurm自動検出） | controller |
| `nfs.client.mount` | mount一覧・読み取り専用化 | `storage.nfs.client` | local |
| `nfs.client.io` | mountへの実I/O応答性 | `storage.nfs.client` | local（mountごとに同時1） |
| `nfs.server.port` | exportポートの応答 | `storage.nfs.server` | local / remote |
| `nfs.server.exports` | export一覧 | `storage.nfs.server` | local |
| `gpu.nvidia` | GPU一覧・温度・メモリ | `gpu.nvidia` | local |
| `journal.events` | kernel / service event | `journal.read` | local（同時1） |

probeの一覧、既定のスケジュール、説明は`src/probes/catalog.rs`で管理します。
`sentinel config init`が書き出す設定ファイルも、`config check`がprobe idの
妥当性を判定するのもここを見ています。probeを追加したらcatalogに追加してください
（`every_probe_in_the_tree_is_listed`テストが強制します）。

運用者が`[probes]`で頻度を変更できます。設定の上書きはprobeの
`ProbeDefinition`自体に適用されます。
いくつかのprobeは、このタイムアウトを外部コマンドの実行制限にも使います。
runnerだけに設定すると、probe内の制限と値が食い違うためです。

### journal probe

kernel event（OOM / I/O error / hung task / NVMe timeout / GPU Xid /
MCE / NFSサーバーnot responding / link down / thermal）を継続的に収集します。

on-demandではなく継続収集である理由は、
障害発生後はホストに接続できず、ログを取得できない場合があるためです
（`SPEC.md` §1「再起動後に原因情報が失われる」）。
収集できたイベントはcontrollerへ保存されるため、障害発生後も確認できます。

取得範囲は、次の4項目で制限します（`IMPLEMENTATION.md` §52）。

| 制限 | 値 |
| --- | --- |
| 時間 | 前回scan以降、最大15分 |
| priority | warning以上 |
| 行数 | 500 |
| バイト数 | 1 MiB |

matched eventは事実として記録するだけで、診断結果とは等価に扱いません
（`SPEC.md` §83）。計算ノードでのOOM killは多くの場合、
ジョブ実行に伴って発生します。

コンテナにはsystemdが無いため、
Docker疑似クラスタでは`journal.read`が検出されずprobeは動きません。
実systemd環境での検証はlevel 4です。

### NFS probeの安全性

hangしたNFS mountに触れたプロセスはuninterruptible sleepに入り、
killもできず、システムコールもcancelできません。
30秒ごとに全mountへ`stat()`するagentは、障害中に
処理が戻らないスレッドを増やし続け、agent自体が停止する危険があります。

そのためprobeを危険度で分けてあります。

* `/proc/1/mounts`などからマウント表を読むprobeは、障害中も安全に動作します。
* 実I/Oを行うprobeはmountごとに同時1本に制限され、
  前回が戻ってこない場合、次回は起動せずスキップします。
  処理が戻らないスレッドは残りますが、mountごとに1本までに制限します。

`sentinel.agent`がremoteのみなのは、
自分への応答確認では、外部から到達できるか判断できないためです。

## Probeの追加手順

1. Capability名を決める（広く有用なら`src/capability/mod.rs::well_known`へ。
   型自体は任意の文字列を受け付けます）。
2. `src/probes/<integration>/`配下に`Probe`を実装します。
3. ペイロードにはraw factのみを返します。結論を出してはいけません。
4. 新しいfactが根拠となるdiagnosis ruleがあれば追加します。

state engine・incident engine・databaseスキーマへの変更は不要なはずです。
変更が必要な場合は、その技術固有の処理をprobeや連携機能内で扱えないか、設計を確認してください。
