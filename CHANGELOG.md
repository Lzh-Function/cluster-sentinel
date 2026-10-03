# 変更履歴

本プロジェクトは開発段階単位で構築されています
（[docs/IMPLEMENTATION.md](docs/IMPLEMENTATION.md) §94）。

## 未リリース

* 通知の見出し・異常の説明・確認手順を日本語にしました。確認手順には実行するホストと出力で確認する項目を追加し、Sentinelコマンドを`sudo -u sentinel sentinel ...`に統一しました。
* Slackの赤のCRITICAL通知に、冒頭の`@channel`を追加しました。長い手順はコマンドを途中で切らずに分割し、表示上限を超える場合は全文を確認するコマンドを表示します。
* 復旧途中は黄、復旧完了は緑で表示します。汎用webhookの`resolved`も、復旧完了時だけ`true`になります。
* README、導入・運用ガイド、設計書、ADR、設定例の説明を整理し、検査条件と操作手順を具体的に記載しました。見出し変更に合わせて文書内リンクも更新しました。

ノード停止後、観測の期限切れだけで`RESOLVED`が送られる問題を修正しました。原因と依存先の復旧を確認できるまでincidentを維持します。

* probeと監視元ごとに状態を判定し、再送・過去の観測・大きく未来の時刻を持つ観測による誤判定を防止しました。判定回数と観測の保存を一つのトランザクションにまとめ、再起動後も引き継ぎます。
* 通知候補をincidentと同時に保存し、失敗した復旧通知を再送します。通知にはイベントごとの識別子を付け、過去の変更の再通知と、障害再発後の古い復旧通知を防止しました。
* 通知処理を観測・診断から分離し、送信間隔の制御をポーリングをまたいで適用しました。送信待ちの間に始まったmaintenanceも確認します。
* ストレージ構成の対応表、状態の分類、再起動時のincident読み込み、SQLiteとspoolの接続時のロック処理を修正しました。

## v0.3.1

実機導入で見つかった不具合の修正と、新規のホストからの導入手順。

* 到達性probeがcapabilityを要求しなくなった。Slurm自動検出で
  見つかったホストには1つもprobeが走っておらず、
  誰も接触していないホストがHEALTHYと表示されていた
* 実クラスタの識別子をドキュメントから削除。
  ホスト名・VLAN・IP・実MAC由来のlink-localアドレス
* 報告アドレスの選択（loopback interface上のアドレスを除外、
  複数NICの曖昧さを報告、`[agent] interface` / `address`）
* systemd unitの`StateDirectory=`、`install --binary`、
  ダウンロード先から実行した場合の警告
* `sentinel install`が`scontrol`を検出してSlurm自動検出を設定
* Ansibleロール（`deploy/ansible/`）
* release workflow（x86_64 / aarch64の静的リンクバイナリ）

## v1.0.4

計画停止の期間を指定し、作業中の通知を止められるようになった。

* `sentinel maintenance`を追加。ディスク換装や電源工事のように
  計画停止を伴う作業の前に宣言しておけば、その間の通知が止まります。

  ```bash
  sentinel maintenance start filesrv01 --reason "HDD 換装" --for 6h
  sentinel maintenance list
  sentinel maintenance end <id>
  ```

  抑止されるのは通知だけです。probeは動き続け、stateもdiagnosisも
  更新され続けるので、作業中に別の障害が始まった場合も、
  いつ始まったのかを後から追えます。

  incidentが抑止されるのは、影響を受けているentityがすべて
  maintenance対象である場合のみです。1台の作業が、
  4台に影響する障害の通知まで止めることはありません。

* maintenanceの型や判定処理は、以前から定義されていました。
  `MaintenanceWindow`型、その判定規則、それ専用のテスト、
  最初のマイグレーションにある`maintenance_windows`テーブル、
  そしてnotification経路が毎回参照する`MaintenanceWindows`引数。
  ただし、その引数を本番環境が毎回空で作っていたため、
  抑止の分岐は実機で一度も通らず、windowを書き込む手段も存在しませんでした。

  定義した処理が実行経路につながっておらず、`nfs.server.exports`の監視漏れと同じ問題でした。
  storeへの書き込み、診断ループでの読み出し、CLIでの操作を実装しました。
  診断ループがwindowを読み込むことをテストで検証し、空のままなら失敗するようにしました。

* `sentinel status`が抑止中であることを表示。
  これが無いと、「openなCRITICALがあるのにSlackに何も来ない」状況が、
  webhookへの送信に失敗している場合と、誰かがmaintenanceを宣言して忘れた場合の
  両方で同じに見えます。

  ```
  ⚠ notifications suppressed by maintenance: filesrv01
    probing and diagnosis continue; end it with: sentinel maintenance end <id>
  ```

* `sentinel audit`が、期限なしで24時間以上放置されたwindowを報告。
  解除を忘れると、監視を継続していても通知が届きません。
  cronから検出できるよう、exit code 2を返します。
  `--for`で期限を付けたwindowは期限になると終了するので、
  どれだけ古くても報告されません。

* 疑似クラスタの受け入れ項目に、maintenance windowを設定した状態で障害を起こし、
  incidentが変わらずopenになり`incident list`に出ることを
  確認する検査を追加（受け入れ項目31 → 33件）

* 受け入れスクリプトの、常にPASSを返す検査を修正。
  `sentinel status`と`sentinel incident list`は報告すべきものがあるとき
  exit 2を返し、スクリプトは`pipefail`で動いているため、

  ```bash
  if ./scripts/sentinel status | grep -q "x"; then
  ```

  はxがあっても失敗します。逆向きの
  `if ... | grep -q BAD; then fail; else pass; fi`の形は
  常にPASSを返していました。既存の2件がその形だったため、
  出力を先に取得してから照合する形に統一しました。

## v1.0.3

疑似クラスタに実環境と同じ構成を追加し、そこで検出した2件を修正。

* 疑似クラスタに、実環境で使う3つの構成を追加した。
  単純なテスト構成では見つからない不具合が、3回続けて実機で発覚したため。

  | 構成 | 検証する内容 |
  | --- | --- |
  | controllerホストがagentも動かす | controllerは自分自身にprobeを実行しないので、この構成でのみ再現する監視漏れがある |
  | 計算ノードの1台がストレージも提供する | 依存グラフに閉路ができ、推移的な探索で誤判定を再現できる |
  | ヘッドノードがexportゼロでexportポートに応答 | 誤検知の検証対象 |

  ルールをv1.0.1の実装に戻すと、擬似クラスタが実機とまったく同じ文言で
  同じ誤検知を出すことを確認しました。これまでは出ませんでした。

* ストレージのルール2つが、推移的な探索による同じ誤判定があった（バグ修正）。
  閉路を入れた途端に現れました。

  ```
  compute03's access to storage02, compute03-scratch, compute02-scratch,
  storage01 is impaired
  ```

  このノードが宣言している`uses_storage`は1本だけです。
  残りはscheduler経由で推移的に辿って拾っていました。

  * `NFS_CLIENT_FAILURE`（どのストレージを使っているか、誰が隣人か）と
    `SHARED_STORAGE_FAILURE`（どのストレージを共有しているか）の両方を、
    直接の`uses_storage` edgeから判定するよう変更。
  * 閉路のあるグラフで両ルールを検証するテストを追加。
  * 到達可能性でグループ化する`group_by_shared_upstream`は、
    これで本番コードから使われなくなりました。

* readinessがコンテナの実際の構成を見ていなかった。
  ストレージサービスを足したコンテナのhealthcheckがslurmdとAPIしか
  見ておらず、起動直後の最初の診断が「数秒で消えるストレージ障害」を
  報告していました。healthcheckが実際に動かすものを見るようにしました。

* 受け入れ項目を23 → 26に。追加したのは「誰もマウントしていないヘッドノードが
  ファイルサーバー障害として報告されないこと」「そのexportポートが実際に応答して
  いること（検査が空振りしていないことの確認）」「`sentinel audit`がクリーン
  であること」の3つです。

## v1.0.2

