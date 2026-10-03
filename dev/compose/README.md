# 疑似クラスタ (Docker Compose)

サービスの停止や通信遮断を再現し、Sentinelの検知・診断を検証する開発・テスト環境です
（`docs/IMPLEMENTATION.md` §6）。

本番環境への配置の仕組みではありません。
本番環境はsystemd + nativeホスト + 単一`sentinel`バイナリです。

## 構成

```text
                    controller
              Sentinel Controller
              Sentinel Agent          ← controller host も監視対象
                    slurmctld
              storage service (export ゼロ)
                        │
        ┌───────────────┼───────────────┐
        │               │               │
    compute01       compute02       compute03
    sentinel        sentinel        sentinel
      slurmd          slurmd          slurmd
                  storage service
        │               │               │
        └───────── peer monitoring ─────┘

           filesrv01           filesrv02
           sentinel            sentinel
        storage service     storage service
```

依存関係（ストレージの共有範囲を分けてある理由）

```text
compute01 ─┐
compute02 ─┴──→ storage01 ──→ filesrv01

compute03 ─────→ storage02 ──→ filesrv02

controller ────→ compute02-scratch ──→ compute02
```

storage01に障害を起こすと2クライアントが同時に劣化し（`SHARED_STORAGE_FAILURE`）、
compute01だけをストレージから切り離せば1クライアントのみが劣化します
（`NFS_CLIENT_FAILURE`）。
この2つを取り違えないことがストレージ診断の要件です。

## 実クラスタの不具合を再現する3つの構成

単純な構成では見つからない欠陥が3つ続けて実機で出たため、
実クラスタで使う次の構成を再現しています。

controllerホストがagentも動かす。
設定ファイルのパスを分けて共存させています（`sentinel install agent --config`
が本番環境で作るのと同じ分け方）。このホストは内側からも監視され、
peerの監視対象でもあります。controllerは自分自身にprobeを実行しないので、
この構成でのみ再現する監視漏れがあります。

compute02がストレージも提供する。
これにより依存グラフに閉路ができます。

```text
storage/compute02-scratch → host/compute02 → scheduler → slurmctld
                          → host/controller → storage/compute02-scratch
```

どのホストから推移的に辿っても、全ホストに到達します。
提供先や使用中のストレージを推移的な到達可能性で判定すると、
直接の依存関係がない対象まで同じストレージを使っていると扱ってしまいます。
実機では、この判定によりヘッドノードに誤ったcriticalが出ました。

controllerのexportポートが応答し、exportはゼロ。
NFSサーバのパッケージが入っているだけのホストの形です。
`/etc/exports`があるのでcapabilityは検出され、サービスはポートに応答し、
しかし誰もそこからマウントしていません。誤検知の検証対象です。

ファイルサーバーにはSlurmを入れていません。
Slurmの外にあるホストも同等に監視できることを示すためです（`SPEC.md` §180）。

## 起動と停止

```bash
cd dev/compose
./scripts/up
```

`up`はコンテナが起動しただけでは戻りません。
controller APIの応答、Slurmのノード登録、ストレージサービス、
agentの登録がすべて揃うまで待ちます。

```bash
./scripts/down     # 停止（volume は残す）
./scripts/reset    # volume ごと破棄して作り直す
```

## 状態の確認

```bash
./scripts/sentinel status
./scripts/sentinel entity show compute01
./scripts/sentinel dependency list

./scripts/logs compute01        # 直近 100 行
./scripts/logs compute01 -f     # follow
```

## 障害注入シナリオ

すべて冪等で、`./scenarios/recover-all`で元に戻ります。

| シナリオ | 停止・変更する対象 | 期待される診断 |
| --- | --- | --- |
| `stop-agent <host>` | Sentinel agentのみ | `SENTINEL_AGENT_FAILURE` |
| `stop-slurmd <host>` | `slurmd`のみ | `SLURMD_SERVICE_FAILURE` |
| `stop-ssh <host>` | `sshd`のみ | `SSH_SERVICE_FAILURE` |
| `drain-node <host>` | Slurm上の状態のみ | `SLURM_ONLY_DEGRADATION` |
| `stop-fileserver <fs>` | ストレージサービスのみ | `NFS_SERVICE_FAILURE` |
| `degrade-storage <fs> [slow\|hang\|ok]` | ストレージの応答性 | ストレージDEGRADED |
| `pause-host <host>` | コンテナ全体を凍結 | `HOST_UNREACHABLE` |
| `stop-host <host>` | コンテナを停止 | `HOST_UNREACHABLE` |
| `stop-controller` | Sentinel controller | agentはspoolを継続 |
| `isolate <a> <b>` | a↔bの経路のみ | `PATH_SPECIFIC_NETWORK_FAILURE` |
| `recover-all` | 該当なし | すべて復旧 |

