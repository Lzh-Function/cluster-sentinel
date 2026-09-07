# 疑似クラスタ (Docker Compose)

Sentinel を **安全かつ再現可能に壊す** ための開発・テスト基盤です
（`docs/IMPLEMENTATION.md` §6）。

production deployment の仕組みではありません。
production は systemd + native host + 単一 `sentinel` バイナリです。

## 構成

```text
                    controller
              Sentinel Controller
                    slurmctld
                        │
        ┌───────────────┼───────────────┐
        │               │               │
    compute01       compute02       compute03
    sentinel        sentinel        sentinel
      slurmd          slurmd          slurmd
        │               │               │
        └───────── peer monitoring ─────┘

           filesrv01           filesrv02
           sentinel            sentinel
        storage service     storage service
```

依存関係（storage domain を 2 つに分けてある理由）:

```text
compute01 ─┐
compute02 ─┴──→ storage01 ──→ filesrv01

compute03 ─────→ storage02 ──→ filesrv02
```

storage01 を壊せば 2 client が同時に劣化し（`SHARED_STORAGE_FAILURE`）、
compute01 だけを storage から切り離せば 1 client のみが劣化します
（`NFS_CLIENT_FAILURE`）。
**この 2 つを取り違えないこと** が storage 診断の要件です。

fileserver には Slurm を入れていません。
Slurm の外にある host も同等に監視できることを示すためです（`SPEC.md` §180）。

## 起動と停止

```bash
cd dev/compose
./scripts/up
```

`up` は container が起動しただけでは戻りません。
controller API の応答、Slurm の node 登録、storage service、
agent の登録がすべて揃うまで待ちます。

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

すべて冪等で、`./scenarios/recover-all` で元に戻ります。

| シナリオ | 壊すもの | 期待される診断 |
| --- | --- | --- |
| `stop-agent <host>` | Sentinel agent のみ | `SENTINEL_AGENT_FAILURE` |
| `stop-slurmd <host>` | `slurmd` のみ | `SLURMD_SERVICE_FAILURE` |
| `stop-ssh <host>` | `sshd` のみ | `SSH_SERVICE_FAILURE` |
| `drain-node <host>` | Slurm 上の状態のみ | `SLURM_ONLY_DEGRADATION` |
| `stop-fileserver <fs>` | storage service のみ | `NFS_SERVICE_FAILURE` |
| `degrade-storage <fs> [slow\|hang\|ok]` | storage の応答性 | storage DEGRADED |
| `pause-host <host>` | container 全体を凍結 | `HOST_UNREACHABLE` |
| `stop-host <host>` | container を停止 | `HOST_UNREACHABLE` |
| `stop-controller` | Sentinel controller | agent は spool を継続 |
| `isolate <a> <b>` | a↔b の経路のみ | `PATH_SPECIFIC_NETWORK_FAILURE` |
| `recover-all` | — | すべて復旧 |

`isolate` が最も重要です。
controller からだけ compute01 が見えない状況で `HOST_UNREACHABLE` と
断定してしまう監視は、健全なマシンのために人をデータセンターへ走らせます。

例:

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

Docker で再現できないものを、再現できたことにしてはいけません
（`docs/IMPLEMENTATION.md` §28）。

| 再現しないもの | 理由 | 委譲先 |
| --- | --- | --- |
| 実機 reboot / boot ID 変化 | container に boot semantics が無い | VM (level 4) |
| kernel hard lock、真の D-state | container は kernel を共有する | VM |
| NFS hard mount による kernel stall | container 内の hang が host を巻き込み得る | VM |
| 実 systemd の挙動 | supervisord による代替 | VM |
| SMART / NVMe / GPU / NIC 故障 | 物理デバイスが必要 | 実機 |
| IPMI / BMC、物理電源断 | out-of-band が必要 | 実機 |

`storage-service` は **実 NFS サーバーではありません**。
`IMPLEMENTATION.md` §11 の指示どおり、
service / network の意味論の再現を優先し、
kernel レベルの NFS 挙動は VM へ委譲しています。
`degrade-storage hang` はユーザー空間での停止であり、D-state ではありません。

## 実装上の判断

**role ごとに image を分けず、image は 1 つ。**
role の違いは「どのプロセスが動くか」だけであり、それは supervisor の設定です。
image を 1 つにすることで build も layer cache も 1 つで済み、
テスト対象と無関係な差異が role 間に紛れ込む余地が無くなります。

**init に systemd ではなく supervisord。**
container に systemd はありません。
また障害注入では「`slurmd` だけを止めて他は動かし続ける」必要があり、
プロセスを background で起動するだけのシェルスクリプトではこれができません。

**munge key は volume 経由で共有。**
key が食い違うと Slurm の認証は極めて分かりにくい形で失敗するため、
`munge-init` で 1 回だけ生成し、全 container が同じものを見ます。