* v1.0.1の修正が問題を解消していなかった（バグ修正）。
  ヘッドノードの誤ったCRITICALが消えませんでした。

  v1.0.1は「このホストが提供するものを誰かが使っているか」を
  グラフに問う形にしましたが、推移的な到達性で実装していました。
  実クラスタのグラフではそれがほぼ全体に届きます。

  ```
  host/parent ← service/slurmctld@parent ← scheduler/slurm
              ← 全 compute node ← その一部が提供する storage
              ← もう 1 ホップで再び全ホスト
  ```

  この問い方では、どのホストも「全員に提供している」ことになります。

  * 判定を直接のedgeに変更しました。「このホストが誰かの使う
    ストレージを提供している」を意味するedgeは2本だけで、
    どちらも1ホップです（`provides`と`uses_storage`）。
    導出が実際に書いているedgeそのものです。
  * v1.0.1のテストでは、実クラスタより単純な依存関係を使っていたため、誤判定を検出できませんでした。schedulerを挟んだ構成を再現するテストを追加し、
    推移的な探索へ戻すとテストが失敗することを確認しました。

## v1.0.1

* v1.0.0がヘッドノードに誤ったCRITICALを出していた（バグ修正）。

  ```
  CRITICAL  parent is up but its export port answers while it exports
            nothing, so clients are refused rather than timed out
  ```

  対象のホストはNFSのクライアントであり、exportはしていません。
  ただしサーバのパッケージが入っていて（それがcapabilityの判定条件）、
  exportサービスが動いてポートに応答していました。
  クライアントとして使うマシンにはよくある状態です。

  v0.3.21で「exportゼロが障害になるのはポートが応答している場合だけ」
  というガードを入れましたが、v0.3.23でpeerがそのポートを見るように
  なった瞬間、ガードが満たされてしまいました。

  * v0.3.22まで、誰もそのポートを見ていない → ガードが働く
  * v0.3.23から、peerが見る → 応答する → ガード通過 → CRITICAL

  監視漏れを修正したことで、別の誤検知が見つかった形です。

  * 「ポートが応答するか」では足りませんでした。判定すべきは
    「このホストが提供するものを、誰かが実際に使っているか」です。
  * その情報は既にあります。ストレージentityは誰かが実際にマウントして
    いるときにだけ作られるので、依存グラフがそのまま答えになります。
  * ルールを、誰かがそのストレージを使っている場合にのみ発火するよう
    変更しました。ポート障害側も同じ扱いです
    。利用するクライアントがなければ、停止してもクライアントへ影響しないためです。
  * 検知能力は落ちていません。exportが消えた本物のファイルサーバーでは
    クライアントが依存グラフに残ります（拒否されるマウントは
    クライアントのマウント表から消えないため）。
    acceptanceの`NFS_SERVICE_FAILURE`も通ったままです。

## v1.0.0

実クラスタ（16ホスト）での2日間の運用と、そこで見つかった11件の修正を経て
v1.0とします。バージョン番号の変更以外に機能の追加はありません。

* `sentinel config show`がwebhook URLを平文で出力していた（修正）。
  webhook URLはcredentialであり、それを持つ者はそのチャンネルへ
  投稿できます。設定ファイルを全ユーザーから読み取り可能にしていない理由がそれ
  なのに、コマンドが出力していました。この出力は、助けを求めるときに
  そのまま貼り付けられる類のものです。
  * `(redacted — see the configuration file)`に置き換えます。
    「設定されていて隠されている」と「そもそも設定されていない」は
    別の問題なので、空欄にはしません。
  * Ansible役割が読む項目（environment / probes / TLS / ポート）は
    そのまま残ることをテストで固定しました。

* ドキュメントの精査。
  * 設定キーの記載漏れ3件（`[controller] observe`、
    `[[entities]] display_name` / `cluster`）を追加。
  * `VM_VALIDATION.md`の「Level 4未実施」を実態に合わせました。
    実機で確認できたこと（arch混在、非標準SSHポート、実NFS、
    通知の全経路、複数NIC、controllerとagentの同居、probe audit）と、
    まだ確認していないこと（TLS、長期運用後の保持期間、
    本物のホスト障害）を分けて記載。
  * 全Markdownに対し、リンク・アンカー・bashブロック内のコマンド実在性・
    設定キーの網羅を機械検査。破損0件。

### 互換性

1.xの間、`config_version`と`protocol_version`は`1`のまま、
`--json`のフィールドは追加のみ、終了コードは不変とします。
人が読むテキスト出力は変わることがあります。

## v0.3.23

`sentinel audit`を実クラスタ（16ホスト）で初めて走らせた結果の3件。
2件はaudit自身の問題、1件はauditが正しく見つけた監視漏れ。

* `probe_last_seen`が6.9秒かかっていた（性能バグ）。
  `status`は毎回このクエリを走らせるため、
  いちばんよく使うコマンドに7秒を足していた。
  `(target, probe)`でグループ化しているのに、既存のindexは
  `(target, finished_at)`と`(probe, finished_at)`で、
  どちらも使えず2週間分の観測を全走査していた。
  * グループ化に一致するindexを追加。
  * JOINを`IN`に変更（JOINだとSQLiteがGROUP BY用の
    一時B-treeを作るが、`IN`ならindexを歩くだけで済む）。
  * 120万件で実測: 3.06s → 0.64s（index）→ 0.14s（+ IN）。

* `systemd.unit`を全ホストに対して「報告が止まっている」と報告していた
  （auditの誤検知）。このprobeはホストもtargetingに含むが、
  agentはサービスentityごとにスケジュールし、
  観測はサービスに紐づく。ホストに聞けば当然「一度も無い」になる。
  すべて誤検知だったため、監視漏れを確認する報告として使えなかった。
  * catalogに「何を測れるか」と「何を測るよう実行処理に登録されているか」を
    分けるフィールドを追加し、auditは後者を見る。
    この違いを確認できず、`nfs.server.exports`の監視漏れを見逃していた。

* controller自身のホストのNFSポートを誰も見ていなかった
  （auditが見つけた監視漏れ）。
  `nfs.server.port`はcontrollerしか実行せず、controllerは
  自分自身のホストには一切probeを実行しない
  （到達性については正当な除外だが、それが全probeに及んでいた）。
  * peer agentも`nfs.server.port`を実行するようにした。
    対象のcapabilityでgateされるので、実際にexportしている
    ホストにしか飛ばない。
  * peerが互いの2049番ポートを検査することで、監視漏れを修正し、独立した観測元からも結果を取得できるようにした。

## v0.3.22

* `sentinel audit`（新規）、動いているはずの検査が動いているか。

  `explain probes`は「何が動くはずか」、`entity observations`は
  「何が動いたか」を言うが、この2つを突き合わせるものが無かった。
  probeが実行されなければ観測が記録されず、
  観測が無ければ失敗もせず、失敗しなければ診断もされない。
  監視漏れを報告する仕組みがなく、異常なしに見えていた。
  `nfs.server.exports`が数か月気づかれなかったのはこれが理由。

  * 各entityについて、有効で・適用され・実行する主体があるのに
    自分の間隔どおりに報告していないprobeを挙げる。
  * probeごとにまとめて表示する。1台だけ観測結果がないのはそのホストの
    問題で`status`が既に言うが、全ホストで観測結果がないのは実行処理に登録されていない
    probeであり、他のどこにも現れない。
  * 「一度も観測が無い」と「以前は動いていたが止まった」を区別する。
  * 誤検知が多いと報告が確認されなくなるので、次は報告しない
    意図的に無効化されたprobe、agentが居ないホストのローカルprobe、
    observerが付いていないホストのリモートprobe（それは
    `explain paths`で確認する内容で、ここで挙げると直すべき場所を取り違える）、
    1回取りこぼしただけのもの（probe間隔の10倍、最低5分待つ）。
  * 観測結果が届いていなければexit code 2。cronやCIに置ける。
  * `status`の末尾にも1行出る。この検査自体を実行し忘れれば
    監視漏れを見逃すため、`status`でも確認できるようにした。

  検証: agentのscheduleからexport probeを再び外して当時の状況を
  再現したところ、`status`が自発的に警告し、`audit`が
  `nfs.server.exports`を両ファイルサーバーで名指しした。戻すと警告が消える。

## v0.3.21

