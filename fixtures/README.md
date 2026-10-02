# Fixtures

実コマンド出力を保存し、パーサーのテストの入力とするディレクトリです。

coreはテストデータを参照しません（`docs/IMPLEMENTATION.md` §79）。
参照するのはテストのみです。

`fixtures/slurm/`の内容は`scontrol`の実出力形式に忠実ですが、
ホスト名は実在しないもの（`compute-a`、`ctl-a`等）へ置き換えてあります。
実環境のホスト名や構成が、テストデータを通じて実装に混入するのを防ぐためです。

| ファイル | 由来 | 含まれる状況 |
| --- | --- | --- |
| `slurm/show_nodes.txt` | `scontrol show nodes -o` | IDLE / MIXED / IDLE+DRAIN / DOWN*、GPUあり・なし、空白を含む`OS=`、free-formな`Reason=` |
| `slurm/show_partitions.txt` | `scontrol show partitions -o` | 圧縮hostlist、既定値partition、複数partition |
| `slurm/ping_up.txt` | `scontrol ping` | controller 1台構成 |
| `slurm/ping_ha.txt` | `scontrol ping` | primary/backup構成、backupがDOWN |
| `slurm/ping_down.txt` | `scontrol ping` | controllerへ到達できない場合 |