`isolate`は、経路障害とホスト全体の到達不能を区別するための重要なシナリオです。
controllerからだけcompute01に到達できない場合に、
`HOST_UNREACHABLE`と誤診断しないことを確認します。

例

```bash
./scenarios/stop-slurmd compute01
./scripts/sentinel status
./scenarios/recover-all
```

```bash
./scenarios/isolate controller compute01
./scripts/sentinel status
./scenarios/recover-all
```

## この環境で保証しないもの

Dockerで再現できないものを、再現できたことにしてはいけません
（`docs/IMPLEMENTATION.md` §28）。

| 再現しないもの | 理由 | 委譲先 |
| --- | --- | --- |
| systemdによるcgroup scope管理 | コンテナにsystemd / dbusが無い。`IgnoreSystemd=yes`でslurmd自身にscopeを作らせている | VM (level 4) |
| 実機再起動 / boot ID変化 | コンテナはホストと同じカーネルを使い、実機の再起動を再現できない | VM (level 4) |
| カーネルのハードロック、D-state | コンテナはkernelを共有する | VM |
| NFS hard mountによるkernel stall | コンテナ内で応答が戻らない処理がホストにも影響する | VM |
| 実systemdの挙動 | supervisordによる代替 | VM |
| SMART / NVMe / GPU / NIC故障 | 物理デバイスが必要 | 実機 |
| IPMI / BMC、物理電源断 | out-of-bandが必要 | 実機 |

`storage-service`は実NFSサーバーではありません。
`IMPLEMENTATION.md` §11の指示どおり、
サービス停止や通信障害の再現を優先し、
kernelレベルのNFS挙動はVMへ委譲しています。
`degrade-storage hang`はユーザー空間での停止であり、D-stateではありません。

## 本番環境との差異

本番環境とテスト環境の違いは、次のとおりです。

| 項目 | 本番環境 | このテスト環境 |
| --- | --- | --- |
| cgroup版 | v2 | v2（一致） |
| cgroup階層 | 実cgroup2 | 実cgroup2（一致） |
| scopeの作成者 | systemd（dbus経由） | slurmd自身（`IgnoreSystemd=yes`）|
| init | systemd | supervisord |
| コンテナ権限 | 該当なし（nativeホスト） | `privileged`（slurmdを動かすコンテナのみ）|
| ストレージ | 実NFS | userspaceの代替サービス |

`privileged`が必要な理由は1点だけです。
Dockerは非特権コンテナの`/sys/fs/cgroup`を読み取り専用でmountするため、
Slurmのcgroup/v2 pluginがscopeディレクトリを作成できません。
本番環境ではsystemdが行う作業であり、
これを許可するcapabilityは`privileged`以外に存在しません。

適用範囲はSlurmデーモンを動かすコンテナ（controller / compute）に限定し、
ファイルサーバーには付けていません。
本番環境のsystemd unitでは、権限とアクセスを制限します（`sentinel install`）。

以前はcgroup v1を使用していました。本番環境はv2であり、
かつtmpfs上の偽の階層だったため、v2 + 実階層へ変更しました。

## 実装上の判断

役割ごとにimageを分けず、imageは1つ。
役割の違いは「どのプロセスが動くか」だけであり、それはsupervisorの設定です。
imageを1つにすることでbuildもlayer cacheも1つで済み、
テスト対象と無関係な差異が役割間に紛れ込む余地が無くなります。

initにsystemdではなくsupervisord。
コンテナにsystemdはありません。
また障害注入では「`slurmd`だけを止めて他は動かし続ける」必要があり、
プロセスをbackgroundで起動するだけのシェルスクリプトではこれができません。

munge keyはvolume経由で共有。
keyが食い違うとSlurmの認証は極めて分かりにくい形で失敗するため、
`munge-init`で1回だけ生成し、全コンテナが同じものを見ます。