* v0.3.20が誤検知を出した（バグ修正）。
  実機のヘッドノードに
  `CRITICAL parent is up but the port answers but nothing is exported`
  が出た。parentはNFSをexportしていない（5台からマウントする側）。

  `storage.nfs.server` capabilityは
  「`/etc/exports`が存在する、または`exportfs`が入っている」で判定される。
  NFSクライアントとしてパッケージが入っていれば真になる。
  そこへv0.3.20でexport probeが走るようになり、
  probeが「exportゼロ」をfailureとして報告したため、
  「NFSを提供できる」ホストが「NFSに障害がある」ホストとして報告された。

  * probeは事実を述べ、ルールが判断するという原則に戻した
    （マウントの`ro`判定で一度通した整理と同じ）。
    exportゼロは`export_count: 0`という事実であってfailureではない。
    ホストのストレージ状態も異常として扱わない。
  * exportゼロが障害になるのは2049で何かがlistenしている場合だけ。
    その場合、クライアントはポートへ接続できてもマウントを拒否される。
    2049で何もlistenしていなければ、
    そのホストは単にNFSサーバではない。
  * exportが消えた本物のファイルサーバー（nfsdは稼働している）は
    従来どおり検知される。
  * メッセージが`... is up but the port answers but nothing is exported`と
    butが2回重なり、読みにくかった。しかも「ポートが応答している」を
    確認せずに報告していた。読める一文に直し、
    報告する内容も実際の観測に合わせた。

## v0.3.20

`nfs.server.exports`の観測が1件も無い、という調査から3件。
いずれも実機（ZFSの`sharenfs`でexportしているファイルサーバー）で発覚。

* `nfs.server.exports`は、誰の環境でも一度も走っていなかった。
  probeは定義され、capabilityでgateされ、catalogに載り、
  診断ルールからも参照されていた。実行経路だけが無かった。
  `ExecutionMode::Local`なので走れるのはagent上だけだが、
  agentの`schedule_storage_probes`はクライアント側の2本しか登録しておらず、
  走る場所が存在しなかった。
  * その結果、`NFS_SERVICE_FAILURE`の
    「ポートは応答するが何もexportされていない」という分岐は
    本番で到達不能だった。ストレージの健全性も
    「2049に何か応答する」だけで決まっていた。
  * クライアント側のearly return（マウントが無ければreturn）より前に登録する。
    マウントを1つも持たない純粋なファイルサーバーは、
    この条件では処理対象から外れるため。

* `/etc/exports`しか読んでいなかった。
  ZFSの`sharenfs`は`/etc/exports.d/zfs.exports`に書き、
  隣の`/etc/exports`はパッケージ同梱のコメントだけのファイルになる。
  プール全体をexportしている健全なファイルサーバーが
  「何もexportしていない」と報告される。上の修正だけを入れていたら、
  全ファイルサーバーにcriticalが出ていた。
  * `exportfs`が読むもの（exports(5)）と同じく、
    `/etc/exports`と`/etc/exports.d/*.exports`の両方を読む。
    ZFS固有の対処ではない。
  * 読めないソースは「exportが無い」ではない。
    権限で読めないファイルがあるときは結論を出さない（Unsupported）。
    実機の`/etc/exports.d/`にはrootのみ読み取り可の
    `zfs.exports.lock`が同居していた。
    拡張子`.exports`のみを読むので実害は無かったが、判定は明示した。
  * catalogの記述も訂正。このprobeは`exportfs -v`を実行しない。
    ファイルを読むだけ。

* entityの参照が曖昧だった。
  導出されたストレージの共有範囲は提供元ホストと同じ名前になるため、
  `david02`が2つのentityを指すようになっていた。
  各コマンドが自分の並び順で候補を示さずに先頭を選んでおり、
  `entity show david02`はストレージを、
  `entity observations david02`はホストを返していた。
  * `host/david02`のような`type/name`を受け付ける
    （依存関係や設定ファイルと同じ書式）。
  * 曖昧なときは自動で選ばず、候補を挙げて拒否する。

## v0.3.19

ストレージの健全性がUNKNOWNのままだという報告を追った結果、3件。

* agentの報告した観測がストレージ導出を経由していなかった（バグ修正）。
  観測がcontrollerに届く経路は3つあり、そのうち
  agentのバッチだけがstate engineを直接呼び出していた。
  `nfs.server.exports`はlocal専用のprobeなので、
  agentのバッチが唯一の経路であり、
  提供元ホストの観測結果が、ストレージentityの状態判定に使われていなかった。
  * 3経路を1箇所に集約した。

* `entity observations --probe X`が「直近N件」の中だけを探していた。
  probeの絞り込みが取得の後に行われていたため、
  上限40件は全probeに対して適用される。
  observerが3台付いたホストでは40件は1分未満で、
  60秒間隔のprobeはまず見えない。
  そして出力は「観測が記録されていません」と言う。
  実際には観測が存在するため、監視が停止しているという誤解を招いていた。
  * クエリ側で絞り込むようにした。
  * 空のときのメッセージを2つに分けた。
    特定のprobeに観測がない場合はcapabilityを確認する。
    entity全体に観測がない場合とは、調査先が異なるため。

* テストが約4回に1回失敗していた（flaky testの修正）。
  同一マイクロ秒に作られた2つの観測は`finished_at`が同値になり、
  クエリはランダムなUUIDで順序を決める。
  観測IDの並びによって合否が変わっていた。明示的な時刻を与えて固定。

## v0.3.18

* NFS導出が「直近N件の観測」を読んでいた（バグ修正）。
  v0.3.17導入直後の実機で発覚。`status`のStorage 5件のうち
  4件がUNKNOWN (stale) になった。

  staleになった4件は、いずれもマウントしているのが1台だけの
  ストレージの共有範囲だった。その1台はagentを入れたばかりのcontroller
  ホストで、peer observerが3台付いたところだった。

  reachabilityはobserverごとに5秒間隔、マウント表の報告は30秒間隔。
  直近N件だけを読むと、高頻度のprobeの観測で1分足らずで件数上限に達し、
  マウント表が取得対象から外れる。マウント表に言及しないsnapshotは
  「そのストレージの共有範囲は無くなった」と言うsnapshotなので、
  マウント構成が変わっていないのに、取得結果にマウント表が含まれるかどうかで依存グラフが変化していた。

  * probeごとの最新1件を読むようにした
    （v0.3.15で診断側に入れたのと同じ修正が、導出側に残っていた）。
  * こちらには鮮度の上限を付けない。「このホストが何を使っているか」
    への最新の記録は最後に判明したマウント表であり、
    報告が止まったagentは「そのホストの健全性」の問題として
    別に扱う。マウント表まで無効にすると、実際には変わっていない依存関係を失う。

## v0.3.17

実機導入で見つかった、必要な情報が表示・記録されない問題3件。

* ストレージentityの健全性が永久にUNKNOWNだった。
  ストレージはファイルサーバーそのものとは別概念として意図的に分けてある
  （「ホストは応答しているがexportだけ停止した」を表現するため）ので、
  直接probeされるものが無い。実クラスタでは
  `status`にUNKNOWNの行が5つ並んでいた。
  * 必要な証拠は提供元ホストのexport probeとして既に存在するので、
    その観測をストレージentityにも向けるようにした。
  * サーバー側の観測だけを使う。ホストはファイルサーバーでありながら
    NFSクライアントでもありうる（scratchをexportしつつ他所のhomeを
    マウントする計算ノード）。両方が同じ`storage` componentに入るため、
    componentをそのまま写すとクライアント側のマウント詰まりが
    「このホストのNFS提供が停止した」として報告される。
    それは調査対象を誤る原因になる。
  * state engineを通すのでdebounceは他と同じ。
    元の観測IDを引き継ぐため、判定から実在する観測まで辿れる。
    DBに行は増えない。
  * probeされていない提供元のストレージはUNKNOWNのまま。
    状態を判断する観測がないため。

* `install`が書いたファイルをサービスユーザーが読めなかった。
  設定はroot所有・0640で書かれるがunitは`User=sentinel`で動く。
  controllerが動いているホストにagentを追加すると、
  systemdからは再起動ループとしか見えない状態になる。
  表示された手順からは、設定ファイルの権限不足を確認できなかった。
  * サービスユーザーが既にあれば、書いたファイルの所有者を設定する。
  * 既にあるものを「作れ」と言わない。従来は手順1が
    「サービスユーザーを作る」で、既にあるホストではその手順ごと
    飛ばされる。そこに新しいファイルに必要なchownが埋まっていた。
    手順2の「credentialを配置する」も同様で、そのまま従えば
    動いているcredentialを上書きして全agentを締め出す。
  * 新規のホストでは従来どおり全手順を出す。

* `entity show`が、ホストの申告したハードウェアを表示していなかった。
  schedulerの設定と比較するために必要だが、
  「Slurmは1 GPUを期待、ホストは0と報告」を
  ホストの実際の申告と照合する手段が無かった。
  * 数えていない場合は`(not stated)`と表示する。
    0とは別物として扱う。

