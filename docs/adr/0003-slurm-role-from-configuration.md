# ADR 0003 Slurmの役割を設定から判定する

* 状態 accepted
* 日付 2026-09-07
* 開発段階 M3

## 背景

当初は、実行ファイルの有無でSlurmのcapabilityを判定していました。

```text
which slurmd    → slurm.compute
which slurmctld → slurm.controller
```

疑似クラスタでは、全計算ノードで`slurm.controller`が検出されました。
Slurmの各デーモンを一つのパッケージで配布する環境では、
計算ノードにも`slurmctld`の実行ファイルが存在するためです。
このままでは、計算ノードにも制御系のprobeが有効になります。

## 決定

`slurm.conf`の設定内容と、対応する実行ファイルの両方で判定します。

* `slurm.controller`は、ホストが`SlurmctldHost`または旧`ControlMachine`に指定されている場合に検出します。
* `slurm.compute`は、ホストが`NodeName`行に含まれる場合に検出します。圧縮されたhostlistも展開します。

判定は`src/integrations/slurm/detect.rs`に実装します。
Slurm固有の設定解析を、汎用の自動検出処理に含めないためです。

当初は、`slurm.conf`が読めない場合に実行ファイルだけで判定していました。
現在は誤検出を防ぐため、その場合のローカル検出結果を未検出とします。
Slurm側の監視対象一覧や、`[capabilities]`での明示的な指定でもcapabilityを設定できます。

## 利点と制限

計算ノードをcontrollerと誤判定しなくなります。
また、デーモンが停止しても設定は変わらないため、障害の検知に必要なprobeを継続できます。
`compute[01-03]`などのhostlist、FQDNの有無、大文字小文字の違いにも対応します。

設定を読み取れる必要があります。読み取りはローカルパスに限定し、
ネットワークファイルシステムの応答待ちを避けます。
設定を読めない環境ではローカル検出できないため、他のproviderや明示的な設定を使います。

## 関連する修正

検証中に、一度保存されたcapabilityが削除されない問題も見つかりました。
当初は、providerから一時的に報告がなくてもcapabilityを残す方針でしたが、
検出処理が不在を明示している場合にも残していました。

`SqliteStore::reconcile_capabilities`でproviderごとに検出結果を更新し、
明示的な未検出を反映するようにしました。
一つのproviderから報告がなくても、他のproviderの検出結果は削除しません。
