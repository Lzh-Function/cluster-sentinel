# 設定リファレンス

Sentinel は TOML ファイルを 1 つ読み込みます。既定値は
`/etc/sentinel/config.toml`（`--config`、または環境変数 `SENTINEL_CONFIG` で変更可）。

何かを再起動する前に検証してください。

```bash
sentinel config check
```

最初の 1 件で止まらず、検出できた問題をすべて報告します。
設定が使用不能な場合は非 0 で終了します。
`sentinel config show` は実効設定を出力します。

## 優先順位

```text
CLI  >  環境変数  >  設定ファイル  >  runtime discovery  >  built-in default
```

解決された各値は、どの層から供給されたかを記憶しています。

## `config_version`

必須。本ビルドが理解するのは version `1` です。

**より新しい** version を宣言したファイルは、部分的に読み込むのではなく拒否します。
理解できない設定を中途半端に適用することは、
「運用者が記述した対象とは別のものを監視する」ことを意味するためです。

```toml
config_version = 1
```

## 最小の agent 設定

```toml
config_version = 1
environment = "mizuno-lab"

[agent]
controller_address = "controller.example:7443"
```

controller address に既定値はありません。
バイナリへ host 名を埋め込むことはしません。

## 最小の controller 設定

```toml
config_version = 1
environment = "mizuno-lab"

[controller]
listen = "0.0.0.0:7443"

[database]
path = "/var/lib/sentinel/sentinel.db"

[peer_monitoring]
degree = 3

[discovery.slurm]
enabled = true
```

## セクション

### `[controller]`

| キー | 型 | 既定値 | 意味 |
| --- | --- | --- | --- |
| `listen` | `host:port` | `0.0.0.0:7443` | controller の待受アドレス |
| `inventory_interval` | duration | `5m` | inventory discovery の実行間隔 |

### `[agent]`

| キー | 型 | 既定値 | 意味 |
| --- | --- | --- | --- |
| `controller_address` | `host:port` | *(なし)* | 報告先 controller |
| `spool_path` | path | `<state dir>/spool.db` | ローカル observation spool |
| `roles` | 文字列配列 | `[]` | grouping と default 提示のみに使うラベル |

`roles` は probe を有効化しません。後述の「Capability」を参照してください。

### `[database]`

| キー | 型 | 既定値 |
| --- | --- | --- |
| `path` | path | `/var/lib/sentinel/sentinel.db` |

### `[peer_monitoring]`

| キー | 型 | 既定値 | 意味 |
| --- | --- | --- | --- |
| `degree` | 整数 | `3` | 1 entity あたりに割り当てる observer 数 |

`degree = 0` は peer monitoring を無効化し、警告されます。
第 2 の視点が無い場合、controller 自身の network path 上の障害と
host の死を区別できなくなるためです。

### `[discovery.slurm]`

| キー | 型 | 既定値 | 意味 |
| --- | --- | --- | --- |
| `enabled` | bool | `false` | Slurm discovery を実行するか |
| `scontrol_path` | path | *(`PATH` から解決)* | `scontrol` の場所 |

Slurm は複数ある inventory provider のうちの 1 つです。
Slurm 外の host も一級市民として扱われます。

### `[capabilities]`

Capability 名をキーとする運用者による上書き。

```toml
[capabilities]
"storage.nfs.server" = "force"    # discovery の結果によらず ON
"storage.smart"      = "disable"  # discovery の結果によらず OFF
"storage.zfs"        = "enable"   # discovery が何も言わなかった場合に ON
```

解決順序（強い順）:

```text
disable  >  force  >  runtime discovery  >  enable / role hint
```

### `[[entities]]`

まだ agent が入っていない、あるいはどの integration も発見しない entity を宣言します。

```toml
[[entities]]
type = "host"
name = "fileserver-a"
labels = { rack = "r01", location = "entrance" }
capabilities = ["storage.nfs.server", "observer.peer"]
addresses = ["10.0.0.10"]

[[entities]]
type = "storage"
name = "shared-a"
```

`type` は `host` / `service` / `storage` / `scheduler` / `external_dependency` のいずれか。

`name` は canonical name であり、`(environment, type)` 内で一意です。
address は到達性のためのデータであり identity ではありません — 1 entity が複数持てます。

### `[[dependencies]]`

`from` が `to` に依存します。両者とも `type/name` 形式で記述します。

```toml
[[dependencies]]
from = "host/compute-a"
to = "storage/shared-a"
type = "uses_storage"
criticality = "critical"

[[dependencies]]
from = "storage/shared-a"
to = "host/fileserver-a"
type = "provides"
```

`type` は `depends_on` / `hosted_on` / `provides` / `uses_storage` /
`uses_scheduler` / `network_reaches` / `observes`、
または integration 固有の任意文字列。

`criticality` は `critical` / `important` / `optional`。
`optional` の edge は障害を伝播しません。

ここで宣言していない entity を参照することは error ではなく warning です。
Slurm discovery や agent registration から正当に到着し得るためです。

cycle は許可されます。実際の依存グラフには cycle が存在します。

## 既定パス

| パス | 内容 |
| --- | --- |
| `/etc/sentinel/config.toml` | 設定 |
| `/var/lib/sentinel/` | database と spool |
| `/run/sentinel/` | runtime state |

ログは stderr へ出力されます。systemd 配下では journal に入ります。