## v0.3.16

* v0.3.15の観測ウィンドウ変更に伴うregressionの修正。
  pseudo-clusterのacceptanceが捕まえた
  （`HOST_UNREACHABLE was not detected (saw: PATH_SPECIFIC_NETWORK_FAILURE)`）。

  「直近32件」というウィンドウは、読み込む量と
  どれだけ古い観測まで採用するかという別々の2つを
  たまたま同時に縛っていた。v0.3.15は前者を
  「probeごと・observerごとの最新1件」に直したが、
  後者を一緒に外してしまっていた。

  結果、ホストを完全に停止したのに「経路障害」と報告された。
  そのホストの観測をやめたobserverの「到達できた」という
  最後の回答が、永久に最新のまま残るため。

  * 観測の鮮度を、そのprobe自身の間隔を基準に判定するようにした。
    古さは相対的で、1分前のreachabilityの回答には価値がないが、
    1分前のSlurmノードviewは現在の値。件数によるウィンドウでは
    これを表現できない。
  * 既定ではprobe間隔の4倍を過ぎた観測は証拠として採用しない
    （reachabilityなら20秒、Slurmノードviewなら20分）。
    1回の取得失敗では状態を変えず、4回分の間隔を過ぎたら観測の停止として扱う。

## v0.3.15

実クラスタで「GPUが1のはずが0」という通知が頻発し、
すぐRESOLVEDになる、という報告からの修正。誤検知で、原因は3つ重なっていた。

* agentが「GPUはあります、0台です」と登録していた。
  registrationがGPU capabilityの有無だけを見て、
  台数には常に`0`を入れて送っていた（数えていなかった）。
  schedulerとの突き合わせルールはそれを信じるので、
  agentが入っているGPUノードは自分のGres行と恒久的に食い違う。
  * probeが実際に数えた台数を報告するようにした。
    まだ一度も数えていなければ「言わない」（`None`）。
    「未申告」は比較対象なしとして扱われるため、誤検知にならない。

* `nvidia-smi`の実行に失敗したとき、probeが`gpu_count: 0`と報告していた。
  数えられなかったことと0台だったことは違う。
  driverから一時的に応答を得られないだけで
  「カードが消えて戻ってきた」ように見えていた。
  * 取得に失敗した場合は台数を書かない。取得できなかったことを、GPUが存在しないと解釈しない。

* 診断が「直近32件の観測」を見ていた（flappingの正体）。
  reachabilityはobserverごとに5秒間隔、Slurmのノードviewは5分間隔。
  取得する件数を固定すると、高頻度のprobeの結果で上限に達し、低頻度のprobeの結果が取得対象から外れる。
  そのためscheduler側の値は「届いた直後の数十秒」しか見えず、
  診断はクラスタの状態が変わらなくても、対象の観測が取得結果に含まれるかどうかで
  現れたり消えたりする。これが「通知が来てすぐRESOLVED」の正体。
  * probeごと・observerごとの最新1件を読むようにした。
    観測の頻度が違っても取りこぼさない。
  * GPUに限らず、Slurm由来のすべての診断が影響を受けていた。

## v0.3.14

* NFSの依存関係を、agentが報告するマウント表から自動導出するようにした。
  実クラスタで、11ノード × 5ファイルサーバーの構成を表現するのに
  `[[dependencies]]`を24個手書きする必要があった。
  マウント表にある構成を設定へ転記すると、変更時に設定が古いまま残る可能性がある。
  その場合、共有ストレージ障害を一つのincidentへまとめられず、
  クライアントごとに報告することになる。

  * `nfs.client.mount`の観測から、ファイルサーバーのホストentity、
    storage entity、`provides`、`uses_storage`をすべて導出する。
    マウント構成が変わっても設定ファイルを触る必要がない。
  * アドレスからentityを作ることはしない。マウントが
    `10.0.0.4:/data`と書かれていて、そのアドレスを持つホストを
    知らない場合は、entityを作成せず「解決できなかった」と報告する。
    アドレスはidentityではない（ADR 0001）。
    `sentinel discover`がどのIPをどのノードが使っているかを示す。
  * 名前で書かれていればホストを作る。`filesrv01:/data`は
    「filesrv01という機械が存在してストレージを提供している」という証拠で、
    従来、設定ファイルで宣言していた情報を取得できる。
  * 導出されたedgeはretractもされる。ノードが別のファイルサーバーに
    移れば古いedgeは消える。増える一方のグラフは、
    使わなくなったファイルサーバーの障害まで関連付けてしまう。
  * 手書きの宣言は併存する（`[discovery.nfs] enabled = false`で完全停止）。

* refusalしか無い状況を「経路障害」と診断していた（バグ修正）。
  実クラスタで誤検知。SSHが22以外に移されていたためprobeは
  誰もlistenしていないポートを呼び出していた。2台のobserverは
  カーネルがRSTを返して`refused`（= 到達、RSTを返すのは生きている証拠）、
  1台はfirewallがDROPしてタイムアウト。
  閉じたポートに対するRSTとDROPの差、つまりfirewallの設定差だけで
  ヘッドノードへの経路障害がCRITICALで報告され続けていた。
  * `PATH_SPECIFIC_NETWORK_FAILURE`は、少なくとも1台のobserverが
    実際に接続を完了していることを条件にした。
    接続に成功した観測元がない場合は、特定の経路だけの障害と判定しない。

* `sentinel status`がincidentを一切表示していなかった。
  2台のホスト間の経路障害はどのentityにも属さないため、
  全ホストがHEALTHY、集計行も「31 healthy」と表示される一方で、
  `sentinel incident list`にはopenなCRITICALがある、という状態になる。
  `status`だけを見ている人には知る術がなかった。
  * openなincidentをentity一覧の前に表示する。
  * openなincidentがあればexit codeも非ゼロになる。

* `ok / refused`という観測表示が誤解を招いていた。
  `refused (so the host answered)`と、何を証明したのかを書くようにした。

* 一度も通知されなかったincidentが、永久に通知されないままになる（バグ修正）。
  実クラスタで発見。`sentinel incident list`にopenなCRITICALが出ているのに、
  webhookには何も届いていない状態が続いていた。

  incidentがannounceされる機会は「それをopenした15秒のパス」1回きりだった。
  そこを逃すと二度と来ない
  * まだwebhookを設定していなかった
  * webhookが500を返した（コード上は「次のパスで再送する」と書かれていたが、
    次のパスにそのincidentはもう乗っていなかった）
  * そのパスの直後にcontrollerが再起動した

  再起動が適用されるのは、controllerが起動時にopenなincidentをengineにseedするため。
  これ自体は正しい（再起動のたびに対応中の障害を再通知しては困る）が、
  「もう伝えた」と「まだ一度も伝えられていない」を区別する情報がどこにも無かった。
  外から見ればどちらも同じ無通知の状態で、正しいのは片方だけ。

  * 通知の配信記録を永続化するようにした。`notifications`テーブルは
    スキーマには最初からあったが、一度も書かれていなかった。
  * controller起動時に配信記録からdeduplicatorを復元する。
  * 通知の候補を「このパスで変化したもの」から
    「まだ伝えていないもの」に変えた。openなincidentは毎パス候補に上がり、
    実際に送るかどうかはdeduplicatorが判断する。
  * openの通知はincidentごとに1回だけ（leaseで期限切れしない）。
    incidentがresolveした時点で記録を消すため、
    同じ障害が後日再発した場合も通知できる。
  * 配信失敗が本当に次のパスで再送されるようになった。
    以前はコメントにだけ書かれていた再送処理を実装した。

* `min_interval`と`format`が生成されるconfigに出ていなかった。
  v0.3.12で追加した設定が`sentinel install` / `sentinel config init`の
  出力にも`docs/templates/controller.toml`にも書かれておらず、
  新規に導入した人はその存在を知る手段がなかった。
  導入時に送信間隔を調整できるよう、生成する設定にも既定値を記載した。
  * 通知セクションを手書きの固定文字列から、
    `Config::default()`から描画する形に変更。以後は既定値と一緒に動く。
  * コメントを外した通知ブロックが実際に解析され、
    書かれている値がコンパイル時の既定値と一致することをテストで固定した。
  * 併せて宛先ごとの`format`（`"generic"` / `"slack"`）も記載。

## v0.3.13

* `sentinel explain`（新規）。この仕組みを作っていない人が読むためのもの。
  `status`は「何を結論したか」を言うが、「それがどうやって分かるのか」は
  どこにも出ていなかった。根拠を確かめられない監視は、
  運用者が診断の正誤を確認できない。
  * `explain capabilities`、各capabilityの意味と、
    ホストに対して何を検査して判定しているか（ファイルの有無、
    `PATH`上のプログラム、設定ファイルの記述）、
    そしてどのprobeを有効にするか。
  * `explain probes`、各probeが実際に実行するコマンドまたはシステムコール、
    必要なcapability、どこが実行するか、間隔とタイムアウト。
    cadenceは設定を反映した値で、停止中なら`[DISABLED]`と出る。
  * `explain paths`、監視経路。ホストごとに、到達アドレス、
    自分のagentが実行するprobe、他所から実行されるprobe、観測者の一覧。
  * `Either`のprobeを「自分自身から」に混ぜない。
    外部の観測元が実行するprobeであり、自分への応答確認では外部からの到達性を検証できないため。
* `entity show`と`doctor`から`explain`への導線を追加。

## v0.3.12

* `[notification] min_interval`（新規、既定`1s`）。
  同一宛先への送信間隔の下限。間引くのではなく間隔を空ける。
  1つの障害が複数の依存先に影響すると1回の診断で複数の通知が発生し、
  webhookは共有されたrate-limitedな資源
  （Slackは概ね毎秒1通で、超えると429）。
  送信数を減らすと、必要な障害通知を失う可能性がある。

## v0.3.11

* `[[notification.webhooks]] format`（新規）。宛先ごとにペイロードの形を選ぶ。
  * `slack`、Block Kit。色つきの帯・見出し・太字・整形済みの詳細。
    復旧は重大度に関わらず緑（色が最初に伝えるべきなのは
    「始まったのか終わったのか」であるため）。
    見出しに重大度を表示し、要約の接頭辞は外して重複を避ける。
  * `generic`（既定）、従来どおり。既存の宛先の挙動は変わらない。
  * Slackは長すぎる`header`を切り詰めず拒否するため、
    全ブロックを上限内に収めている。

## v0.3.10

* Slackへの通知が400で弾かれていた（バグ修正）。
  Slackのincoming webhookは`text`（または`blocks` / `attachments`）を
  要求し、無ければ`missing_text_or_fallback_or_attachments`を返す。
  汎用JSONをそのまま送っていたため、Slack宛の通知は1件も届かなかった。
  * ペイロードに`text`（Slack / Microsoft Teams）と
    `content`（Discord）を追加した。中身はtitle・body・推奨アクションを
    まとめた読める文章。
  * 構造化されたフィールドはそのまま残っているので、
    severityで振り分ける受け手は影響を受けない。
  * 各サービス専用のproviderを書けば色やスレッドも使えるが、
    前段に変換を挟まないと動かないwebhookは、多くの人にとって
    動かないwebhookなので、URLだけで動くことを優先した。

## v0.3.9

* incidentが開いても通知されないことがあった（バグ修正）。
  correlationが自動検出ループと診断ループの両方で走っており、
  通知を送るのは後者だけだった。先に走ったほうが「開いた」という事実を
  消費するため、自動検出ループで開かれたincidentは記録されるだけで
  一度も通知されない。
  * 両ループの実行順によって通知漏れが起きるため、常に再現するわけではなかった。
  * incidentを開く場所を1箇所（診断ループ）に統一した。
    自動検出は監視対象一覧・observation・stateまでを担い、
    診断結果は報告するがincidentは作らない。
  * `DiscoveryReport`から`incidents_opened` / `incidents_resolved`を削除。

## v0.3.8

* `sentinel notify test`（新規）。設定した通知先に届くかを、
  障害を待たずに確かめられる。宛先ごとに成否を表示する。
  incidentもdatabaseも重複排除も触らない。
  障害が起きる前に、URLの誤りや通信上の問題を確認できる。
  * 送る内容は、人が見てもフィルタが見てもテストと分かるようにしてある。
  * `fingerprint`は固定値で、実際のincidentと衝突しない
    （衝突すれば通常の障害通知を抑止してしまう）。
  * テスト通知は、通常通知の重大度の下限である`min_severity`に関係なく送る。
* `docs/OPERATIONS.md`に、通知経路の確認と、
  本番で試せる最小の実障害（agentの停止）を追加。

## v0.3.7

* サービスentityを誰も観測していなかった（バグ修正）。
  `slurmd@<node>`は監視対象一覧に存在するのに、probeが1つも向いておらず、
  実クラスタで31 entity中13が永久にUNKNOWNだった。
  サービスを独立したentityにしているのは
  「デーモンが停止した」と「ホスト全体が到達不能になった」を別の答えにするためであり、
  サービスを誰も測らなければその区別は存在できない。
  * unitをsystemdに訊くのはlocalな問い（controllerは肩代わりできない）。
    agentが、自分のcapabilityが示すunitを監視するようにした
    （`slurm.compute` → `slurmd`、`slurm.controller` → `slurmctld`）。
  * 観測はサービスに帰属させ、ホストには帰属させない。
    ホストに混ぜるとデーモンとマシンの区別が消える。
  * capabilityによる判定であり役割では決めない。capabilityの判定には
    設定とバイナリの両方が必要（`docs/adr/0003`）。
  * systemdの無いホストには何もスケジュールしない
    （UNSUPPORTEDを出し続けるより無いほうがよい）。

## v0.3.6

* agentが自分のsandboxのマウント表を読んでいた（バグ修正）。
  生成されるsystemd unitは`ProtectSystem=strict`を設定するため、
  サービスはファイルシステム全体が読み取り専用に再マウントされた
  専用のmount namespaceで動く。probeが読んでいた`/proc/self/mounts`は
  そのnamespace内のマウント状態であり、書き込み可能なNFS共有が全ノードで
  読み取り専用と報告されていた。自分のsandboxを説明して、
  それをホストの状態と称していたことになる。
  ホストのマウント表（`/proc/1/mounts`）を読むようにした。

## v0.3.5

* 到達性probeがSSHポートを無視して22番に固定されていた（バグ修正）。
  SSHを22以外に移し、22番をfirewallでDROPしている環境では、
  健全なホストが到達不能と報告される。実クラスタで13台が該当した。
  22番がREJECTを返すホストだけが「到達可能」と判定され、
  同じクラスタ内で結果が割れていた。
  probeは`ports.ssh`（agentが`sshd_config`から自動検出して報告する値）を
  優先して接続するようになった。refusalを成功とみなす点は変わらない。
* `sentinel`ユーザーが`systemd-journal` groupに入っていなかった。
  journalが読めるホストでも`journal.events`がUNSUPPORTEDになり、
  カーネルイベントが一切収集されていなかった。
  `install`の案内とAnsibleロールの両方でgroupに追加する。
* 読み取り専用なNFS mountを「劣化」と判定しなくなった（誤検知の修正）。
  `/proc/mounts`の`ro`は「読み取り専用である」ことしか示さず、
  「読み取り専用へ変更された」かどうかは分からない。意図的にroで
  export / mountしている共有は珍しくなく、それを常時DEGRADEDと
  報告するのは恒久的な誤警報で、ストレージcomponent全体が信用されなくなる。
  事実として記録するだけにした。カーネルがroへ変更した場合は
  journal probe（`filesystem_readonly`）が捉える。
* `docs/DEPLOYMENT.md`にfirewallで開けるポートの節を追加。

## v0.3.4

Ansibleロールのみの変更です。バイナリに変更はありません。

* 各ノードへ配る共通設定を、controllerの設定ファイルから取得するようになった。ロールが実行時にcontrollerの
  `config.toml`を読み、揃っていなければならない設定
  （`environment` / `[probes]` / `[tls]`のクライアント側 / 待ち受けポート）を
  各ノードへ配る。読み取りはcontroller自身のバイナリ
  （`config show --json`）で行うため、デーモンが解釈するのと同じ値が配られる。
  監視頻度の変更がcontroller 1箇所の編集で済む。
* credentialが0750になっていた（バグ修正）。所有者とモードを
  `recurse`で一括設定していたため、再帰的な`file`タスクが
  ディレクトリ用のモードをファイルにも適用し、直前に0400で書いた
  tokenをgroup読み取り可能かつ実行可能にしていた。
* 毎回バイナリを再ダウンロードしていた（バグ修正）。
  `sentinel_version`は`v0.3.3`、`sentinel version`の出力は`sentinel 0.3.3`。
  先頭の`v`のせいで比較が一致せず、実行のたびに全ノードが取得していた。
* `-e sentinel_observer=false`がobserverを有効にしていた（バグ修正）。
  `-e`で渡された値は文字列で、`"false"`は真。booleanを全て`| bool`で受ける。
* controllerに対して実行するとcontrollerの設定を上書きしていた。
  `sentinel-controller.service`があるホストでは実行を拒否する。
  `sentinel_manage_config=false`でバイナリとunitのみの配布も可能。
* `[probes]`をAnsibleインベントリから設定できるようになった（同期を使わない場合）。
* 冪等性を検証: まっさらから1回目`changed=9`、2回目以降`changed=0`。

## v0.3.3

実機での切り分けに必要だったものと、Ansibleロールの修正。

* `sentinel entity observations <name>`（新規）。
  stateやdiagnosisではなく、どの観測者が何を見たかを直接表示する。
  時刻 / probe / observer / status / アドレスと失敗理由。
  「SSHは通るのに到達不能」のような一見矛盾した状態は、
  観測者ごとの食い違いであることが多く、observer列を見れば矛盾でなくなる。
* `sentinel entity show`がprobe先アドレスと、その決まり方を表示する。
  agentが報告したアドレスなのか、entity名から毎回解決しているのかは、
  probeの成否を左右するが、これまで出力に無かった。
* `doctor`のcredential判定が環境変数しか見ていなかった。
  デーモンはunitの`SENTINEL_TOKEN_FILE`から読むため、対話シェルでは
  常にNOT CONFIGUREDと表示されていた。設定ファイルの隣のtokenも見る。
* Ansibleロールの`sentinel_version`がv0.3.0のままだった。
  ロール自身が使う`install --binary`を持たないバイナリを配っていた。
  リポジトリの版と一致していなければ、テストが失敗するようにした。
* Ansibleロールがアーキテクチャ別の成果物を明示的な対応表で選ぶ。
* Ansible: SSHユーザーの決まり方、ノードごとに異なるsudoパスワード
  （暗号化ファイル1つで済む方法）を文書化。

## v0.3.2

* `install --force`がcredentialを上書きしなくなった（バグ修正）。
  unitを更新するために`--force`を実行すると、cluster credentialが
  再生成され、全agentが一斉に締め出されていた。
  ファイルの上書きに認証情報の更新を含めると、既存のagentが認証できなくなる。
  意図的な更新は、ファイルを削除してから`install`を実行する。
* アップグレード手順を`docs/OPERATIONS.md`に具体化
  （バイナリ入れ替え、順序、unit更新時、切り戻し）。
* AnsibleロールがSSH / sudoのパスワード認証環境で動くように。

M0-M10（core scope）完了。
CLIからクラスタ状態と障害原因を説明できる状態です。

`IMPLEMENTATION.md` §97のv1受け入れ手順を
`dev/compose/scripts/acceptance`として自動化しており、
Docker疑似クラスタに対して23項目すべてが通ります。

### 横断的な事項

* 到達性probeがcapabilityを要求しなくなった（バグ修正）。
  実機導入で発覚。Slurm自動検出で見つかったホストは
  `slurm.compute`しか持たないため、`network.tcp`を要求していた
  reachability probeが1つも動いていなかった。
  結果として、誰も接触していないホストがHEALTHYと表示されていた
  （SlurmがIDLEと言っているだけ）。
  * TCP接続を開くのに相手側に必要なものは何も無い。必要なのはアドレスだけで、
    それは呼び出し側が既に持っている。capabilityが有効化を判断するべきなのは
    `nvidia-smi`や`journalctl`、NFS exportのように
    対象に何かが存在することを要求するprobeである。
  * これでagentの無いホストも、controllerとpeerから実際に確認される。
* controllerは自分自身のホストを観測しない（peer割り当てと同じ理由）。
  controllerが「自分のホストは応答する」と報告しても何も証明していない
  （応答していなければ報告できない）。UNKNOWNのままにしておくほうが
  「誰も独立に見ていない」という事実を正しく表し、
  対処（そこにagentを置く / peer observerを付ける）を促す。
* Ansibleロール（`deploy/ansible/`）。
  ノードが10台を超えると手作業は現実的でない。
  アーキテクチャ別のバイナリ取得・チェックサム検証・`sentinel install`・
  credential配布・`config check`・起動まで。
  Sentinel側にAnsible固有のものは無い。
* 報告アドレスの選択（バグ修正 + 新規機能）。
  agentが報告するアドレスはpeerが最初に接続先であり、
  間違えると健全なホストが到達不能に見える。
  正常な対象を到達不能と誤診断するため、影響が大きい。
  * バグ: loopback判定が *アドレス* だけを見ており、
    *インターフェース* を見ていなかった。`lo`に付いた非loopbackアドレス
    （WSLの`10.255.255.254/32`など）が残り、文字列順で先頭に来ていた。
    開発機で実際に再現。誰からも到達できないアドレスを報告していた。
  * バグ: アドレスの文字列ソートで先頭を選んでいたため、
    コンテナbridgeが実NICを追い越しうる。
  * 到達不能なもの（loopback、link-local、`lo`上の全アドレス）を除外し、
    物理NICを仮想NICより優先、IPv4をIPv6より優先する順位付けに変更。
  * 自動検出では答えられない問いがあることを明示した。
    `vlan101` / `vlan102` / `vlan103`を持つホストで、
    どれをクラスタ内通信に使うかは、ホストの情報だけでは判別できない。
    物理NIC候補が複数ある場合は曖昧であると報告する。
    確認せずに選ぶと、間違っていても気づけない。
  * `[agent] interface`（NIC名）と`[agent] address`（直接指定）を追加。
    `interface`の指定先にアドレスが無い場合、別のNICにフォールバックせず
    何も報告しない。運用者が選ばなかったネットワークにpeerを
    向けるのが、この設定で防ぎたい障害そのものだから。
  * `sentinel doctor`が、報告されるアドレス・候補一覧・
    曖昧な場合の警告を表示する。
* 監視頻度の設定（新規機能、`[probes]`）。
  コンパイル時の既定値は数百ノード・健全なネットワークを想定したもので、
  どこでも正しいわけではない。変更できなければ、
  誤った頻度で動かすか動かさないかの二択になる。
  * probe idごとに`interval` / `timeout` / `max_outstanding` / `enabled`。
  * 書かれていないprobeは既定のまま。1つ調整しても他に影響しない。
  * 設定の上書きは`ProbeDefinition`自体に適用する。decoratorで包まないのは、
    いくつかのprobeが自分のタイムアウトで実行コマンドを制限しているため。
    runnerだけが知るタイムアウトは「同じ名前の別の値」になる。
  * `max_outstanding`は引き下げのみ。`nfs.client.io`と
    `journal.events`の同時実行1は、blockingシステムコールを積み上げないための
    制約であり（`SPEC.md` §76）、設定ファイルで覆せない。
  * 存在しないprobe idはerror。警告なしに無視されると
    「変更したつもりで変わっていない」状態になる。
  * controllerのremote probeとagentのpeer probeにも同じ設定の上書きが適用される。
    観測者ごとに頻度が違うとquorumが異なる頻度の観測を比較することになる。
  * `src/probes/catalog.rs`をprobe一覧を管理するファイルとして追加。
* 設定ファイルの自動生成（新規機能）。
  バイナリを持ち込んだ最初の5分が転記作業になっていた。
  手順を印刷して人間に実行させると、飛ばされるのは必ずcredentialの行。
  * `sentinel install <role>`が設定ファイル・systemd unit・
    （controllerのみ）cluster credentialをまとめて生成する。
  * 設定ファイルには全設定が既定値のまま、説明つきで書き出される。
    値は`Config::default()`とprobe catalogから生成するため、
    設定ファイルとバイナリの既定値が一致する。
  * 書き換えが必要な行だけ`CHANGE-ME`が入る
    （controllerは1行、agentは2行）。
  * 既存ファイルは上書きしない。二度実行しても安全で、
    credentialが入れ替わって全agentが締め出されることもない。
    `--force`を付けた場合のみ上書きする。
  * credentialは32 byte乱数、mode 0400。
    agentには生成しない（クラスタの誰も知らないcredentialができるため）。
  * tokenの位置は`--config`の隣に決まる。unitの
    `SENTINEL_TOKEN_FILE`も同じ場所を指す。
  * `sentinel config init [--role R] [--output P] [--dry-run] [--force]`。
* 記録の保持期間（新規機能、`[retention]`）。
  databaseは書き込み一方で、削除する経路がコードのどこにも無かった。
  実測で5ホストあたり約10 KB/s、ホスト1台あたり1日約170 MB。
  100ノードなら1日17 GBで、放置するとディスクが埋まり、controllerが停止する。
  * classごとに期間を設定する（observation 14d / transition 90d /
    解決済みincident 180d / 孤立diagnosis 30dが既定）。
    データの種類によって必要な保存期間が異なるため。
  * `"never"`で個別に無期限保持を選べる。
  * openなincidentは、発生からの期間に関わらず削除しない。
    1年開いているincidentは1年直っていない障害である。
  * 保存されているincidentやdiagnosisが参照するobservationは、保持期間を過ぎても残す。
    診断の根拠を後から検証できるようにするため。
  * 各entityは直近`keep_per_entity`件を必ず残す。
    保持期間より長く停止しているホストも、最後の観測結果を確認できるようにする。
  * controller内で1時間ごと、および起動時に実行。
  * `sentinel prune [--dry-run] [--vacuum] [--observations <期間>]`。
    `--dry-run`の件数は実際のDELETEを実行してrollbackしたもので、
    別のCOUNTクエリではない（本番と食い違わないため）。
* TLS（新規機能、`[tls]`）。
  credentialはbearer tokenであり、wireを読める者は全agentに
  なりすませる。従来はTLSを使うために、運用者がreverse proxyを別途用意する必要があった。
  * `cert` + `key`でcontrollerがTLS listenする。
  * `client_ca`を書くとクライアント証明書が必須になる（任意にはならない。
    任意のクライアント認証は攻撃者が提示しないだけで無効化される）。
    tokenが漏れても耐えられる構成はこれだけ。
  * agent側は`ca` / `client_cert` / `client_key` / `server_name` /
    `insecure_skip_verify`。`ca`はsystem rootを置換せず追加する。
  * クライアント側の設定が1つでもあれば`host:port`は`https://`と解釈する。
  * TLSの証明書や秘密鍵が読めない場合、controllerは起動に失敗する。
    暗号化が無効のまま運用されるのを防ぐため。
  * 証明書の自動生成はしない。監視システムがtrust anchorを発行すれば、
    CAの管理と監査を別途行う必要が生じる。
  * agentのhealth endpointは平文のまま。credentialを運ばず、
    liveness以外を明かさない。
* 診断結果のretraction修正。`diagnose_and_classify`が
  classificationを追加しかしておらず、解決済みの障害のラベルが
  databaseに残り続けていた。画面では現在の障害と解決済みの障害を区別できなかった。
* 非標準ポートの設定（新規機能）。
  `metadata.ports`は読まれていたが、どのproviderも書いていなかったため、
  SSHが22以外のクラスタは設定不可能だった。
  * agentが`/etc/ssh/sshd_config`の`Port` / `ListenAddress host:port`を
    読んで自動検出し、controllerへ報告する。
  * `[agent] ssh_port`で明示的に上書きできる。
  * agentがいないホストは`[[entities]] ports = { ssh = 2222 }`で宣言する。
  * agentは自分の`listen`ポートも報告するため、
    変更してもpeerが正しい場所を接続する。
* `docs/DEPLOYMENT.md`、実クラスタ導入マニュアル。
* `docs/templates/`、コピーして使える設定テンプレート。
  テンプレートが`config check`を通ることをテストで強制する。

* v1受け入れ手順の自動化（23項目）。
  各段階で一つの障害を起こし、原因を正しく診断し、他の対象を誤診断しないことを検査する。
* `SENTINEL_AGENT_FAILURE` / `SSH_SERVICE_FAILURE` rule。
  いずれも「ホストが別経路で応答している」積極的証拠を要求する。
* CI設定。fmt / clippy / test / release buildはDocker不要。
  疑似クラスタは別job。
* `docs/VM_VALIDATION.md`。Dockerで検証できない項目の一覧。

### M10、Notification / Operations

* Notificationは変化があったときのみ送信する。
  継続中のincidentは何度pollingしても送信しない。
  pollingごとの再通知は、監視を無視する習慣を作る。
* 重複排除は宛先ごと・triggerごと。
  復旧通知が発生通知の重複として抑止されることはない。
  controllerとfallback notifierが同じincidentを見ても通知は1回。
* 送信失敗は記録しないため、次回再試行される。
* Maintenance windowは通知のみを抑止する。
  observation・state・diagnosisは継続し、異常をhealthyに書き換えない。
  影響entityがすべて対象の場合のみ抑止する。
* Webhook provider（ntfy / Gotify / Slack / Discord等にPOST可能）。
  ペイロードは自己記述的で、受信側がSentinelの内部を知る必要がない。
* `sentinel install controller|agent`、権限とアクセスの制限済みsystemd unitを生成。
  credentialはunitに書かず、ファイルを参照する。
* `docs/OPERATIONS.md`を追加。段階的導入、診断結果の読み方、
  トラブルシューティングを記載。

### M9、Diagnosis / Incident Correlation

* Incident engine。相関の原則は 「同じ症状」ではなく「同じ原因」。
  fingerprintをsuspected root entityから導出するため、
  1つのファイルサーバーに起因する複数の症状が1 incidentにまとまる。
  無関係な障害が同時刻に起きても統合されない。
* Dependency fan-outによるseverity決定（`SPEC.md` §101）。
  3台以上が依存する対象の障害はcritical。
  低confidenceの推測はfan-outによらずcriticalにしない。
* Lifecycle: rootが復旧しても依存先が未復旧なら`RECOVERING`に留まり、
  `RESOLVED`にしない（`IMPLEMENTATION.md` §75）。
* 再発は30分以内なら同一incidentをreopenし、
  flappingで毎回新規alertを出さない。
* Incident・diagnosis・根拠となる観測・timelineの永続化。
  controller再起動時はopenなincidentを再開し、再alertしない。
* timelineは「変化」のみ記録する。pollingごとに1行増えると
  重要な出来事が埋もれるため。
* 根拠となる観測は上限付き。発生時点のものを優先して保持する。
* `sentinel incident list` / `sentinel incident show`を追加。

### M8、Peer Monitoring

* Peer assignment（`SPEC.md` §46-§49）。
  observerは「互いに似ていない」ことを優先して選ぶ
  同じ依存先を持つ範囲、異なる障害ドメイン、依存先を共有しないインフラから選ぶ。
  同じストレージを使うobserver 3台は、共有するストレージの障害で同時に影響を受ける。
* 同じ入力から同じ監視元を割り当てる。controller再起動で全observerが入れ替わり、
  debounce counterが一斉にリセットされることを防ぐ。
* `GET /v1/agents/{id}/assignments`。アドレスはcontrollerが監視対象一覧から解決し、
  agentが推測する余地を残さない。
* Agentは割り当てられたpeerを観測し、observationに自分を署名する。
  controller到達不能時も、既存の割り当てで観測を継続する。
* Reachability rule
  * `HOST_UNREACHABLE`、独立したobserverが全員失敗（2台以上）。
  * `PATH_SPECIFIC_NETWORK_FAILURE`、一部のみ失敗。
  * observerが1台だけならどちらも出さない。
* `POWER_OFF`とは判定しない。confidenceは`High`止まりで`Confirmed`にしない。
  電源断・NIC故障・switch障害はここからは同じに見える。
* `sentinel peers`を追加。observerが付いていないentityを明示する。

### M7、GPU

* `gpu.nvidia` probe（`nvidia-smi`のCSV出力を解析）。
  `[N/A]` / `[Not Supported]`を解析失敗ではなく「値なし」として扱う。
* Agentはprobeが実際に観測したGPU数をregistrationへ反映する。
  capability（GPUを持ち得る）と観測結果（GPUが2枚ある）を混同しない。
* GPU数の不一致はdegradedとして報告し、ノードをbrokenとは呼ばない。
  SlurmにGRES行が無い場合は「不一致」ではなく「未記載」として扱う。
* GPUの状態を変更する`nvidia-smi`オプションを使わないことをテストで強制。
* 実GPUの検証はlevel 4（VM / 実クラスタ）。コンテナではmockのみ。

### M6、Storage / NFS

* NFS probeを危険度で分割
  * `nfs.client.mount`、`/proc/self/mounts`のみ。hang中も安全。
  * `nfs.client.io`、実I/O。mountごとに同時1本（`SPEC.md` §76）。
  * `nfs.server.port` / `nfs.server.exports`、到達性とexport一覧。
* `NFS_OK` / `NFS_SLOW` / `NFS_TIMEOUT` / `NFS_STUCK`の区別。
* Storage診断rule
  * `NFS_SERVICE_FAILURE`、ホストは稼働、exportサービスのみ異常。
  * `SHARED_STORAGE_FAILURE`、同一ストレージの複数クライアントが同時異常。
    groupはdependencyグラフから導出し、宣言しない。
  * `NFS_CLIENT_FAILURE`、1クライアントのみ異常、同一ストレージのpeerは正常。
  最後の2つは相互排他であることをテストで強制。
* AgentはNFS mountごとにprobeを個別スケジュール（同時実行枠も個別）。
* Slurm capability検出をより保守的に変更。
  `slurm.conf`が読めないホストはSlurm役割を報告しない。
  バイナリの存在だけではファイルサーバーまで計算ノードと判定してしまう。
* `[controller] observe`を追加。controller自身がremote probeを行うかの制御。

### M5、Slurm Diagnosis

* Diagnosis engine: typed Rust rule、決定的評価、ruleのpanic隔離。
  DSLもLLMも使わない（`SPEC.md` §94）。
* `DiagnosisContext`: ruleが参照できるのは保存済みの
  監視対象一覧 / state / observationのみ。
  外部通信もコマンド実行も行わないため、診断は後から再現・検証できる。
* Slurm rule
  * `SLURM_ONLY_DEGRADATION`、ホストは健全、Slurm上のみDRAIN。
  * `SLURMD_SERVICE_FAILURE`、ホストは応答するがslurmdが登録しない。
  * `SLURM_CONTROL_PLANE_FAILURE`、制御系自体の異常。
  * `RESOURCE_CONFIGURATION_MISMATCH` / `GPU_CONFIGURATION_MISMATCH`。
* すべてのruleが「ホストが健全である積極的証拠」を要求する。
  証拠が無ければ診断を出さない。
  これが無いと、到達不能なホストが「停止したデーモン」として報告される。
* `sentinel diagnose`を追加。診断・根拠・読み取り専用な調査コマンドを表示。
* 疑似クラスタで検証: drain / slurmd停止 / ホスト停止が
  それぞれ異なる結果（3つ目は「診断を出さない」）になること。

### M4、Basic Host Monitoring

* Probe runner: タイムアウト、panic隔離、targetごとの同時実行数制限。
  probeのpanicでデーモンが停止しないことをテストで検証。
* Probe実装
  * `host.metrics`、`/proc`からload / memory / pressure / uptime / boot ID。
    高負荷はdegradedとして扱い、failedにしない。計算処理中の高負荷だけでは故障と判断できないため。
  * `network.tcp`、到達性。refusalは到達可能の証拠（ADR 0004）。
  * `ssh.service`、banner確認のみ。認証もremote実行も行わない。
  * `systemd.unit`、`systemctl show`による読み取り専用な状態取得。
  * `sentinel.agent`、peerからagentのhealth endpointを確認。
* Agentのhealth endpoint（`/v1/agent/health`）。読み取り専用、GETのみ。
  agent本体をロックせずに応答するため、probeが遅くてもpeerから見える。
* Agent側local probe scheduler（probeごとのintervalとjitter）。
* Controllerがobserverとしてremote probeを実行。observationにはobserverを記録。
* State component（ホスト / ネットワーク / ssh / agent / サービス）へのmapping。
  未mappingのprobeが生まれないことをテストで強制。
* 疑似クラスタで検証: agent停止とsshd停止が、
  それぞれ独立したcomponent異常として観測されること。

### M3、Docker Compose疑似クラスタ

* 6コンテナ（controller 1 / compute 3 / ファイルサーバー2）の疑似クラスタ。
  実munge / slurmctld / slurmdが動作する。
* 障害注入シナリオ11種
  `stop-agent` / `stop-slurmd` / `stop-ssh` / `drain-node` / `stop-fileserver` /
  `degrade-storage` / `pause-host` / `stop-host` / `stop-controller` /
  `isolate` / `recover-all`。すべて冪等で復旧可能。
* `wait-healthy`はコンテナの起動だけでなく、controller API・Slurmのノード登録・
  ストレージサービス・agent登録がすべて揃うまで待つ。
* Docker統合テスト11件（`SENTINEL_DOCKER_TESTS=1`でopt-in）。
  Docker非対応環境でもunit / simulationテストは通る。
* Slurm役割の判定を設定ベースへ変更（ADR 0003）。
  バイナリの存在では計算ノードが制御系と報告してしまう。
* capabilityの取り下げをprovider単位で反映（`reconcile_capabilities`）。
* `docs/SECURITY.md`、`dev/compose/README.md`を追加。

### M2、Agent + Protocol

* Wire protocol v1（versioned JSON over HTTP）。バイナリversionとprotocol versionを分離。
  未知フィールドを許容し、controllerの更新で、全ノードとの通信が停止しない設計。
* 認証: cluster-scoped bearer credential。未認証動作は提供しない。
  credentialはファイルまたは環境変数から読み、バイナリへ埋め込まない。
  比較はconstant-time、`Debug`出力は常にredact。
* Controller HTTP API`/v1/health`（無認証）、`/v1/agents/register`、
  `/v1/agents/heartbeat`、`/v1/observations/batch`。
  コマンドを実行するrouteは存在しない（テストで検査）。
* Agent session管理: boot IDの変化のみを再起動と判定し、
  agentプロセスの再起動と区別する。
* 自動検出に`SystemInspector`インターフェースを使い、
  実機に無い構成（GPUノード、ファイルサーバー等）に対してもテスト可能。
  capabilityの不在も明示的に記録し、役割hintを上書きできるようにする。
* Local spool: WAL、age / rows / bytesによる上限、順序付きreplay、
  idempotent再送、重要な行を優先保持。送信前に書く（ADR 0002）。
* Agentデーモン: 登録・heartbeat・spool flush。controller不在時も動作を継続。
* CLI`sentinel controller`、`sentinel agent`、`sentinel doctor`。
* `docs/SECURITY.md`を追加。

### M1、Passive Controller + Slurm

* 共通externalコマンドrunner: タイムアウト、出力サイズ上限（truncate事実の記録）、
  allowlist、構造化エラー。probeが個別に`Command::new()`を書くことを禁止。
* Slurm integration`scontrol show nodes -o` / `show partitions -o` / `ping`の
  パーサー、hostlist展開、ノードstate（base + flag + suffix）の解釈。
  未知フィールド・未知stateに対して寛容。
* Inventory: 複数providerからのmerge、自動検出source保存、
  消失時のstale化（削除しない）、endpoint未発見のedgeの遅延解決。
* Slurm監視対象一覧provider（Host / Service / Scheduler entityと依存edgeを生成）と
  static config provider。
* State engine: probe → componentの宣言的mapping、debounce、severity cap。
  再起動時は保存済みstateから再開。
* 永続化: entity / capability / ラベル / dependency / observation / state /
  transitionのread-write。observation ingestionはidempotent。
* Passive controller: 自動検出処理1回で監視対象一覧・observation・stateを更新。
  providerが失敗しても他providerの結果を消さない。
* CLI`sentinel status`、`sentinel discover`、`sentinel entity list|show`、
  `sentinel dependency list`。全て`--json`対応。
* `fixtures/slurm/`に実出力形式のテストデータを追加し、パーサーのテストの入力とする。

### M0、Repository / Core Domain

* Core domain modelを追加`Environment` / `ManagedEntity` / `Capability` /
  `DependencyEdge` / `ProbeDefinition` / `Observation` / `EntityState` /
  `StateTransition` / `Diagnosis` / `Incident`。
* Entity identityを識別キーからのUUIDv5導出に決定（ADR 0001）。
* Capability解決の優先順位を実装（force-disable > force-enable >
  runtime discovery > role hint）。
* Dependencyを閉路許容の汎用有向グラフとして実装。全traversalにvisited set。
* Debounce / hysteresis（既定: warning 2、critical 3、recovery 2）。
* 設定の読み込み・優先順位追跡・検証。未知の`config_version`は拒否。
* SQLite coreスキーマの初期マイグレーション。controller複数台を禁止しない設計。
* CLI skeleton`sentinel version`、`sentinel config check`、`sentinel config show`。
* `src/`へdeployment固有名が混入していないことを検査するテスト。
