# lazypencil 実装と公開準備の状況

CI待機中に、生成browserの高速レイアウト試験をスキーマ別の最大2並列へ変更した。各ケースの一時文書・素材・data・scriptプロセスは独立し、開始済みの両threadを終了時にjoinする。54ケース・162段階の検査、実Chromium画素・構成／OS・期限・計測の順序は保持する。この変更も現在のCIにはpushせず、後続batchで実測する。

進行中の `d6c5808` は軽量207件・28サマリーとClippyを成功し、Linuxの変更対象8 targetとruntime並列確認へ進んだ。有用なCIを中断せず、次の差分はローカルだけで準備する。生成 script の無効な `script_scope` がコピー後の実行まで拒否していた経路を修正し、URI変換での再有効化と入力側の独立した有効性を区別する。両形式・4経路・2条件の実 worker／コピー／再コピーと、まだ有効・継承中の入力を除去しない負の対照を追加した。通常のスコープ解析を共用し、形式／hashの検査を保持する。整形・差分・YAML検査は成功、追加RustのGHA検証は待機中。有効な明示スコープ全体の合成と公開全体の残件は維持する。

`6e00b07` の軽量ゲートでは公開入口の修正がコンパイルを通過し、追加した転送往復・nanosecond時刻／mode・内部ハードリンク・破損拒否・tool整理を含むci target41件は成功した。旧workflow試験1件がファイル全体の最初のキー出現位置を比較しており、新しいartifact用outputをcache復元順と誤認した。実際のrestore-keysリストごとに順序を検査するよう範囲を修正し、nativeの2リストと配布検証の1リストを要求する。fallback順・試験範囲と期限は保持し、全環境はまだ開始していない。

`03a037d` の軽量ゲートは、browser成果物検査の分割で2つの公開入口が欠けていたため、xtaskのコンパイル時に停止した。重い本体buildと全環境検証は開始していない。画像検査の入口を再公開し、実行を伴うpanel描画は元の制御モジュールへ戻す。既存の試験・期限は変更せず、修正のRust検証はGHAで先行する。

直近のmacOS保存証拠では、実PTYで受けた日本語・結合文字／ZWJを含む下書き、80×24・50×16・145×38の描画画面とカーソル座標、サイズ変更後の追加入力と保存済み下書きの全文一致を独立照合した。以後も既存のPTY・GHA上のKitty／xterm検証を使い、利用者への反復確認依頼はしない。物理macSKKの未確定文字・候補表示の合格とは区別する。

CIの次の未検証差分は、同じrun・commit・SDK／compiler条件の先行成果物を短期artifactで全体検証へ渡し、依存cache枠に同じfocus payloadを保存しない処理である。転送用tarは時刻のナノ秒精度・mode・Cargoの内部ハードリンクを保持し、展開前の構造検査と既存の全compiler入力／hash検査を分ける。検証済みxtaskの保存直前には、復元元と同じSDK／cache形式の古いtool世代だけを整理する。既存試験へ転送往復・時刻／mode・内部リンク・破損拒否・対象限定の回帰を接続した。ローカルの整形・差分・YAML検査は成功、Rust build／試験／Clippyと再利用実績・短縮量はGHA待ちである。進行中の `de3a5d8` を中断しない。

`de3a5d8` の [GHA](https://github.com/EkeMinusYou/lazypencil/actions/runs/40626927395) はLinux先行406件・61サマリー、全3環境・RustSec・署名付きsourceまで成功した。Mac1,446件・Linux各1,457件、各144サマリーで、従来ケースの回数＋新規2実行を照合した。各768 PNG・42描画計測・clipboard3,072画素・IME保存4画面と、両形式・22経路・3段階の132画像／67,584画素、元ファイル削除後の再コピー・素材revisionを独立照合した。従来671画像のcoverageと4 samplerの全画素再計算、animation・network・transportの従来基準も維持した。jobはMac21分19秒・arm64 18分04秒・x64 52分22秒。x64はimage20261026.354.1のnative cacheがなく、通常build704.36秒・release1296.95秒で、先行のSDKとも異なり成果物を再利用しなかった。全workflow66分11秒を短縮達成とは扱わず、SDK／compiler条件は保持する。source1,154ファイル・32,029,936バイトの全Git blob／mode、tar hash・DSSE署名・source／workflow／runを独立照合した。最後まで中断せず検証した。

次の未検証差分はURL所有文脈を別に保持するbrowser scope v2、空の名前空間、空／未指定URLのplaceholderと27経路・162画像の試験である。画像成果物検査も実行順制御とは別のRustモジュールへ分け、既存の制御関数の内容が変わっていないことを確認した。Rustのbuild・試験・ClippyはGHA待ちで、公開全体の残件は維持する。

`ed72448` の [GHA](https://github.com/EkeMinusYou/lazypencil/actions/runs/40622329188) は先行367件・50サマリー、全3環境、RustSec、署名付きsourceまで成功した。全体はMac1,444件・Linux各1,455件、各144サマリーで、8eの全ケース回数＋新規2実行を照合した。各678画像（元671の667 byte一致＋4 samplerの全画素再計算、新規animation2・network2・transport3）・42描画計測・clipboard3,072画素・IME保存4画面を独立照合した。通常／保持のonline・復帰はSTUN3／TURN1／page QUIC4／worker QUIC4、offline／refreshは各0で、全18 UDP／6 TCP endpointと3色・8,192ケース画素・metadataも一致した。jobはMac21分33秒・x64 16分22秒・arm64 34分52秒。x64は同commit先行106ファイルを再利用し、Cargo既定build確認0.69秒、残るtest catalogueは21.23秒だった。arm64はSDK image20261026.166.1へ替わり対応native cacheがなく、通常build364.44秒・release680.84秒を要した。SDK／compiler条件を維持し、全workflowは46分14秒で全体短縮とは扱わない。source1,154ファイル・32,002,878バイトの全Git blob／mode、tar SHA-256 `5946df4ad22f42e7e36c585ff6557b2168ab71e84c6d666e7ec8ed3497ec200a`、DSSE署名・source／workflow／runを照合した。

次の差分では、生成browserの宣言済み別名の捕捉・再コピー合成と、両形式・22経路・元ファイル削除後の132画像、コピー先の同名libraryを借用しない実worker試験を準備した。変換不要な参照ツリーは複製せず、ガイドを同時更新する場合もbrowserとガイドの試験を先行し、元の全3環境gateを維持する。URLを持たない不一致スコープ、未捕捉文書・旧閉包、明示scriptスコープと本家往復互換の残件は維持する。追加Rustのbuild・試験・ClippyはGHA未確認で、ローカルは整形・差分・YAML検査だけを実行した。物理macSKK候補・最低OS・性能・認証と公開全体の完成条件は引き続き必要である。

`d799a72` の [Linux先行](https://github.com/EkeMinusYou/lazypencil/actions/runs/40620904559) は軽量205件・Clippy、対象build／Clippy、従来のHTTP方針再現を通過した。新しいtransport再現は14.23秒で保持取得のoffline比較に失敗し、全環境は開始しなかった。STUN／TURNは9／3のまま、page／worker QUICは各12→13だった。元の観測は接続先を共用しており、この数字だけでは旧接続の終了と新取得の要求を区別できない。修正では各mode・phaseに別の所有TCP／UDP endpointを与え、以前のendpointを生かしてport再利用を防ぐ。phase別の緑・青・cyan画素で最新HTMLの取得も要求する。通常／保持・refresh・復帰、各protocolの正の対照・offline各0、1500ms観測・既存期限と本体コードは維持する。別Chrome156.0.8078.48の同じHTMLでは3色・通常3／1／4／4・制限時各0を確認した。固定GHA版／修正Rustの合格は未確認である。

`8e9c3b8` の [GHA](https://github.com/EkeMinusYou/lazypencil/actions/runs/40615966446) は軽量205件・Clippy、Linux先行378件／50サマリー、全3環境・RustSec・署名付きsourceまで成功した。Mac1,442件・Linux各1,453件、各144サマリーで、元の4e全ケースの回数＋新規7実行を照合した。通信方針の新再現は7.52秒で長いnamespace試験の前に実行し、同条件の残部からだけ除いた。各675画像（従来671の667 byte一致＋4 sampler全画素再計算、新規animation2・network2画像）・42描画計測・clipboard3,072画素・IME保存4画面を独立照合した。通常HTTP4／WS2／TLS試行8とoffline増加0、metadata／context／refresh・8,192画素・artifact digestも一致した。全体jobはMac22分00秒・x64 18分30秒・arm64 35分11秒。x64の同commit先行／全体はSDK imageが異なりapplicationを再利用しなかった。arm64は旧image20261023.149.3へ戻りnative cacheがなく、通常build380.60秒・release699.46秒を要した。compiler／SDK条件は維持し、cache消失の原因や全workflow短縮を断定しない。source1,152ファイル・31,978,566バイトの全Git blob／modeとarchive hash・DSSE署名／source／workflow／runを照合した。物理macSKK候補・最低OS・性能・認証と公開全体の残件は維持する。

続く所有headless診断では、HTTP proxyだけではWebRTCのSTUN UDPが到達した。通常Chromeとheadless-shellの対応switchを起動へ接続し、proxyもプロセス作成時に一度だけ所有する差分を準備した。network変更時は通常／保持取得の両方でプロセスを更新する。新しいRust試験と同じHTMLを別Chrome156.0.8078.48で実行し、通常STUN3／TURN1／page QUIC4／worker QUIC4とoffline各0を確認した。これは新Rustや固定GHA版の成功証拠ではない。断片化したTCPとUDP分類のfixture試験、通常／保持・refresh・復帰の実取得試験と早期実行の回帰を準備し、有用な8eが最終sourceまで完了した後、次のbatchへまとめる。英語guide1.0.29、追加RustはGHA未検証。認証profile・さらに広いprotocolとhost egress・既存の全公開要件は維持する。

`6aa68ef` の [Linux先行CI](https://github.com/EkeMinusYou/lazypencil/actions/runs/40615471449) は軽量205件・Clippyとnative前のscript確認を通過し、対象Clippyを10.91秒で実行して共有serverの未使用counter／getterとcollapsible-ifを検出した。全環境は開始しなかった。共有helperの別integration単位ではcounterを観測しないことを該当field／getterだけに明記し、条件を統合する。Unix専用importのcfgも合わせる。通信方針の新しい実Chromium再現は長いnamespace試験より前に1回だけ実行し、同条件の先行残部からだけ除く。既存の順序・重複防止回帰とcommand fixtureを拡張し、元の試験・期限・全環境の実行範囲は保持する。修正差分のRust／runtime結果は未確認である。

`4e938b7` の [GHA `40610172472`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40610172472) は軽量205件・Clippy、Linux先行374件／49サマリー、全3環境・RustSec・署名付きsourceまで成功した。既知のnative animation再現を長い試験より先に1.25秒で実行し、同条件の後段だけから除いた。Mac全体1,435件・Linux各1,446件、各144サマリーで、前走全ケースの回数＋新規4実行を保持した。各従来671 PNG（667 byte一致＋4 sampler全画素再計算）・新規animation2画像／2,048画素・42描画計測・clipboard3,072画素・IME保存4画面を独立照合した。停止4時刻59,900ms・再開60,000msとsource／artifact hashも一致した。全体jobはMac15分51秒・x64 11分31秒・arm64 35分03秒。arm64は同SDK条件のtool／native／application cacheがなく、通常build366.72秒・release691.06秒となった。cache消失の原因は断定せず、全workflowの短縮達成とは扱わない。source1,151ファイル・31,943,340バイトの全Git blob／mode・archive hashとDSSE署名／source／workflow／runを照合した。元の6eの実測差は保存されておらず原因は断定しない。物理macSKK候補表示・最低OS・性能・製品全体の残件は維持し、利用者へ再試験を繰り返し依頼しない。

次の差分はnetwork_policyの設定・schema・template・取得metadataと転送しない所有Rust proxyを接続する。ローカルsocket対で停止し、待機の定期pollを避け、受けたheader／URLを保存しない。既定onlineは追加resourceを作らない。HTTP／WS成功対照、TLS sinkへのHTTPS／WSS接続試行（正常TLS本文の対照ではない）、拒否・worker／子frame・cache・新context・失敗後の回復をRust試験へ加えた。同じ所有HTMLを使う別Chrome156.0.8078.48の隔離診断では通常HTTP4／WS2／TLS8、拒否時はすべて0を確認した。これは新Rustや固定GHA版の成功証拠ではない。オフラインのremote main URLを拒否し、redirect先がChromiumエラーページへ置換された場合もunreachableUrlから拒否する。隔離headlessの所有redirect再現でorigin接続0と置換を確認し、selector／追加待機中にも検査する。追加差分はGHA未検証、英語guideは1.0.28。WebRTC／UDP・WebTransportと他の必須範囲は維持する。

`6e25e38` の [Linux先行 `40608512869`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40608512869) はnative animation時刻の停止比較で失敗し、全環境は開始しなかった。設定・cache・保持context更新の追加試験は通過したが、失敗時の各時刻は旧assertionに残らず、原因は未確定である。macOSの別Chrome156.0.8078.48では、所有する一時profile／隔離contextの3回の診断で4 animationの59,900ms保持と再開を確認した。固定GHA版156.0.8078.47のLinux検証の代替とはしない。試験fixtureを本体と同じload完了後にそろえ、pending playのreadyと描画frameを待ってから計測し、失敗時も初期値・観測値・各native状態を保存する。350ms観測・0.01ms比較・全RGBA画素・再開の基準と製品コードは維持する。この既知の再現をdynamic先行の最初のbrowser試験にし、同条件の後段からその1件だけを除く。通常の全環境・並列runtimeでは引き続き実行する。追加Rust／fixture／順序の差分はGHA未検証である。

`360813d` の [GHA `40605012001`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40605012001) は軽量205件・Clippy、Mac先行458件／66サマリー、全3環境・RustSec・署名付きsourceまで成功した。全体はMac19分18秒／1,431件、x64 18分40秒／1,442件、arm64 16分52秒／1,442件、各144サマリーで、前走全ケースの回数と新規5実行を照合した。Macは同commit先行160ファイルを再利用し、通常build2.26秒・catalog79.16秒・release339.01秒・再確認0.80秒。x64／arm64 releaseは203.20／159.81秒でnative cacheを使った。本体変更を含むため、旧releaseの誤再利用や全workflowの短縮達成とは扱わない。各671 PNG・42描画計測・clipboard3,072画素とIME保存画面4枚を独立照合した。Macは全PNG byte一致、Linuxは667一致＋4 samplerの全画素を式から再計算した。80×24・50×16・145×38と追加入力の日本語・下書きhash・cursor・文書不変・agent未起動・端末復元・終了0を確認し、空白frameの採取を解消した。追加待機の実Chromium3件と設定継承も通過した。source1,151ファイル・31,921,213バイトの全Git blob／mode・archive hashとDSSE署名／source／workflow／runを照合した。物理macSKK候補表示、最低OS・性能と製品全体の残件は維持し、利用者へ再試験を繰り返し依頼しない。

次の本体差分は `animation_policy` の設定・取得metadata・native document再生率を接続する。既定runningを維持し、pausedはreadiness／追加待機後に適用する。CSS animation／transition／Web Animationsと停止後の追加animationの時刻・全画素、再開、通常frameの継続、設定継承・cache／保持contextの更新を検査するRust試験を追加した。追加差分はGHA未検証であり、完了した360の検証を基準に対象環境から確認する。英語guideを1.0.27へ更新し、先行のretained_除外へ誤一致しない試験名も確認した。認証・通信・残るanimation範囲・最低OS・性能・製品全体の残件は維持する。

`327c1db` のmacOS先行は成功し、変更した試験helperからの再buildを独立照合した。旧ソースから1,185ファイルを保持・36ファイルを失効させ、アプリの通常library／CLIはCargoのJSONでFreshだった。対象build 4.64秒、通常build 0.25／0.36秒、追加catalog 3.68秒。新しいIME報告は80×24／50×16／145×38のcursorが(6,19)／(6,11)／(6,33)、追加入力後(11,33)と完全な下書きを保持した。ただし保存画面2枚は描画途中の空白で、VT100モデルはZWJのglyph幅を完全再現しないことも独立照合で判明した。下書き・表示文字・cursorを同一の観測フレームで確認して保存する修正を準備する。全3環境と署名付きsourceまで完了してから次の差分を送る。

`327c1db` の [GHA `40600734818`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40600734818) は軽量205件・Clippy、Mac先行7分54秒、全3環境・RustSec・署名付きsourceまで成功した。全体はMac12分17秒／1,426件、arm64 17分46秒／1,437件、x64 26分29秒／1,437件で、各144サマリーと前走の全ケース名・回数を保持した。Macは旧1,185ファイルと同commit先行差分18ファイルを復元し、release確認0.58／0.82秒・実行ファイルとmapのhash一致を確認した。debug/test完全cacheはrelease15ファイルを除き1,206ファイルとなり、upload前に旧Mac application約1.19GBを整理した。arm64はnative cache HIT、x64は同SDK keyのnative cache MISSで通常build310.50秒・release613.35秒となった。cache消失の原因は断定せず、cold走行をwarm高速化とは扱わない。全3組の671 PNG・42描画計測・実clipboard3,072画素、source1,151ファイル・31,907,456バイトのGit blob／mode・archive hashとDSSE署名を独立照合した。物理IMEの候補表示、最低OSと製品全体の残件は維持する。

`e249f07` の [GHA `40595368880`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40595368880) は軽量202件・Clippy、core先行381件／33サマリー、全3環境・RustSec・署名付きソースまで成功した。Linux各1,434件・Mac1,423件、各144サマリーで、元の全ケース名と回数に追加9実行を加えたことを照合した。x64全体16分50秒・Mac21分43秒・arm64 35分20秒。x64は同commit・環境の先行75ファイルを再利用し、core追加build 9.34秒・通常16.86秒・catalog 41.53秒・release 224.77秒・再確認0.22秒だった。arm64はimage `20261026.166.1` への切替でnative／tool cacheがなく、通常381.04秒・catalog159.14秒・release684.48秒を要した。SDK条件は緩めない。各671画像・42描画計測・実clipboard3,072画素を独立照合し、667画像は前ソースとbyte一致、4 sampler画像は全画素を式から再計算した。ソース1,151ファイル・31,889,601バイトの全Git blob／modeとarchive・DSSEのsource／workflow／runを照合した。重複tool依存4件と旧tool完全成果物1件の500,121,791バイトを整理し、完了後は5つのnative SDK cacheが残っていた。全体性能・物理IME・最低OS・公開品質全体の残件は維持する。


先行する [GHA `40585104571`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40585104571)、source `7fe854b` は先行・全3環境・監査・署名付きソースまで成功した。Mac全体14分28秒／1,414件、Linux arm64 13分29秒／1,425件、Linux x64 43分49秒／1,425件、各144サマリー。旧ケース名と実行回数に新しい実Cargo回帰2件を加えたことを照合した。Macは旧debug/test 1,206ファイルと同commitのrelease差分を再利用し、全体のrelease確認は各1秒未満だった。x64はrunner imageが旧版へ戻りnative cacheがなく、通常build 609.87秒・release 1,109.67秒を要したため、全体高速化の達成とは扱わない。各671画像・42描画計測・実clipboardを独立照合し、ソース1,151ファイルの全Git blob・mode・archive hashとDSSE署名・source／workflow／run対応も確認した。Macの元20起動／80操作／100 workerと通常200MiBゲートは維持して成功した。物理macSKKの未確定表示と最低OSなどの残件は維持し、ユーザーへ実機確認を繰り返し依頼しない。

`e249f07` で検証した補助ツールの完全cacheを先に復元し、成功時の依存cache二重保存と再buildを除く。旧ソースのfallbackはCargoのpackage単位cleanで古い試験本体を破棄してからbuildし、依存は残す。SDK・compiler・cache形式ごとに、より新しい完全cacheがある依存fallbackだけを整理する。古いmtimeの変更が未対処では見逃されるnegative controlと、変更後assertionの実行を実Cargoで検査する。runtime導入はFIFO・そのsymlink・directoryを即時拒否し、hash読み取りを固定sizeへ制限し、導入lock待機へ既存installerと同じ60秒上限を付ける。共有lock抽出は文書の既存5秒と取消を保つ。対象core再現を最初に実行し、成功後に元の3環境検証へ進める。このtool方針・導入回帰は上記の全3環境で検証済みである。

`0a8f336` の [macOS 先行 `40574081272`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40574081272) は補助・機能519件／56サマリー、候補検査、元の通常メモリゲートを成功した。20起動／80操作・100 worker profile・通常6役割／Chromium14役割と各完全な30秒 idleを独立照合し、通常最大 RSS 175.171875MiB、cached feedback p95 28.87ms、通常 idle CPU 上限0.433%だった。65 PNG・36,344,288画素は `35e959f` と全 byte 一致した。追加Chromiumは最大1,255.17MiB、2.091秒の membership settling後に30.135秒 idleを保持した。効果のなかった回収要求を削除し、従来の renderer と文書専用MCPの medium設定だけで確認した。独立overrideとadapter環境保持も早期再現を成功した。候補631／evidence626／ソース1,150ファイル、原map4 archive／904 member、FreeType原文12点とJPEG原文2点・謝辞も照合した。releaseの実コンパイル7分21秒とruntimeの並行、最後のCargo再確認0.89秒を確認した。Mac全体も18分57秒・1,412件／144サマリーで成功し、同じcommitの150ファイル、実行ファイル／mapの同一hash、release再確認1.69秒／1.01秒を照合した。671画像と実clipboardの2画像・3,072画素も確認した。Linux x64／arm64も各1,423件／144サマリーで成功し、元の試験名・回数、各671画像、42描画計測、実clipboardの3,072画素を独立照合した。全体はx64 47分17秒、arm64 37分20秒で、両方とも異なるrunner imageのnative cache missだった。ソース1,150ファイルの全Git blob・mode・archive checksumと署名済みprovenanceのsource／workflow／run／digest対応も確認した。統制した性能・物理端末・公開品質全体は完了扱いにしない。

`6225841` の [macOS 先行 `40563884709`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40563884709) は環境設定の早期再現・関連試験・候補と原文通知を成功し、元の全計測後に通常 RSS 216.65625MiBで失敗した。TUI 約106.78MiB・MCP 約93.20MiBが残る。20起動／80操作・100 worker profile・通常6役割／Chromium14役割を独立照合し、65 PNG・36,344,288画素は `35e959f` と全 byte 一致した。候補631ファイル・ソース1,149ファイル・map4 archive／904 member・FreeType原文12点とJPEG原文2点・製品謝辞を確認した。全3環境へは進めていない。次はアプリを再ビルドせず既存候補を再利用する macOS の任意診断を用意し、元の30秒 idle 後に所有するTUI／MCPの allocator summary を取得する。原計測・期限・200MiBは保つ。追加のRustと実GHA診断は未検証である。

`9010cf6` の [macOS 先行 `40560551846`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40560551846) は補助ツール194件・機能試験・候補の展開と隔離 smoke を通過し、元の20起動／80操作・通常6役割／Chromium14役割の全観測を保存した後、通常 RSS 236.50MiBで200MiBゲートに失敗した。全3環境には進んでいない。候補631ファイル・ソース1,149ファイル・原文通知と、元 ld64 map の4 archive／904 memberを独立照合した。65 PNG・36,344,288画素は `35e959f` と全 byte 一致した。固定 clip の試行は戻し、Mac の文書専用 MCP の ACP environment に省メモリ方針を渡す候補を準備する。明示 override と adapter の環境保持を両 provider の実 CLI／疑似 ACP で早期再現し、元の計測・期限・200MiBを維持する。JPEG の原文2ファイルは固定 upstream と一致し、別途必要な製品謝辞と欠損拒否も準備した。新しい Rust 差分は GHA 待ちである。

`4d6f200` の [macOS 先行 `40556268470`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40556268470) は機能試験と release build を通過したが、候補の証跡収集がリンクマップ全体を UTF-8 と仮定して失敗した。全3環境とメモリ計測は開始していない。以前の実マップでも、169,027バイトの object table は有効な UTF-8 だが、後方の literal-string symbol に非 UTF-8 byte があることを確認した。元マップの全 byte／hash を保持し、使用する archive・member だけを厳密に復号する修正と、3形式の opaque symbol／不正な入力パスの回帰試験を準備した。先行 runtime 3分35秒と後続 release 4分30秒を重ねる変更も準備し、両方の回収後に ownership と計測へ進む。Rust の追加差分は GHA 待ちである。

`35e959f` の全3環境・監査・署名付きソースが成功した。各144サマリーの全試験名・回数を照合し、Linux 各1,412件、macOS 1,401件（軽量ツールの新しい scheduling 試験1件を含む）を確認した。ソース1,149ファイル・32,058,232バイトの内容・mode と署名対象を独立照合した。Linux の全体は18分12秒、macOS は20分44秒、arm64 は39分15秒で、arm64 は ImageVersion `20261026.166.1` に更新され、通常 build 6分36秒・release 12分58秒を要した。Linux は同一 commit の先行211ファイルを再利用し、通常 build 0.85秒・catalog 20.49秒・release 0.26秒だった。異なる SDK／cache 条件を混ぜた速度向上とは表現しない。

Mac の別候補 [GHA `40551198843`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40551198843) は15ファイルの release 再利用、配布・署名検証・元の計測を成功したが、通常の最大標本 RSS は255.09MiBで200MiB未達だった。65 PNG・36,344,288画素は前候補と全バイト一致した。検証済み CLI の隔離した headless MCP をローカルで診断し、同じ PNG を3回生成して正常終了した。`vmmap` は約92MiBの解放済み large 領域を示した。可変寸法の native layer を再利用しやすくする Mac 専用の修正候補を準備し、16MiBを超える layer は従来の allocation hint を維持する。すべての元計測を保存した後に通常 bound group の200MiBを判定する CI gate と、リンクマップ由来の archive 一覧・配布時の再計算検査も準備した。新しい Rust のビルド・画素・実測効果は GHA 待ちであり、公開準備全体は未完了である。

`35e959f` の [GHA `40543956605`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40543956605) は Linux x64 の先行・候補・性能計測を成功し、全3環境の検証へ進んだ。元の20起動・80操作・30秒待機、100 worker profile、MCP・両 JS を含む6役割と、Chromium を含む14役割の別計測を独立照合した。通常の最大標本 RSS は168.04MiB、cached feedback p95 は20.26msで、保持した65 PNG・36,344,288画素は `b937eff` と全バイト一致した。Chromium の取得後処理を2.05秒観測した後の30.01秒待機も成功し、全14役割と未知の所有子プロセスを集計した。候補のソース1,149ファイル・配布619ファイルの内容・mode・notice・link map の結び付けも確認した。全環境・macOS の200MiB・完全な性能受け入れ・最終配布レビューは未完了であり、ローカルで重い build は行わない。

`e75b275` の先行 Linux は補助ツールの全試験と core の sampler 再現を成功し、通常構成の Clippy で SHA-256 出力配列の `LowerHex` 未実装を検出した。全体 matrix は開始していない。既存のバイト単位の16進数化へ修正し、同一ハッシュの試験と元の機能・期限を維持して先行検証から再確認する。Cargo の Linux 用 `--jobs 2` の試験は成功し、環境変数と依存キャッシュキーは維持されている。

2026-10-28: `b937eff` の [GHA `40506257246`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40506257246) は先行・全3環境・監査・署名付きソースを成功した。Linux 各1,407件、macOS 1,396件、各144サマリーの元の試験名と実行回数を照合し、ソース配布物1,147ファイルの内容・mode・署名対象との対応も確認した。通常描画の57 PNGは各環境で `3126d3c` と全バイト一致した。Linux arm64 の別候補では、Kitty／xterm の明示・自動選択の4モード、40画面・33,103,860画素と入力後のカーソル位置を独立に確認し、Kitty と Sixel の画面も目視で確認した。これは GHA 上の端末であり、macSKK 固有の候補・未確定文字表示の実機確認を含まない。今後の手動確認をユーザーへ繰り返し依頼せず、実 PTY・端末画像・プロトコルの検証を継続する。

同候補の20起動・80操作・元の30秒待機は維持し、cached feedback p95 は Linux x64／macOS／Linux arm64 の順に19.27／15.42／15.07msだった。MCP・両スクリプト・監視役を含む通常時の最大標本 RSS は185.74／287.63／186.07MiBで、macOS の200MiB目標は未達である。Chromium を含む追加計測は x64 と macOS で完了したが、arm64 は取得済みページの renderer が待機開始後に終了し、厳格なプロセス構成検査で停止した。先に保存した通常計測と端末検証は有効で、追加計測の合格とは読み替えない。後処理中も全プロセスを記録し、その後に元の30秒を計測する修正と、全シーンを複製しない識別ハッシュ計算をローカルで準備した。OS フォントマネージャーも実際の外部フォント検索まで遅延する。これらの Rust ビルド・同一ハッシュ・実測効果は GHA 待ちである。[実測と範囲](performance.md)

`3bfa2f1` の [GHA `40531196449`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40531196449) は先行・全3環境・監査・署名付きソースを成功した。macOS 1,400件、Linux 各1,411件・各144サマリーの元の試験名と回数を照合した。ソース1,149ファイルの内容・mode・署名対象との対応も確認した。各環境の671 PNGのうち667枚は前候補と全バイト一致し、残る4枚は一時ディレクトリの絶対パスを表示する試験で画像幅が変わったものだった。両候補それぞれのコピー前後の全バイト一致と、シェーダーの独立した数式に対する全画素一致を確認した。プロジェクトのエージェント設定では、別用途の実行許可、CLI の明示指定の優先、物理設定先とプロジェクトへの結び付け、取消し・再許可・設定変更前の起動条件の拒否を CLI と実 PTY で確認した。許可前も閲覧と下書きは利用でき、デザイン指示の許可は実行を許可しない。試験で起きた PTY の出力詰まりは、同期的な `ps` の間も画像出力を読み続ける待ち方に修正した。元の試験・15秒の各待ち時間・子プロセス終了の確認は維持する。設定機能の対象3環境の合格は確認したが、公開準備全体は未完了である。

`559c6d1` のMac先行検証は機能・配布候補・元の20起動／80操作と追加6役割の計測を成功したが、最大標本RSSは270.91MiBだった。57 PNGは前候補と全byte一致し、cached feedback p95は37.60ms。効果を確認できなかったmalloc解放呼出しは除去し、200MiB未達は継続する。全体CIのMac1,395件・arm64 1,406件は成功したが、x64は実PTYのscript-config復帰で失敗した。IME確定レポートは成功し、日本語下書きも保持された。x64の依存cacheは実際のImageVersion `20261023.334.3` に対応するものがなく、debug通常build 672.96秒・test catalog 103.39秒・release 1,270.79秒だった。release laneは試験の失敗後も回収して全結果を確認した。試験本体の5百万loop、2,000ms／1ms、各15秒waitを保ち、TUI target全体を元の引数のままrelease開始前に1回実行する。共有900秒に課金し、後段から同じtargetを外す。全体・source合格にはせず、次のGHAで先に再現を確認する。Chromiumを含む独立した追加計測も準備中で、元の試験・期限・合格基準は維持する。

`3126d3c` の [GHA `40483453796`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40483453796) は対象3環境・監査・署名付きsourceを成功した。Linux各1,404件・Mac1,393件、各144サマリーで元ケース名と回数を照合した。同梱の固定Notoだけを遅延共有し、文脈の破棄後・複数threadで従来のコピー経路と比較する追加試験も通過した。ユーザーフォントは従来の所有データと文書ごとの設定を保つ。

Mac候補 `40488551412` とarm64候補 `40488556293` も成功した。元の20起動／80操作・30秒idle、100 worker記録と、実MCP・両JSを含む6プロセスの追加走行・別30秒idleを独立照合した。元の48 PNG／環境は `361a8ef` と全byte一致し、追加走行も7画像／環境・7,837,200画素のCRC・hash・復元を検査した。本体側の最大標本RSSは x64 189.50／Mac271.00／arm64 192.35MiBであり、Macの200MiB超過は残る。機能成功を性能合格へ読み替えない。

arm64の実Kitty／xtermは4モード×10phase・40画像を成功し、native cursor・拡大縮小・移動・resize・復元を独立に確認した。3環境のPTYでは日本語確定・結合文字／ZWJ・カーソル座標と元データ保持も通過した。利用者への再試験依頼を繰り返さず、この代替検証を継続する。物理的なmacSKK候補表示は未確認である。

`74748c6` はアプリの完全なcompiler dep-infoに消費先がない実行時専用helperの変更でも既存成果物を保持し、重複した完全checkpointを作らない。欠損dep-infoと本体側の消費先は引き続き拒否し、Cargoの鮮度判定を維持する。実Cargoを使う回帰を含む先行185件と、Macの全体1,394件・144サマリーは成功した。Macは1,221ファイルを再利用し、releaseのCargo確認は0.54／0.91秒だった。Linux両環境も各1,405件・144サマリー、監査と署名付きsourceを成功した。新imageのcacheなしのrelease compileはx64 713秒／arm64 983秒だった。runner image更新による依存cache不一致は強制回避しない。

キャッシュ一覧では補助ツールの完全成果物が9件・約1.33GBを占めていた。同じrunner／compiler／cache形式ごとに最新1世代を保持し、古い補助ツールのソースは別に保持した依存cacheから再ビルドする候補を用意した。事前一覧での見積りは4世代・590,992,236バイトだった。`559c6d1` の先行186件と保持方針の回帰を通過し、実際のcleanupは古い完全tool cache 3件・457,231,355バイトを削減した。依存cacheは削除対象にしていない。環境・compiler・cache形式の分離、PRと依存cacheの保持は元の回帰試験で検査し、試験件数や機能の条件は減らさない。Mac向け候補と同じpushで検証した。

`74748c6` のLinux x64全体も1,405件・144サマリーを成功し、元のケース名と回数を照合した。新しいimageでは依存cacheがなく、release compileは713秒、最後のCargo確認は0.13秒だった。JSONのtest build 407.92秒の後にnormal build 24.05秒がSkia bindings等をコンパイルし、次のTUI再現でも本体を13.89秒コンパイルしていた。通常・testの両構成と元のコマンド・期限を保ち、normal buildを先に置く候補を同じpushへ加える。実Cargoの回帰は、normal／devの依存featureを異なる値で検査し、testの再build判定がfreshになることと実行成功を確かめる。新しい順序と回帰はGHA未検証である。

Macの配布CLIを隔離した追加観測では、描画後に約92〜94MiBの解放済み大領域がmallocに保持されていた。次の候補は、描画の作業領域がdropした後にAppleの公開APIで未使用allocatorページの返却を要求する。保持中の画像・layout・font・runtimeの所有権は保つ。効果・遅延・画素は未検証であり、有用な進行中CIの終了後にMac先行で確認する。元の試験・期限・性能目標は変えない。

`2d39f79` の [GHA `40470633331`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40470633331) は軽量181件・Clippy、対象3環境・監査・署名付きsourceを成功した。Linux各1,400件・macOS1,390件、各144サマリーと元ケースの回数を照合し、全3環境でflags5・日本語確定・結合文字／ZWJ・native cursor座標・未送信・文書保持と端末復元を確認した。Linux実Kitty／xtermは4モード×10phaseの画面をdebug／配布候補とも通過した。Mac候補 `40474611969` は同コミットのrelease15ファイルを再利用し、梱包・署名付き証跡・元の20起動／80操作も成功した。macSKKの候補ウィンドウ自体は未確認で、利用者への再試験依頼は繰り返さない。

既存の資源観測は静的ビューアー1プロセスだけのため、MCP・起動中のJSワーカーを含める設計条件には不足している。別走行のdashboardヘッダーを元の子ノードを返すJSへ置き換え、同じ1,000展開ノードとTUI画素を保ち、実MCPの2描画とTUIの4操作の間に所有プロセス群を観測する。生の全体RSSを残し、識別した疑似ACPだけを別集計する。本体・supervisor・MCP・両JSを含め、PIDの役割・終了後の消滅・元データ保持を検査する。元の20／80・idle・期限は変更しない。新しい走行と200MiB条件はGHA未検証である。

`1bde3c5` の GHA `40448997560` は180 tool試験・Clippy、3環境・監査・署名付きsourceまで成功した。Linux各1,398件・macOS1,388件、各144サマリーと元ケースの回数を照合し、実Notoの元比較と線の78画像／環境を独立に検査した。Mac候補 `40451759065` もsource1,147ファイル・通知・binaryの対応、元の20起動／80操作と別資源観測を通過した。macOS27.0.1では配布CLIのmesh26ノードと実JS256画素も成功した。

同候補の利用者によるKitty操作では、日本語の変換確定で何も入力されなかった。拡大縮小は動いていそう、サイズ変更と画像添付はたぶん成功という報告であり、後者も確定した合格にはしない。全キーのescape-code報告を要求しながら、入力libraryがIMEのassociated-textを扱わない構成を修正する。通常文字のUTF-8経路を残し、修飾キー・解放・代替キーの識別は保持する。元の端末再現試験の先頭で、交渉したflagsに従うIME確定、分割UTF-8、結合文字／ZWJ、未送信と文書保持を検査する。修正後のGHAと利用者による実IME確認はまだ必要である。Kittyのcomputer-useは利用者の許可後もツールが安全上の理由で拒否したため、迂回していない。

`1c0ffa0` の macOS先行は358件・36サマリー、Clippy、入力再現、native clipboard、配布候補と元の20起動／80操作を成功した。候補のソース1,147ファイル・全構成物・通知・binary対応と隔離JS256画素を独立照合し、ローカルmesh画像も元の画素hashと一致した。利用者は修正版で文字を入力できたが、macSKKのスクショに未確定文字とカーソルの位置ずれが見られた。端末カーソル位置を通知していない描画を修正し、文字装飾の擬似カーソルからnative cursorへ切り替える。Unicode幅・スクロール・resize・focusと実PTYの位置を検査し、IMEレポートも保持する。追加修正はGHA未検証で、実macSKKの完全合格は未確定である。利用者は以後の自律確認を指示した。Kitty操作は再許可後もツールに拒否されたため、利用者への反復依頼はせず、許可されたGHA実emulator・PTYによる検証を進める。

`30c105f` の macOS先行437件・68サマリーと配布候補・元の20起動／80操作を成功し、入力レポートの文字・座標・未送信・文書保持を独立照合した。元Noto78 PNGも全byte一致した。追加確認で、pinned textareaの論理列がZWJ成分を別々に数えることを確認した。描画済みcaretを一時的なcell属性で識別し、flush前に属性を除去してnative cursorを同じcellへ置く。独自のUnicode幅計算やforkを追加せず、結合絵文字・旗・家族・tab・選択範囲と属性非漏洩を試験へ加える。元の全条件と期限は維持し、有用な進行中CIを中断せず追加修正をまとめて検証する。

`091ca6e` は macOSの日本語／結合文字・ZWJの実PTY再現、Clippyとnative clipboardを通過したが、元の長い日本語行でcaret spanが横スクロールの広字境界から切れ、native cursorが消えることを既存条件の試験が検出した。全環境とreleaseには進めていない。描画spanが取れない時だけwidgetのviewport座標で表示位置を保持する。元の98列／48列の位置・resize・全文保持と新しいUnicode／選択／非漏洩条件を変更せず再検証する。

`9abd444` は先行379件・44サマリー、全3環境（Linux各1,399件／Mac1,389件・各144サマリー）、監査・source gateを成功した。配布候補と入力文字・日本語／ZWJ位置・無送信・文書保持を独立照合した。Linux実端末の追加検査は全4モードのtyped-cursorで停止したが、PNGはカーソルが正しく9画素（1cell）動いたことを示した。検査側がright/bottomのexclusive境界をinclusiveとして扱った誤りである。正しい境界規約に修正し、元のサイズ・色・入力・geometry・終了条件と期限を保つ。アプリ本体の差分は追加せず、同一コンパイラー入力の配布成果物を再利用して検証する。

`6059042` のLinux実端末先行は Sixel／auto-Sixel の全10phaseを成功した。Kitty／auto-Kittyもnative cursorの入力・削除・非focus時の非表示を通過したが、zoomで停止した。実read traceは `Tab` とBackspaceが2回、plain `+` は1回届いたことを示し、画面はPreviewを越えてNodesへ移っていた。Kitty0.32.2で不要なevent-type報告を要求するとlegacy releaseも押下と区別できないbyte列になる。入力は押下だけを使うため同報告要求を外し、UTF-8・修飾キー識別・alternate keyを維持する。mode交渉でbit2が無いことも検査する。混在差分にLinux専用実emulator runnerが含まれる場合はLinuxを先行し、BSD bridgeだけの差分は従来通りMac先行、明示overrideも維持する。元の試験と期限を変えず検証する。

`67e799b` の [GHA `40442963340`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40442963340) は3環境・監査・最終ソース検証を20分4秒で成功した。元のケース名・実行回数を照合し、追加回帰を含めLinux各1,394件・macOS1,384件、各144サマリーを確認した。各1,221ファイルを再利用し、core／既定buildは0〜5秒、並列runtimeは150〜227秒、releaseのCargo確認は0.18〜0.56秒だった。native依存3件と完全成果物3件を保持し、旧形式重複は0件。別走行のmacOS／arm64配布計測も元の20起動・80操作と追加資源観測を成功し、raw記録・画素・会話を照合した。物理表示・全性能・製品全体の完成とはしない。

カラー／bitmap書体の文字の線は `869f0e5` の Linux 先行402試験・39サマリーを通過した。macOS／arm64の全体も各1,387／1,397件・144サマリーで成功し、元ケースと回数を照合した。3組の18画像、合計5,616,000画素のhash・穴・低alpha部分・paintとschema一致を独立に確認した。通常vector輪郭とnativeの色塗りを維持し、layout identityは7である。最終Linux x64は試験用Notoフォントの取得中に30秒のper-read timeoutで停止し、描画失敗とは区別する。他の有用な実行は終了まで保持した。追加バッチでは元の実Noto14比較／形式を維持し、CBDT／COLRv1の線を18条件／形式で検査する。固定フォントと原licenseの3ファイルだけを共通cacheへ置き、既存Rust処理で毎回サイズ・SHAを再確認する。cache破損・symlinkの拒否も実CLIで検査する。取得の既存300秒global／30秒per-read条件とnative全試験を維持し、取得を重い依存cache復元より前へ移す。失敗した入力名も診断に残す。追加差分はGHA未検証であり、全3環境と最終source gateの成功はまだ示さない。

配布検証の `40440434615`（macOS）と `40440440922`（Linux arm64）は、候補をメモリ内で展開し、ソース1,146ファイル・全構成ファイル・mode・SHAと隔離QuickJSの256画素を独立照合した。macOSの埋め込みFreeType原文12件・178,787バイトと表示、Linuxのsystem FreeTypeを区別した。各20起動・80操作・100個の元worker記録と診断画像を照合し、キャッシュ画像のPTY受信p95は38.11ms／14.27ms、完全画像は278.90ms／219.85msだった。物理表示や全性能条件の合格とはしない。Xvfb修正後の Linux x64 `40441668699` は4端末モード・署名付き候補・計測まで成功した。追加照合を進める。

端末検証モジュール `xtask/src/terminal/` の変更も、設定だけの確認で終わらず実emulatorと配布候補の先行ゲートへ送る修正を準備した。動作中のメモリ証跡も不足していたため、元の20起動・80操作・30秒idleの後に別の資源観測走行を追加する。50msを目安にOSのプロセス群RSS・CPU・観測時間・実際の間隔を残し、同じ文書の起動と4操作、元画素への復帰、正常終了・会話を開始しないことを検査する。元の応答時間分布へ混ぜず、瞬間ピークや物理表示・200MiB条件の完成を主張しない。これら追加変更はGHA未検証であり、進行中CIを中断しないでまとめて検証する。

`1aae305` は [初回CI `40433942703`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40433942703) と [同一コミットの再利用確認 `40438125730`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40438125730) が、3環境・監査・最終ソース検証まで成功した。初回40分45秒、変更なしの再実行17分52秒で、先行試験の選択条件も異なるため時間差すべてをキャッシュの効果とはしない。両実行で各Linux1,393件・macOS1,383件、各144サマリー、元のケース名と実行回数を照合した。再利用時は各環境1,221ファイルを復元し、release確認は0.17〜0.64秒、native依存3件を保持し、完全成果物3件・旧形式重複0件・保存件数増加0を確認した。色索引は必要時に一度だけ生成する修正を検証済みで、元の並列runtime構成と期限を維持した。

同じソースの配布候補検証では macOS `40440434615` と Linux arm64 `40440440922` が署名付きビルド証跡・候補の隔離smoke・TUI計測まで成功した。Linux x64 `40440437702` は明示Kitty／自動Kitty／明示Sixelを通過したが、自動Sixelのxtermが画像試験前に `Can't open display: :99` で終了した。画素の不一致やPencil互換性の証拠とは区別する。候補の再生成前に、所有するXvfbをクライアント間でリセットしない設定と、起動失敗の終了状態・上限付きstderr診断を追加する。30秒の起動期限・元の画素条件・4モードの必須範囲は維持し、修正後の検証を待つ。

`f99fa5c` の [GHA `40432773921`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40432773921) は軽量179試験・Clippy後、macOS先行の単独PTY再現で同じ失敗を18.9秒で検出し、全環境へ展開せず終了した。完全画像2枚・次画像236KB受信途中という追加ログと入力処理を確認し、共有CPU負荷という仮説を棄却した。色検索を画素の線形走査へ変えたことで、BSD PTYの細かいreadごとの待機条件が同じ画像を繰り返し全走査して受信を詰まらせていた。修正候補は不変のRGBAに対し必要時だけOnceLockで色集合を生成する。画素比較・hash・PNGと索引の有無を分離し、計測専用の画像は索引を作らない。不要なruntime直列化は撤回し、元の15呼び出しと並列構成・期限を戻す。共有observerを参照する実library-test単位もcompiler dep-infoで無効化し、本番libraryをFreshのまま保てる回帰試験を追加した。これら修正候補はGHA検証待ちであり、原因修正や全環境合格をまだ確定しない。

`79d3fb8` の [GHA `40429994256`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40429994256) は軽量178試験・ClippyとLinux先行候補、Linux x64／arm64全体を成功した。Linux全体は同じコミットの先行成果物66ファイルを再利用し、releaseのCargo確認は0.14秒だった。一方macOSの並列runtime工程で保持browserページのPTY再生が既存15秒の青色更新待ちに失敗したため、全体成功とは扱わない。次のバッチは共有端末observerの利用先であるこの実browser再現をmacOS先行に含め、並列runtime負荷をかける前に同じ条件で実行する。残りのdynamic試験から当該1件だけを除いて全ケースを一度ずつ保持し、browser全体の1,200秒予算と各15秒待機を維持する。共有負荷の影響の切り分けと修正後の最終3環境検証は未確認であり、合格を先取りしない。

共有Rust試験ヘルパーの変更にも既存の互換キャッシュを適用する追加改善を準備した。実 compiler dep-info から影響する全integration単位を無効化し、無関係な本体・example・試験と依存を保持する。ヘルパーを本体などが参照する場合、単位のdep-info欠損、ファイル集合・mode・recipeの変化は再利用を拒否する。実Cargoを使う回帰試験では古いmtimeを維持して変更し、複数の利用試験の更新・変更後コードの実行・先行差分だけの保存を確認する。対象候補の拡張で互換キャッシュの共通contextは一度変わるが、native依存とrelease専用keyは維持する。端末試験observerの未使用色集合の毎画像生成も除去し、必要時には元RGBAから検索する。これら追加差分はGHA検証待ちであり、下記の既測定時間へ上乗せの短縮を主張しない。試験範囲・並列度・期限・計測分離は維持する。

CI短縮の検証基準は `347ae09` の [GHA `40417707707`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40417707707)。同じ条件での成果物再利用、完全成果物の重複保存の削減、独立した browser／MCP／lifecycle／Material 試験の並列化と通常試験の負荷分散を検証し、3環境・監査・署名付きソース確認を22分6秒で成功した。比較元の28分41秒から約23%短縮し、通常試験の各ケースと実行数、元の15個の runtime 呼び出し・引数、画素と完了基準は維持した。この時間は再利用可能なアプリ成果物がある場合の実測であり、変更したアプリの最適化コンパイル時間を含む保証ではない。

続く `3a1dbb0` の [GHA `40421503533`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40421503533) は、TUI の毎フレームの PNG 圧縮・展開を直接画素転送へ置き換える。Linux 先行の軽量170試験・Clippy、対象 native 試験・release ビルド・配布候補と20起動／80操作の計測は成功した。独立照合した100個の元の計測記録で画素コピー平均2.59ms、画像出力 p95 は219.34msだった。従来の44 PNGはバイト一致した。その後、3環境・監査・署名付きソース検証まで38分41秒で成功した。全環境で新しい全アルファ／チャンネルの画素転送とPNG再生成を確認し、通常試験はLinux各1,162件・macOS1,152件と、従来の全ケース・実行数に追加1件を保持した。静的9画像・6scene hashと、配布ソース1,145ファイル・31,561,890バイトのGit・tar・mode・署名対象の対応も独立照合した。100ms目標・物理表示・公開準備全体の完成は示さない。設計にあるパン・ズーム中の既存画像再利用を次のバッチとして準備し、仮表示を完全画像の書き出し・画素比較・元の応答時間へ代用しない。進行中CIを中断せず完了まで待ったうえで、TUI／renderの先行候補検証・Mac共有PTY／クリップボード・元の3環境最終検証へ進める。配布検証でも既存のrelease専用キャッシュを使い、Cargoの鮮度確認を維持する。

独立照合済みの描画・配布ソースの基準は `aafc705` の [GHA `40292925670`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40292925670)。軽量143試験・Clippy、先行検証、macOS Apple Silicon と Linux x64／arm64、依存監査、最終ソースと署名の検証を成功した。全3環境のフォント288画像・19,968,000画素、生成 browser 126画像・64,512画素、独自 contract 36画像・4,054,272画素と、実行ファイル・link map・ソース・計測の対応を独立に照合した。配布ソース1,135ファイル・31,372,804バイトは Git の内容・mode と実際の tar の全メンバーに一致した。

`2123963` で見つかった影付き文字7,407画素の差は、親の効果・半透明と offscreen 描画で従来の合成を維持して解消した。その後の `772bb35` の [GHA `40352211495`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40352211495) は軽量153試験・Clippy、先行・全3環境・監査・最終ソース検証を35分20秒で成功した。通常／core・再現は Linux 各1,161件・macOS 1,151件、各99サマリー、実 browser 各43件と MCP／lifecycle／Material 各7件を維持した。文字36条件・自制作例936,000画素の厳密比較、静的9画像・6 scene hash の不変、配布ソース1,139ファイルの内容・mode と署名対象の対応を確認した。Linux x64 は同じコミットの先行成果物126ファイルを再利用し、release の再確認は2回とも0.16秒だった。

同実行の1,000ノード・20起動・80操作では、TUI 専用の可逆 PNG 圧縮の平均時間が32.48msから16.88msとなり、画像出力 p95 は236.03msだった。24診断画像と20素材画像は従来とバイト一致し、100個の元の計測記録も照合した。CLI／MCP の標準圧縮設定は維持する。入力 p95 は17.91ms、初回画像 p95 は237.24msで、100ms のパン・ズーム目標と物理表示・統制した cold／プロセス群全体の性能判定は未完了。以下の過去の数値を現在の性能とは扱わない。

CI の整理後、`cb437c2` の [GHA `40302309021`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40302309021) は先行・全3環境・依存監査・最終ソース検証を19分20秒で成功した。軽量144試験・Clippy、通常／core・再現試験は Linux 各1,159件・macOS 1,149件、各99サマリー、各環境の実 browser 43件と MCP／lifecycle／Material 7件を維持した。各環境は入力と hash を照合した `aafc705` の1,211ファイルを再利用し、release の Cargo 確認は Linux x64 0.14秒・arm64 0.16秒・macOS 1.41秒だった。待機は先行2秒・全体2〜4秒で、ビルドより通常試験と実 browser が残る主要な時間だった。release と browser の並列化に加え、CI 選択試験だけの変更から、変更のないアプリの20起動計測を外した。計測対象となる実装変更では20起動・80操作を維持し、試験や期限を削減していない。直前の `6b8d65d` は26分54秒だった。今回の19分20秒はアプリ成果物を再利用できる条件の実測であり、cold build の保証ではない。詳細と観測条件は [性能記録](performance.md) を参照。以下の日付付き段落は過去の記録であり、完成条件は末尾の必須要件を維持する。

2026-10-26: `d1f39c4` の [GHA `40252162690`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40252162690) は軽量118試験・先行・全3環境・監査・最終 source を成功した。通常／core は Linux 各1,156件・macOS 1,146件、各99サマリー。実 Noto Emoji 2形式と自作 sbix の288画像・19,968,000画素、生成 browser の126画像・64,512画素、独自 contract の36画像・4,054,272画素を独立に照合した。全3環境の map・実行ファイル・source・計測の対応、静的 release の9画像と6 scene hash の不変、配布ソース1,122ファイル・31,204,208バイトと Git の内容・mode の一致を確認した。macOS と Linux arm64 の別候補も、同じ実行ファイルを再コンパイルせず、全構成ファイルと隔離 QuickJS 描画を独立確認した。macOS に埋め込む FreeType は固定 upstream の原文12ファイル・178,787バイトと配布表示を照合し、Linux は実 ELF の system `libfreetype.so.6` と区別した。製品全体・実端末・性能・正式配布の完成判定とはしない。

同実行は64分57秒で、全3環境の開始待ちは2〜4秒、Linux x64 の既定試験 build は798秒、release と独立試験の並列工程は1,167秒だった。失敗済み実行の後半で完成した依存を固定 checkpoint key に保存できない衝突も確認し、phase・run・retry ごとの immutable key と世代回収を Rust に追加した。`2b1076e` では追加分を含む軽量124試験・Clippy と候補作成が成功し、Linux x64 の配布候補は `d1f39c4` の実行ファイルを再利用した。ただし新しい実端末 gate は Kitty の拡大で失敗した。自作文書の絶対配置を明示した再現でも、一度の Tab が通常入力2件として届くことと、能力応答が下書きへ混入することを実際の read 記録で確認した。全キーの押下・解放を区別する要求、明示 protocol と直接 Kitty 接続での不要な照会の除去、明示／自動の両 Kitty・Sixel と空の下書きを検査する修正を追加した。修正版の実端末・全体 gate は検証中であり、キャッシュの速度改善と端末互換性の成功を先取りしない。

2026-10-26: `6d6c105` の macOS 全体検証は、通常／core の1,146件・99サマリー、実フォント2形式、生成 browser、独自 contract、実 Chromium と並列の release compile／lifecycle／Material を成功したが、後続 Clippy の `items_after_test_module` で失敗した。配布画像・計測・link map の最終段には達しておらず、最終ゲート成功とは扱わない。取得した96フォント画像・6,656,000比較画素、42生成 browser画像・21,504画素、12 contract画像・1,351,424画素は独立に照合した。テストモジュールを末尾へ移し、同じ全対象 Clippy を早期再現試験の直後へ移す。fixture README が候補ゲートを外す分類も修正し、実 compiler 入力になった場合の cache 拒否は維持する。キャンセル時にも完了済み依存からアプリ成果物を除去して checkpoint 保存を試みるようにする。いずれも修正後の GHA 実行は未検証。Linux 両環境は core・既定 build・実フォント・通常試験を成功しており、既知の失敗がある実行の残りは通常の cancel を要求した。

2026-10-26: FreeType の固定ソース `264b5fbf5b912b39f98d038bf75d39be0a73f21b` の `LICENSE.TXT` を確認し、一般的なファイル名判定で漏れる FTL／GPLv2 と別ライセンス部分の原文12ファイルを native notice 収集の必須入力にする。FTL を選択し、配布表示に FreeType Team の利用表記を加える。全原文のバイト保持・hash・公開束の照合と、各入力欠損時の拒否試験を追加する。収集処理の変更も実配布候補の先行ゲートへ送る。ローカルは整形・差分検査までで、追加した Rust 試験と実候補での収集は GHA 待ち。進行中の `6d6c105` 全3環境検証を中断せず、その終了後に送る。

2026-10-26: `e137ff1` の [GHA `40242040657`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40242040657) は xtask 113試験・先行・全3環境・監査・最終 source を成功した。自作カラー書体の4試験が、通常描画の固有色・ZWJ／modifier／selector・混在文字・装飾・gradient／image／mesh・半透明合成・CLI と library の一致を各環境で通した。通常／core は Linux 各1,152件・macOS 1,142件、各98サマリー・407必須画像・42 release 書き出しを維持した。生成 browser の126画像・64,512画素、独自 contract の36画像・4,054,272画素、全3環境の map／実行ファイル／source／計測の対応を独立照合し、静的 release の9画像と6 scene hash は `44ad881` と一致した。ソース1,118ファイル・31,154,521バイトも Git の内容・サイズ・mode と一致した。全体は38分17秒。Linux x64 は同じコミットの先行成果物166ファイルを再利用し、既定試験の build は39秒だった。変更したアプリの release compile と、その後の0.25秒の Cargo 再利用確認は区別する。CI のみを変えた前回との単純な速度比較はしない。次に、公式 Noto Emoji の未改変 CBDT／CBLC と COLRv1 を版・サイズ・SHA-256 固定で hosted 検証時だけ取得し、元の OFL を保持する Rust xtask と、2形式それぞれの実合字・固有色・両 schema の native Paragraph／通常描画の画素一致試験を追加する。`0747efb` の先行では両形式の色付き5ケース・両 schema の20比較を通し、30画像・2,080,000画素を独立照合した。その後、公式 SVG でも無彩色の家族絵文字に色相を要求した試験の誤りで失敗した。黒・189・224・白の元の4階調を各100画素超要求するよう修正し、ほかの色付き条件と全画素一致は維持する。修正後の `c5fe339` は Linux の先行を成功し、両形式各7ケース・28比較・42 PNG の2,912,000画素、元ソース・フォント hash・実際の合字と色／グレーのパレットを独立に照合した。全体の早期試験では Linux x64・arm64 がともに成功し、各42 PNG・2,912,000画素を独立照合した。macOS は CBDT の読み込み失敗と COLRv1 の白紙を再現したため、失敗済み実行の残りの後続試験を停止した。macOS の該当形式だけでファイル用 FreeType を遅延生成し、通常書体・COLRv0・sbix と system lookup の CoreText を保つ修正を追加する。layout engine は6へ進める。自作 sbix の2 PNG・strike・dupe・CRC・全入力画素を軽量 Rust で検査し、合字と3サイズ・両 schema の36比較・54画像を native Paragraph と照合する試験も追加する。修正後は macOS を先行し、その成功後に全3環境を必須とする。`6d6c105` では軽量117試験・Clippy・Wasm/source gate を成功し、macOS 15 の先行 native compile・元のカラー文字・実フォント2形式・sbix と選択した layout／render 試験を成功した。96 PNG・6,656,000比較画素を独立照合し、実絵文字・混在文字・拡大 sbix を目視した。sbix の元フォント bytes は単独保存していないため、生成元の hash は native 試験と報告の一致までの確認とする。全3環境・新 binary の native source／license／link map／配布を引き続き検証する。全絵文字形式・製品全体・性能・正式配布の完成とはしない。

2026-10-26: `44ad881` の [GHA `40238504077`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40238504077) は xtask 111試験・先行候補・全3環境・監査・最終 source を成功し、24分6秒だった。通常／core は Linux 各1,148件・macOS 1,138件、各407必須画像と42 release 書き出しを維持した。ソース1,116ファイル・31,126,313バイトを Git と照合した。別実行を含む全3環境の候補は、全構成ファイル・元ソース・収集表示・map と実行ファイルの対応、および元の native source／build tree を隠した QuickJS の768画素を独立確認した。各実行ファイルは `b15bfa5` と同じ hash であり、3環境とも不要な release 再コンパイルを避けた。手元の macOS 27.0.1 でも同じ配布物の CLI・自作例・QuickJS は成功した。自作例は GHA の macOS と1,539画素が異なり、差はすべて宣言された文字領域内だった。原因は断定しない。Kitty／WezTerm の実操作はツールに拒否され、迂回せず未検証として残す。製品全体・性能・正式配布の完成判定とはしない。

カラー文字の通常描画は Paragraph の出力をアルファマスクにして塗り直すため、フォント固有の色が失われる。第三者のフォントを転用せず、OpenType の公開構造に従う自作 COLR／CPAL・ZWJ／modifier の GSUB・異体字 selector の cmap を Rust で生成する再現試験を追加する。まずネイティブ組版と固有色を確認し、通常描画でも色を保つことを両形式で要求する。新しい再現試験は GHA 未確認であり、修正や絵文字対応の完了とは扱わない。

2026-10-26: `2b9d12c` の [GHA `40237973288`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40237973288) は xtask 110試験と Clippy、Linux の先行 build／描画を成功したが、全3環境を開始する前の候補用 notice 収集で失敗した。release の Cargo 確認は0.17秒で既存成果物を再利用できていたものの、収集側に「新規コンパイルのみ」の古い条件が残っていた。正常終了した Cargo の再利用／新規コンパイルを双方受け入れ、実際の `fresh` を記録するよう修正する。manifest・profile・実行ファイル位置・native 入力・ソース・map の検査は維持し、欠落／不正な `fresh` と異なる manifest を拒否する。収集処理だけの変更も同じ候補ゲートの対象へ加える。修正後の Rust 実行と実候補作成は未検証である。

2026-10-26: `b15bfa5` の [GHA `40234583740`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40234583740) は先行・全3環境・監査・source がすべて成功した。Linux 各1,148件・macOS 1,138件の通常／core、各407必須画像と42回の release 書き出しを通した。生成 browser の126画像・64,512画素と既存 contract の36画像・4,054,272画素、全3環境の map／実行ファイル／source／計測の対応を独立照合した。静的 release の9画像と6 scene hash は `fb05d40` と一致し、配布ソース1,113ファイル・31,071,034バイトも Git の全内容・サイズ・mode と一致した。所要時間は35分46秒で、先行11分14秒、全体 x64 16分13秒・ARM 23分1秒・macOS 20分50秒だった。次の CI は browser-only の先行範囲を絞り、配布変更では候補の実作成・展開・隔離 QuickJS 描画を代表1環境で先に確認する。最終3環境の試験と期限は維持する。新しい配布処理と対象選択はまだ実行未検証である。

2026-10-26: 生成 browser の `b15bfa5` は [GHA `40234583740`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40234583740) の先行 Linux を成功した。非表示の保存用部品を除き、実際の生成ノードを検査する fixture に直した上で、両形式・7経路・元ファイル削除後と再コピー後の42画像を独立照合した。全21,504画素・86,016 RGBA 成分、PNG hash、前後のバイト列、素材 revision は一致した。宣言なしの同名 URL と、コピー先だけにある未知ページの拒否も通した。全3環境の最終検証は進行中である。

対応ソース・全収集済み表示・検証記録を含むバイナリー配布候補を `cargo xtask dist binary`／`verify-binary` で作成・再検証する実装を追加する。アーカイブと展開処理はソース配布と共有する。初回3 target のヘッダー、ソース・実行ファイル・link map・notice の対応、全構成ファイル・権限・hash を確認し、別ソース・CPU・実行ファイル・欠落した表示・改変を拒否する。GHA は通常検証の実行ファイル／map を復元し、強制再ビルドせず Cargo で再確認する。展開後は元の native source／build tree を隠して版表示と実 QuickJS 描画を試験し、失敗・panic・途中失敗でも元へ戻す。新しい配布コードと試験はローカルの整形・actionlint・差分検査までで、Rust の実行は未検証である。進行中 CI を中断せず、終了後に送る。正式リリースのレビュー・署名・環境保証は継続する。

2026-10-26: 共通 release recipe の `fb05d40` は [GHA `40231164855`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40231164855) の先行検証・全3環境・監査・source を成功した。xtask 103件、通常／core は Linux 各1,145件・macOS 1,135件で、各環境の365必須画像と42回の release 書き出しを維持した。全3環境の raw map、直接ロード宣言、実行ファイルと benchmark の対応を独立照合し、静的9画像と6 scene hash は以前の基準と一致した。独自 contract の36画像・4,054,272画素と、配布ソース1,112ファイル・31,036,145バイトも照合した。先行 Linux の release コンパイル約194秒に対し、同じコミットの54ファイルを再利用した全体側の Cargo 確認は0.19秒、最終確認は0.21秒だった。全体は31分36秒であり、異なる runner 負荷から短縮の因果関係までは主張しない。アプリに埋め込まない native notice の確認資料をアプリキャッシュの識別から除外する追加変更を用意し、検査そのものと compiler dep-info の拒否条件は維持する。製品全体の完成条件は継続する。

2026-10-26: 生成 browser の独立コピーで、script の宣言済み Web 素材 namespace を `browser_library` と既存 bundle に保持する実装を追加する。URL を所有する文脈、metadata の継承、ref／descendant／children の上書き、入れ子 script、再コピーを扱う。両形式の7経路と42画像、同じ URL の異なるコピー先素材、未宣言ページの拒否を試験に追加し、先行検証で早期に実行する。新しい実装は GHA 未検証である。進行中の `fb05d40` の release recipe 検証は先行 Linux が成功し、全3環境の終了を待ってから次の変更を送る。試験・終了待ちの基準と初回リリース対象は維持する。

2026-10-26: 共通のローカル Web 描画 `03fe238` は [GHA `40212537150`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40212537150) の先行 Linux・全3環境・監査・最終 source を成功した。通常／core は macOS 1,132件、Linux 各1,142件で、各環境365画像の必須確認と release の42回の書き出しを通した。新しい共通描画14枚と CLI／MCP 8枚は全3環境の73,728画素を独立照合し、素材 revision、保持 page の更新、元ディレクトリ削除後の bundle、CLI と MCP の PNG バイト一致を確認した。コピー128枚と script override 84枚も各環境で全画素一致し、静的 release 画像は以前の基準と一致した。配布ソース1,111ファイル・31,036,368バイトは Git の内容・サイズ・mode と一致した。本家の往復互換、ネイティブ file の全挙動、物理端末、統制した性能、最終バイナリーの帰属・配布などの完成条件は継続する。

CI の次の変更では、計測済みの長い release コンパイルを既存の lifecycle／Material の2試験レーンと並行させる。試験は commit と hash を検査した不変の debug バイナリーを使い、Cargo のロックと release 出力先を共有しない。最大3レーンの全終了と失敗・panic の伝達を Rust で検査し、測定は join 後に隔離して実行する。試験・期限・最終基準は維持し、短縮効果は次の GHA の実測で確認する。ローカルは fmt・actionlint・差分検査に限定する。

2026-10-26: 宣言済みのローカル Web 素材を共通描画入口へ接続する実装を追加する。`set_browser_resources` で所有文書の素材 root を編集し、既存 Project の許可 root と私有データの境界を維持する。捕捉元・内容 revision を cache と保持 page の識別子に含め、素材の変更・宣言解除では画面外の古い context も閉じる。描画取消では最初の確認で中止した場合も browser／shader と動的 cache を回収する。両形式の共通描画14画像と MCP／CLI の8画像を検査し、元ディレクトリ削除後の bundle の読取、古い依存 revision の拒否、親子 worker、未登録 host script の拒否、寸法変更時の DOM 維持、取消後の復帰を確認する。英語 guide は1.0.24。新実装は GHA 未検証であり、ネイティブ file の全挙動、本家の往復互換と製品全体の残件は維持する。ローカルで重い build は行わない。

2026-10-26: Web 素材の管理 bundle `d0e74b7` は [GHA `40205037740`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40205037740) の先行 Linux・全3環境・監査・source を成功した。通常／core は macOS 1,113件、Linux 各1,123件で、各環境の215画像の必須確認、元素材を削除して保存済み bundle を読み直した後の CSS／画像／iframe／親子 worker と未登録 script の拒否を通した。拡張子なしファイルは通常の Chromium の MIME・実行状態・PNG と一致し、新画像の全3,072画素と配布ソース1,103ファイルを独立照合した。各環境の静的 release 画像と42回の書き出しも確認した。

Web 素材の namespace を独立コピー／再コピー、URL／metadata の部分上書き・切り離しへ結ぶ。リテラル URL と捕捉元を版付きの補助情報に保持し、同じ URL のコピー先素材へフォールバックしない。通常 metadata と実行文脈の更新、専用 envelope の検査、補助ライブラリの別名の衝突処理を script／画像と共通化する。`44ea392` の先行 Linux は core 44件・Clippy・新しい128画像を成功し、全90,112画素を独立照合した。全体の macOS は再コピー先の未正規化パスと依存の正規化済みパスを比べた新試験2件が失敗したため、Store の正規化済み保存先を比較基準に直す。CI は直前 commit の単一環境で native 試験が失敗していれば、その環境を次の先行検証に選び、明示指定と最終3環境の基準は維持する。公開描画 API の file URL、ユーザーの読取許可・宣言編集・cache／保持 page の更新、本家の往復互換、および製品全体の残件は維持する。

2026-10-25: file URL の browser 素材閉包に向け、ファイルを読まない CDP の入力経路を追加する。呼び出し側が捕捉した URL／MIME／内容だけを応答し、未登録の file 要求や書き込みメソッドは拒否する。子 target は Fetch と再帰 auto-attach の設定応答後に起動を再開し、非同期の応答を通常コマンドと混同しない。URL 別名・バイナリー・HEAD・ヘッダー注入・容量／件数・子 session の破棄を Rust 単体試験で確認し、実 Chromium では元ディレクトリ削除後の HTML・CSS import・画像・JavaScript・iframe の全画素、元の query／fragment、ホスト上の未登録 script の拒否を検査する。新しい実装は GHA 未確認で、製品側の file URL はまだ有効にしない。宣言した素材 root の捕捉・管理 bundle・コピー／再コピー・キャッシュの改版との統合、実 worker／別 process frame の検証と本家の往復互換は引き続き必要である。ローカルの重いビルドは行わない。

2026-10-25: 継承した script の入力・コード・metadata の部分上書きを、元定義への継承を保って独立コピーする実装を追加する。入力用の対応表をコード生成用の対応表から分離し、通常 metadata を継承する部分上書きには専用 envelope を使う。同じ JSON の明示更新・入力とコードの同時更新・切り離しでも古い対応を復活させない。両形式×6条件で、元ライブラリ削除・別文書への再コピー・実 worker の画素・継承元の後続編集を検査し、84画像を既存の128画像に加えて必須化する。Chromium より前に core と新しい実 worker 試験を先行し、成功後に対象3環境を検証する。ローカルは書式・静的確認のみで、新しい Rust の結果は GHA 未確認である。生成時の別スコープ、browser の素材閉包、本家との往復互換と製品全体の残件は維持する。

2026-10-25: 相対 sampler 画像の独立コピーで、symlink を通る別画像のパス投影、大文字小文字、ファイル／ディレクトリの衝突を扱う実装を追加する。通常の配置を保てる場合は保持し、純粋なパス投影だけが失敗する場合に版付き画像参照へ移す。管理 bundle のパス検査と画像形式の判定を共通化し、実ファイルの改変・すり替えは従来どおり拒否する。両形式・3条件・2テーマで、元ライブラリの削除、別文書への再コピーと初回コピー素材の削除、文字列・画像・将来の変数編集を検査する。実 WebGL の前／コピー／再コピーの36画像と、従来92画像を GHA で保存・照合する。通常／core の再現は Chromium の準備前に先行し、成功後に対象3環境へ進む。新しい Rust の結果は未確認であり、本家の往復描画互換と製品全体の残件は維持する。

2026-10-25: shader の文字列変数が絶対パスの画像を指す場合も、独立コピーで素材を取り込む実装を追加する。テーマ・別名とテキストに使う元の文字列は保ち、画像としての解決だけを生成ライブラリの版付き補助データで管理素材へ結ぶ。元ライブラリの削除、別プロジェクトへの移動、再コピー、コピー後の変数編集を通常／core の試験に加える。未知版・不正な転送先・許可領域外への symlink を拒否し、元文書・履歴を保つ。実 WebGL では相対／絶対の両方・2テーマのコピー前後 PNG を比較し、source 付き資料を保存する。ローカルは整形と静的確認に限り、新しい Rust と実画像は先行 GHA の成功後に対象3環境で検証する。本家の往復描画互換と製品全体の残件は維持する。

2026-10-25: 開発・CI・ソース配布・ライセンス収集・計測・試験用プロセスを、アプリ本体や Skia に依存しない独立した Rust の `xtask` workspace へ移した。入口は `cargo xtask …` とし、プロジェクトが管理する Python と Shell の補助処理は削除した。PTY のシナリオは Rust の統合テスト、ACP・ブラウザー・入力ワーカーの fixture は専用の Rust バイナリーへ移した。従来の小さな入力と期待結果を固定 fixture に保持し、原文ライセンス・改変拒否・パス検査を比較する。期限付きプロセス実行・取得・配布検証を共通化し、並列試験の計測は同じ実行処理で記録する。履歴中の旧 script 名と Python の試験数は当時の記録であり、現在の入口は README の Rust コマンドを参照する。補助ツール単体、対象を絞った再現試験、最終3環境の通常／core・実描画・配布証跡を GHA で検証する。製品全体の未完了条件は維持する。

移行後の source `8177c28` は [GHA `40178627693`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40178627693) で補助ツール81試験と Clippy、先行 Linux x64、対象3環境の通常／core・実描画・release・監査・配布ソース検証を成功した。追加の整理では旧 Python ファイル向けの CI 分類を削除し、独立 workspace／依存 lock と管理対象ディレクトリの補助スクリプト再導入を検査する Rust 統合試験を加えた。この整理を含む source `d0e74b7` の [GHA `40205037740`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40205037740) で、補助ツール84試験と Clippy、対象3環境・監査・配布ソース検証の成功を確認した。ローカル build tree は作っていない。Skia upstream 自身のビルドツールと実 Wasm ABI の JavaScript fixture は外部入力として保持し、その実行・期限・結果確認は Rust から管理する。

source `54ce426` の先行補助ツール83試験・Clippy・ソース検証と依存監査は成功した。全体検証では macOS の既存 hardlink 試験が、選択した `second.pen` を `first.pen` として解決する失敗を検出した。標準の canonicalize 後の不要な `F_GETPATH` 再解決を削除し、hardlink を持つ大文字小文字／Unicode 別名と、名前変更後の保存先・原本・履歴の分離を既存試験で検査する。関連変更は macOS の paths 試験を先行し、成功後に全3環境へ進む。試験の省略や期限変更は行わない。

設計書 v0.10 の全体を実装対象とする。部分的な CLI や描画の試作だけで完成とは扱わない。以下は実際の実装・試験結果で更新する。

初回リリースの完成判定は macOS Apple Silicon・Linux x64／arm64（glibc 系）の3環境に限定する。Intel Mac は対象外、Windows は既存実装・fixture を保持して後日対応とする。以下の過去の5環境検証や Windows／Intel の残件は、初回リリースの必須作業へ追加しない。機能・互換性・性能・実端末・ライセンス・配布の基準は [設計書の受け入れ条件](design.md#検証と受け入れ条件) に従う。

### 保存会話のローカル削除

2026-10-25: 削除 `4a51d23` の GHA `40139304446` は対象3環境・監査・最終 source gate を成功した。Linux 各991件・macOS 981件の通常/core、既存84画像と release の各42書き出しを照合し、配布ソース969ファイルが Git と一致した。削除の共通5試験は両構成で1回ずつ、CLI・TUI・実 PTY の UUID／確認／取消／stale 拒否／現在の下書き／端末復元を保存資料へ照合した。物理端末の証拠へは読み替えない。

次の実装では、ユーザー保持設定と全ドキュメントの期間・容量による確認付き整理を追加する。busy・下書き・保留画像・継続待ち・未知データを保護し、上限未達や不明な容量を報告する。設定・内容の変更と途中取消を再確認し、確定済み削除を含む結果を返す。元文書がなくても `cleanup --all` が有効な journal を回収する。回収待ちは内部の単一ワーカー・上限付き queue に分け、取消／deadline 待ちを止めても journal を保つ。削除確認では UUID と操作キーを固定し、50×16・長いパスの本文とエラーをスクロールする。新しい shared API／設定／CLI／権限エラー／最小端末の試験は GHA 未確認である。製品全体の未完了条件を維持する。

2026-10-25: 使用中の会話を排他ロックで保護し、共通 API・CLI と TUI に確認リビジョン付きのローカル削除を追加する。確認は全ファイルを上限付きで hash し、下書き・画像・容量と完全な UUID を示す。TUI は `d` の確認後 `y` で実行し、`n`/Esc は戻る。変更後の確認の再利用を拒否し、原子的な移動後の取消を未実行へ読み替えない。payload 外の journal で部分削除後も回収できる。文書・チェックポイント・プロジェクト素材・provider 履歴を保持する。共通の同長・同時刻の内容変更、busy／foreign／取消、部分回収・symlink と、CLI・TUI・実 PTY の確認／拒否／取消／削除／現在の下書き保持を GHA で検証する。新しい Rust の結果は未確認である。期間・容量による全体の保持設定と製品全体の未完了条件は維持する。

選択の先行実装 `b387074` は GHA `40134426833` の対象3環境と source gate を成功し、下書き・画像・native ID の保存と new→new→load、実 PTY の新規／選択／終了 UUID を独立照合した。CI 改善 `a219222` の `40137073301` も3環境・監査・source を成功し、配布ソース968ファイルを Git と照合した。重いビルドは引き続き GHA で実行し、今回の削除も先行1環境の core/default・実 UI が成功してから全3環境へ進める。

### TUI の閲覧・共通操作のキー設定

2026-10-25: ユーザーの `[tui.navigation]` に、パネル移動・ヘルプ・コマンド・終了・再取得・外部表示、パン／ズーム・対象合わせ・時刻／再生、ノード／添付一覧・検索・削除・会話スクロールを追加する。明示したリストで既定値を置換し、同じ画面とプロンプト送信／貼り付けの衝突を起動前に拒否する。入力／検索の文字、承認／checkpoint modal、固定の Ctrl+C を優先し、実行中の設定変更ではキーを変更しない。表示は実効設定から作り、ヘルプをスクロール可能にする。型・設定／schema と実 UI の操作、許可 modal の優先、実 PTY の日本語下書き・画像のパン／ズーム・ヘルプ・終了を GHA で検証する。先行は静的 TUI／user config と Clippy に絞り、対象3環境の最終検証は維持する。物理端末・IME／実クリップボード、その他の製品完成条件は維持する。


source `c14abc0` の GHA `40103285273` は3環境・監査・source をすべて成功した。Linux x64／arm64 各926件・macOS 916件の通常／core、3種類の静的 PNG の同 OS 一致と各42回の release export を照合した。所有権14ケース（macOS は13）と再開9ケースは両構成で一度ずつ成功し、実 compiler host と配布ソース949ファイルの blob・実行 bit も照合した。ただし full の実 GLSL step には codec 出力先の環境変数がなく、新しい構造体・マクロの試験は実行して成功したものの、その PNG／report が成果物へ残っていなかった。既存の参照画像や先行 Linux の64画像の独立照合とは区別し、この欠損を新しい3環境の独立画素検証済みとは扱わない。full の出力先を設定し、構造体16・マクロ64・native 取消4の計84 PNG の source・hash・寸法と取消後の一致が揃わなければ後続へ進まない guard を加える。新しい source の GHA で全対象を再照合する。

2026-10-25: 静的な文書で script 計算の入口から native layout へ取消が渡っていなかったため、共通の計算・文字計測に協調的な確認を追加する。展開・寸法・配置・visual bounds・shaping の前後・最大64字形ごとの輪郭計測と snapshot の確定に同じ要求 token を渡し、script を含む反復では既存の絶対期限も native 計算の内部へ渡す。単一の同期 Skia 呼び出しは途中強制終了せず、次の区切りで失敗を返す。途中の TextBlock／snapshot は公開しない。描画は作業面の確保前にも確認する。中断・期限の4 native unit と、公開 API・静的 runtime・サービスの取消／再計算・元ファイルと履歴・worker の非起動を検査する3試験を追加する。先行 CI はこれらを他の描画と Chromium 導入より前に実行し、後半の重複は除く。新しい Rust 試験は GHA で検証し、物理端末や全体の取消遅延・プロダクト完成の確認とは扱わない。スクリプトの独立コピーは生成時に決まる参照を含む再割り当てが必要であり、引き続き未完了とする。

source `844572b` の先行 GHA `40102336001` は build・CPU 入力／cache・Clippy を通過し、新しい64ケースの実 WebGL も成功したが、既存の2試験が失敗した。macro の配列長が展開で整数になり、短い配列を従来の link 後より早く拒否するため、macro と const 式の双方でそれぞれの失敗・復帰画素を検査する。注釈の参照画像では directive が展開で消え、直前の注釈が次の宣言へ誤って接続された。元ソースの隣接規則を保つため、directive 直前の documentation marker だけを同じ byte 長の通常コメントへ変える内部コピーを使い、元コードのコンパイル・保存内容・行とファイル番号・容量制限を維持する。参照画像の許容差は変更しない。次の先行実行ではこの2件を他の実画素より先に実行し、後半の同じ2件は除いて重複させない。未知の実行待ちによる再試行ではなく、観測した差を修正して GHA で再検証する。3環境の重い検証は失敗した focus からは起動していない。

2026-10-24: shader の入力検査を実コンパイラー設定によるマクロ展開後の宣言へ接続する。型・名前・構造体 field・配列長の生成と条件分岐を扱い、選択された annotation と型検査・画像参照元を維持する。元の GLSL と文書は変更せず、元コードのコンパイル検査と Wasm の境界も維持する。宣言取得・入力準備・描画の期限を共有し、展開ソースを PNG と同じ LRU の容量・件数制限へ含める。自作8ケース×2形式×塗り／効果×2倍率の実画像、型エラー・未知名・予約入力・元の未対応版の拒否と idle 後の非起動、共有 cache の制限と宣言の元データ保持を検査する。英語ガイドは1.0.20。ローカルで重いビルドを行わず、Rust と実 WebGL は GHA で検証する。新しい実行結果は未確認であり、全 shader 互換性と製品全体の完成条件は維持する。

2026-10-24: GLSL の構造体 uniform を型付きの末端名へ展開し、構造体配列・入れ子・field の数値配列・画像 sampler を既存の平坦な入力へ結ぶ実装を追加する。添字・型・既定値と容量を検査し、WebGL の active 名から対応するパターンへ結び、宣言型との差も拒否する。展開と読み取りには上限を保ち、共通する documentation は Arc で共有する。英語ガイド1.0.19へ修飾名を追加する。自作10ケースの本家 CLI 0.3.9 の Export はすべて宣言解析エラーであり、出力画像を正解にはしない。自作の5 CPU 試験と、2形式それぞれ8ケースの実 WebGL・全画素・元文書不変を先行 shader gate で検証する。新しい実行は GHA 未確認であり、GLSL ES 1.00 の直接の配列の配列と embedded 型定義は規格外として拒否する。macro/conditional interface・全 shader 互換性と元の公開要件は引き続き対象とする。

source `bd4cec1` の GHA `40097555337` は初回対象3環境・監査・source をすべて成功した。Linux 各918件・macOS 908件の通常／core、実 Chromium・MCP・TUI・Material と両構成 Clippy、各42回の release 書き出しを成果物へ照合した。再開・中断・旧会話移行・transcript 失敗時の保持の関連9ケースは両構成で一度ずつ成功し、所有権・失効の14ケース（macOS は13）と実 compiler host も照合した。各3枚の静的 PNG は同 OS 基準と一致し、配布ソース948ファイルの blob・実行 bit を Git と照合した。構造体 uniform の新しい実装はこの source に含まれず、先行 shader gate で確認する。全体の未完了条件は維持する。

2026-10-24: 新しいネイティブ会話の作成直後に中断し、次回の load が成功した場合でも文脈を失わないよう、引き継ぎ待ちを会話メタデータへ保存する。最初の依頼のローカル完了まで維持し、中断・エラー・transcript 完了失敗では解除しない。次の明示的な依頼で現在の文書と上限付き履歴を渡し、`local_context` と新規の native restart を区別する。private metadata は3とし、既存1／2の読み込み・下書きと ID の保持・未知版の保存拒否を検査する。実 adapter／MCP で、初期化後の取消がプロンプトを送らず、次の load 後には文脈を渡して一度だけ編集する再現を用意する。新しい Rust 試験は GHA で検証し、provider 実接続・会話管理全体の完了とは扱わない。

2026-10-24: 会話の再開非対応時に停止していた経路へ、新しいネイティブ会話への切り替えを追加する。ローカル会話は保持し、現在の文書リビジョンと最大4ターンの説明的な履歴抜粋を渡す。読み取りは4096項目・各 transcript の先頭／末尾64 KiB・各依頼1024文字で区切り、失敗・中断・欠損を成功へ読み替えない。CLI／TUI と transcript に切り替えを明示し、過去の依頼を再送しない。広告済みの load の失敗は自動で隠さず、明示的な `--restart-session` による継続を用意する。実 MCP の編集回数・外部変更の保持・下書き・画像保持と既存履歴の不変を検査し、Skia 不要の先行 gate へ再現を追加する。ローカルは軽量検査だけを行い、新しい Rust 試験は GHA 未確認。実 provider・会話管理全体の完成とは扱わない。

2026-10-24: Unix の画像添付・ガイド取得の接続に、識別子付き owner file と close-on-exec の排他ロックを追加する。MCP は各要求の非ブロッキング共有 probe で生存を検査し、親の強制終了後は既存接続も失効する。 所有接続のすべての tool call に dispatch 前の検査を設け、文書の読み取り・編集も拒否する。すでに進行中の保存は元の原子的な保存規則に従う。会話の排他ロック取得後、同じ文書・会話に束縛した失効済み manifest／owner のみを回収し、画像・transcript・usage journal・プロジェクト素材を保持する。cleanup が共有 probe を持っても権限を復活させず、symlink・FIFO・inode の置換・不明な記録を扱う。実 CLI の強制終了と再開、接続済みの実 MCP の失効と journal 不変を描画なしで先行し、添付・MCP の利用側も対象へ含める。実装・試験は準備段階であり、GHA 結果は未確認。Windows の既存動作は保持し、保持方針・会話管理全体の完成とは扱わない。


source `4f01ce5` の GHA `40094747115` は初回対象3環境・監査・source をすべて成功した。Linux 各902件・macOS 892件の通常／core、実 Chromium・MCP・TUI・Material と両構成 Clippy、各42回の release 書き出しを成果物へ照合した。接続失効・回収の関連14ケース（macOS は13）は両構成で一度ずつ成功し、実 compiler host も対象3種類と一致した。各3枚の静的 PNG は同 OS 基準と一致し、配布ソース947ファイルの blob・実行 bit を Git と照合した。focus 後の native 起動待ちは1〜4秒、core build／試験は26／31・79／33・70／44秒、通常 build／試験は76／191・270／210・210／219秒、release 工程は150・248・369秒だった（Linux x64／arm64／macOS）。再開時の新しい文脈引き継ぎはこの source に含まれず、後続で検証する。全体の未完了条件は維持する。

source `e0e1a2b` の GHA `40091647214` は Linux の先行72試験を成功し、guardian を含む回収8ケースを成果物と照合した。通常／core の試験本体も macOS 880件・Linux x64 890件が成功したが、共通化後に残った `ExitStatus` の未使用 import により、両構成 Clippy が失敗した。未使用 import を除き、描画なしの先行検証でも対象 lib／integration の Clippy を test executable のビルドより前に実行する。警告を許可する変更は行わない。実試験の失敗ではないが、当該 source の全体の成功・配布ソースの完成とは扱わない。修正版 `00e21c7` の GHA `40092658303` は3環境・監査・source をすべて成功した。Linux 各890・macOS 880件の通常／core、実 Chromium・MCP・TUI・Material、両構成 Clippy と各42回の release 書き出しを成果物と照合した。追加した guardian 回収は Linux 8ケース・macOS 7ケースが各構成で一度ずつ成功し、実際の compiler host も対象3種類と一致した。各3枚の静的 PNG は同 OS 基準と一致し、配布ソース946ファイルを Git の blob・実行 bit へ照合した。接続失効の owner file はこの source に含まれず後続で検証する。


2026-10-24: Unix エージェントにも専用 guardian を追加し、ACP とは独立した匿名 socket の EOF で親の終了を検知する。Chromium で検証した PID を再利用前に固定する group 回収を共通化し、adapter の close・取消・runtime drop・親 CLI の強制終了を扱う。Linux は subreaper で独立 session の子・孫も回収する。起動 status・stderr の上限付き転送・subscription の環境除去を維持し、所有 descriptor は adapter 起動前に close-on-exec にする。runtime 破棄時も実際の child を保持する独立 waiter で guardian 自体を回収し、試験では guardian・adapter・子・孫の4 PID を確認する。実プロセスの回収・別会話の分離・詰まった出力・所有者／adapter 欠損を試験に追加し、Skia 不要の force-kill 再現を先行する。Windows 実装は変更しない。新しい Rust の実行と両構成の確認は GHA で行い、macOS の group を離脱する子孫や grant の crash 後回収等は完了としない。


2026-10-24: 本家 CLI 0.3.9 の自作22ケースを調べ、未指定入力の型別既定値、null によるリセット、空の ref、保存済みの旧入力の無視へ合わせる。旧入力に残った `$missing` も解決せず、元データは保存する。512×528の参照 PNG の全1,081,344 RGBA成分を独立に確認した。新たな入れ子のコード依存を取得した後の入力解決と、null で戻した参照の既定ライブラリも検査する。明示値の型と enum の検査は維持し、実行メタデータの engine と英語ガイド1.0.18を更新する。GHA の core worker を先行し、2形式の全画素比較・元データ／履歴の保持と既存の乱数 corpus を続ける。project 変更では query の利用側も対象に含める。実装と参照データを準備し、ローカル Rust／Skia build は行わない。

source `eac581a` の先行 GHA `40088087463` は実 worker の22ケースと、入れ子・既定参照ライブラリ・既存乱数の試験を成功したが、サービスの構造検査が旧入力の `$missing` を先に拒否した。ノード／インスタンス／descendant override の script inputs はヘッダー取得後の検査へ委ね、通常の塗りや `inputs` という名前の shader uniform の変数検査は維持する。新しい構造検査の回帰と、宣言済みの未定義変数が worker 起動前に失敗する試験を追加する。core の project と worker を Skia 前に先行し、operations の変更も既存 CLI 試験へ広げる。失敗した先行 gate の段階で全環境は未起動である。

source `dc84dc5` の先行 `40089004985` は core project／worker 各15件と、その後の171件が成功した。入力22ケースと既存乱数4ケースは、両形式の保存 PNG を独立に復号して本家参照と全 RGBA 成分一致を確認した。ただし変更なしの Wasm job が skipped の場合に後続の通常3環境・監査・source まで skipped となり、workflow 全体が success と表示された。これを全体合格として扱わず、native／audit に直接の依存成功条件を明示する。source job は必要な native／audit が両方 success でなければ、生成・アップロード前に失敗させる。スキップ・失敗・取消・欠損した結果を拒否する検査を追加した。修正した source `89477a8` の GHA `40089734168` は、任意の Wasm 検証を省略しても初回対象3環境・監査・source がすべて成功し、source の必須 gate も実際に実行された。Linux 各874・macOS 866件の通常／core、実 Chromium・MCP・TUI・Material・両構成 Clippy と各42回の release 書き出しを照合した。各環境で入力22ケースと乱数4ケースの2形式の画像を独立に復号して本家参照との全 RGBA 成分一致を確認し、配布ソース943ファイルは Git の blob と実行 bit へ照合した。Unix adapter の新しい guardian はこの source に含まれず、後続で検証する。

source `e6a1bfb` の GHA `40085384401` は初回対象3環境・監査・配布ソースをすべて成功した。Linux 各866・macOS 858件の通常／core 試験、実 Chromium・MCP・TUI・両構成 Clippy・Material と各42回の release 書き出しを確認した。各環境の2形式について、乱数4ケースの各37サンプルと、shader の座標10ケース・注釈20ケースの1倍／2倍・既定値14ケースの保存画像を独立に復号し、本家参照と全 RGBA 成分が一致した。release の3 PNG は各同 OS の基準画像と一致し、配布ソース915ファイルは Git の blob・実行 bit と照合した。focus 後の native 起動待ちは2〜6秒、通常構成の build は99／283／125秒、試験は281／207／200秒（Linux x64／arm64／macOS）、release build と smoke は214／241／256秒だった。全体の初回完成を示すものではなく、新しい22入力ケースと元の互換・provider・性能・実端末・配布条件は引き続き検証する。

2026-10-24: 本家 CLI 0.3.9 の公開 Export で、自作 JS の2.11・2.19・2.21ヘッダーと、寸法・入力を変えた再評価を確認した。4ケースの各37個の乱数は Mulberry32 seed 0 と一致し、1328×192の参照画像の全1,019,904 RGBA成分を独立に照合した。現在の xorshift32 とは異なるため、原作者の CC0 表示を保持して置き換え、JS コンテキストごとの初期化と `Math.imul` の変更への耐性を維持する。2.11ヘッダーの受け入れは文書コーデックとは分離し、未知版の拒否は維持する。英語ガイドは1.0.17。実 worker と2版の全画素比較は GHA で検証し、先に Skia 不要の worker 試験を実行する。レイアウト変更には graphics／render の利用側も先行対象へ含める。ローカル native build は行わない。

source `a3523e1` の GHA `40084017580` は Wasm 再ビルド一致・17 ABI試験、Linux の17 CPU・8実 WebGL を成功した。保存した10ケースの2倍、注釈20ケースの1倍／2倍、既定値14ケースの画像を独立に復号し、両形式とも本家参照と全 RGBA 成分一致を確認した。元ソースの行番号と effect／z/w 試験も通過した。先行 job の cache restore は20秒、build は101秒、Clippy は13秒、実 WebGL は39秒だった。終了済み log は旧 v2 key の復元に加え、Skia binding の再コンパイルも示したため、全依存が無再構築だったとは扱わない。通常 job は起動期限 fixture が source を省略していたため、新しい事前検査で `SHADER_COMPILE_FAILED` となり停止した。描画コードの入力検査は維持し、fixture に有効な GLSL を与え、3種類の期限・3秒以内の復帰・子の回収を引き続き検査する。欠損・型違い・1 MiB超の source は起動前に拒否する回帰も追加し、先行 dynamic gate では Chromium 取得前に実行する。全3環境と配布ソースの成功はまだ示していない。

2026-10-24: source `e60a0da` の先行 GHA `40083564201` で17件の CPU と6件の実 WebGL が成功し、注釈20ケースの1倍／2倍・既定値14ケース・元の行番号を保つ4ソースも通過した。残り2試験は、CDP が返す数値 `32` と、試験側の JSON `32.0` の表現差で停止した。論理寸法を `as_f64` で数値として比較するよう直し、本家10ケースの全画素比較と effect／z/w の条件は維持する。通常3環境は未起動である。同じ先行実行は監査済み旧 v2 key を実際に18秒で復元し、app の再ビルドは約50秒だった。先行する `e9c90da` は新しい属性 key の miss で Skia binding を再構築していたことを終了済み log から確認した。

2026-10-24: 行番号修正の source `5260f21`／GHA `40083330165` は実 Wasm 17ケース・原表示収集・wasm32 の標準 library 収集が成功した。古い同梱 module を拒否する最終検査で停止し、native job は開始していない。生成 artifact を ZIP digest・Git のソース54ファイル・69 package・標準 library 54点・memory import の64 MiB上限へ独立に照合し、481,016バイト／SHA-256 `3d762d31fd40930cd980abeaa78573e9ddc3a695851154b11d6eff193c1ba1f4` の新 module と原表示へ置き換える。次の GHA で再現ビルド一致と実描画を検証し、全3環境の最終基準は維持する。

2026-10-24: 接続後の追加確認で、glsl-lang-pp の `__LINE__` が通常行・明示的 version・`#line` のすべてで1小さいことを、GHA 生成済み module の軽量実行で再現した。内部 resolver の0始まりは保ち、builtin だけを1始まりへ変換し、`#line 0` も維持する。通常・version・複数行 comment・明示的0・file／version の Wasm 再現と実 WebGL の色による試験を追加する。component 変更後は GHA で新 module を生成して取り込み、古い生成物では native gate を開始しない。同時に `.gitattributes` の Wasm／新しい原表示向け追加だけで native cache key が変わっていたことを確認した。依存入力67パスの属性・Cargo manifest／lock・native source・build policy が不変であることを Git から照合し、新しい primary key と全 app 出力の除去を維持したまま、同じ compiler・image・既定 flags に限り直前の v2 依存 key を復元する。

2026-10-24: GHA で生成した481,013バイトの GLSL 前処理 Wasm を共通描画へ接続する。元のソースを WebGL で検証した後、実コンパイラーの GL_ macro を uniform の反映から取得し、専用64 MiB memory 内で macro を展開する。塗り・罫線・effect の論理寸法を raster 寸法から分け、`@resolution` と `gl_FragCoord.xy` を同じ論理単位へ揃え、z/w・保存済みソースと hash は保持する。取得結果と cache に module hash を含める。GHA artifact `12554629641` の ZIP digest・ソース54ファイルと Git・実 Wasm 16ケース・memory import 上限・69 package の原表示・wasm32 標準 library 54点を独立に検査した。同梱 module のソース／license との一致と再ビルドのバイト一致を gate とし、原表示と生成物を release evidence へ保持する。新しい CPU 入力試験、本家10ケースの2倍比較、注釈20ケースの1倍／2倍、実コンパイラーの macro・shader effect の検証を追加する。英語ガイドは1.0.16。ローカルは軽量検査だけで、Rust・実描画は GHA で検証する。全互換や全体完成の主張ではない。

2026-10-24: 拡大時の GLSL 座標を直すため、独立した Wasm 前処理 component を追加する。proper preprocessing 後の genuine builtin だけを補正し、マクロの名前・引数・条件分岐・override／undef と z/w を保持する設計とする。upstream glsl-lang-pp 0.8.1 と関連2 crate の BSD-3-Clause 原文を公開 archive の VCS revision と照合し、元の license を同梱した。Node による実 Wasm ABI・16ケース・64 MiB memory／5秒の増幅再現、全69 package（registry 67・local 2）と wasm32 の標準 library の原表示収集を GHA へ組み込み、対象 component が変わった場合だけ native 検証より先行させる。本体の Cargo.lock と Skia cache は変更しない。ローカルは版固定と整形・Python 146件・Node syntax・actionlint までで、Rust／Wasm はビルドしない。生成した小さい index は削除した。source `4541485` の GHA `40080130393` で Wasm のコンパイルと Clippy は成功したが、同じ macro の入れ子では想定した増幅負荷を作れず、資源試験が失敗した。異なる32段の object macro の倍増へ置き換え、小さい同名 function の入れ子も独立に検査する。新しい gate は重い native 検証を開始する前に停止した。release profile を明示した own-package の clean と除去確認、後段試験が失敗した場合の依存だけの保存を加える。source `546bcd3` の GHA `40080345332` は小さい `F(F(x))` の引数内を展開しきらない upstream の不具合を検出した。BSD の原 license と archive hash・revision・差分説明を保持して glsl-lang-pp を同梱し、使用する通常引数だけを置換前に展開する。未使用引数と連結に用いる引数は元のまま保持する。入れ子と未使用引数の再現を維持し、修正版を GHA で検証する。実描画への接続は未完了である。 また、自作10ケースを本家 CLI 0.3.9／形式2.19で800×40へ公開 Export し、固定寸法・明示／自動 resolution・object／function macro・同名引数・override／undef の128,000 RGBA成分を論理座標の式と照合して全一致を確認した。参照 PNG は `63ea422f17aed304d372d9b706608c47d1a33653f8a1b169d3de41bdb0cec6d5` であり、ベンダーのプログラム・文章・認証情報は含まない。

2026-10-24: シェーダー注釈を宣言直前の最も近い JSDoc に限定し、別 comment・コード・プリプロセッサ行で破棄する。宣言後の注釈は次の宣言へ適用し、宣言内では適用しない。本家 CLI 0.3.9／形式2.19の公開 Export で自作20ケースを確認し、全 panel の40,960画素・163,840 RGBA成分を検査した。参照 PNG の SHA-256 は `1e07851442f0bd08cdbeec7ea45d05cf32f13bb59ccd176699695b1dd0b92ad5`。別の公開操作で `@time`／`@default` の併用拒否も確認した。元の GLSL・文書・PNG を fixture に追加し、CPU 入力試験と2版の実 WebGL 画像比較を加える。比較は拡大座標の課題と分けて1倍で行い、2倍の native 参照の対応位置を照合する。英語ガイドは1.0.15。ローカル Python 145件・fmt・actionlint は成功し、Rust／実描画は GHA で検証する。拡大出力の logical 座標・resolution とその他の全互換条件は未完了のまま維持する。

2026-10-24: 四角形以外の辺別罫線は、全周を `top` の幅で描く本家 CLI 0.3.9／形式2.19の挙動に合わせる。自作の33ケース、3配置×文字・開閉 path 等の120ケース、省略値36ケースを公開 Export で調べ、`top` の省略は0になることも確認した。Rust の共通線幅解決でレイアウト・文字輪郭・描画を揃え、保存済みの各辺の値は変更しない。33ケースの参照 PNG を2版で比較する試験、75ケースの scalar 等価性、bounds・不正値の回帰を追加する。英語ガイドは1.0.14。CI 設定を同時変更しても graphics/layout を先行し、graphics の重複 TUI unit を除く。ローカル native build は行わず、新しい Rust と画像比較の GHA 検証は未完了である。

source `cb5ba45` の先行104試験と、保存した2版各33ケースの独立画像検査は成功した。通常 CI では、旧 `tests/render.rs` が非四角形の罫線を未対応として拒否する期待を残していたため失敗した。この期待を実画像・top の幅・出力範囲の検証へ更新し、保存サイズ上限と未設定 runtime の拒否は維持する。`tests/render.rs` のみの変更は実 Chromium を要する変更と区別し、CPU の graphics gate で先に検証する。対象3環境の最終基準は維持し、修正版を GHA で確認する。 source `1b74a35` の通常 GHA `40076690568` は対象3環境・監査・ソース配布物がすべて成功した。Linux 各858件・macOS 850件の通常／描画なし試験、実 Chromium・MCP・TUI・両構成 Clippy と各42回の release 書き出しを成果物から独立に確認し、33ケースの2版比較も成功した。release PNG は各同 OS の基準と一致し、配布ソースは Git の blob・実行 bit と照合した。全要件の完成を示すものではない。

2026-10-24: uniform 宣言前の documentation comment にある `@default` を読み、数値・色・カンマ区切り成分・grouped 宣言へ適用する。保存済みの 0／false を含む指定値を優先し、通常 comment と宣言後の comment は既定値にしない。配列の既定値は先頭要素だけを初期化し、残る active 要素を0にする。短い明示配列の拒否は維持する。本家 CLI 0.3.9 の公開 Export で自作の14ケースを560×40へ出力し、全画素と診断なしを確認した。vendor のプログラム・文章・スキル・認証情報は取り込まず、機能上の注釈記法だけを確認した。英語ガイドは1.0.13。専用の shader focus は Chromium の取得前に CPU 検証を行い、成功後に実 WebGL と native 画像比較を実行する。source `1f6166c` の GHA `40074036061` は対象3環境・監査・配布ソースをすべて成功した。保存画像を独立に復号し、全環境の literal／constant の2方式で14ケースの全 RGBA 画素が本家参照と一致した。Linux は各853・macOS は845件の通常／描画なし試験、実 Chromium・MCP・TUI、各42回の release 書き出しも通過し、ソース653ファイルを Git の blob と実行 bit へ照合した。これは対象 corpus の証拠であり、プロダクト全体の完成ではない。

2026-10-24: macOS／Linux の Chromium を専用 guardian から起動する処理を追加する。所有者だけが書き込み側を保持する pipe の EOF で、所有者の強制終了時にもブラウザーの group を終了する。group leader の終了は `waitid(WNOWAIT)` で確認し、共有する group ID を無効化してから wait するため、回収後の PID を signal しない。Linux の guardian だけを subreaper とし、別 session の子孫も採用・停止・wait する。稼働中の孤立した子孫もその終了ごとに回収し、定期 polling は使わない。通常の終了待ちは3秒で区切り、OS の終了待ちが遅い場合も guardian と waiter の所有を保つ。 一時プロファイルは guardian が自分で作成・所有し、ブラウザーの終了後に削除する。起動通知は4 KiB 以下の pipe メッセージで返し、ホストに終了処理が走らない場合も、プロファイルと status file が残らない構成にする。既存の取消・親の強制終了・実 Chromium 試験に profile の削除と別所有者の保持を加え、途中の status・容量超過・不正 JSON・早期 EOF と取消も先行試験で扱う。このディスク回収は source `12b9afe` の GHA `40070808429` で対象3環境すべてが成功し、両構成・実 Chromium・profile の削除／所有者分離と OS 子孫の回収、配布ソース638ファイルの Git 照合を確認した。実プロセスの取消・起動期限・親の強制終了・所有者分離・root の先行終了・継承した SIGCHLD ignore・所有者不在・起動失敗を先行試験へ加えた。実 Chromium は既存の画像検証に加え、CDP と OS の親子関係から観測した子孫の終了と別所有者の CDP 継続を検査する。OS の一覧取得は3秒・1 MiB で区切り、列挙した PID には signal を送らず、生存と終了だけを確認する。macOS の managed runtime の group 条件も検証し、GHA の対象3環境で成功するまでは保証済みとしない。ローカルでは小さい子プロセスで macOS の waitid ABI を確認しただけで、Rust／Chromium はビルドしない。

2026-10-24: CDP のソケット読み書きごとに、現在の操作の絶対期限と取消を確認する経路を追加する。従来の個別 read timeout だけでは、HTTP ヘッダーや WebSocket frame が少しずつ届く間、ライブラリ内部の呼び出しが戻らず外側の期限確認へ進めなかった。TCP 接続にも残り期限を適用し、各 command で取得時の期限を更新する。部分ヘッダー・部分 frame の継続受信、送信先の読み取り停止、接続後に期限が切れた場合の次 command を実ソケットで検査し、動的コンテンツの focused gate でも CDP unit を先行させる。初回対象3環境の GHA で検証し、ローカル native build は行わない。Unix の browser 子孫回収など、他の未完了要件も維持する。

2026-10-24: ユーザーの指定により、初回リリースの対象を macOS Apple Silicon・Linux x64／arm64 の3環境へ変更する。Intel Mac はサポート対象外、Windows は後日対応とし、既存の Windows 実装・fixture を保持する。設計・README・通常／手動 CI と完成条件をこの範囲へ揃え、追加の Windows 対応は延期する。以下の5環境・Windows／Intel の記述は変更前の実装・検証記録であり、現在の必須条件ではない。機能・互換性・性能・実接続・端末・ライセンス・配布の未完了条件は対象3環境について維持する。旧5環境の GHA `40063955857` は範囲変更に伴い停止要求済みで、新しい3環境構成で検証する。

2026-10-24: Windows の Chromium に所有者単位の kill-on-close Job Object を追加する。非継承の所有 handle はホストだけが保持し、専用 CLI worker は Job への参加と一時 handle の解放を済ませてからブラウザーを起動する。起動状態は private profile 内へ小さい JSON を atomic に公開し、読み取り・待機は既存の取消と deadline に従う。診断 PID は worker でなく実 Chromium を保持する。Skia 不要の先行試験で取消・起動期限・親の強制終了・別所有者の分離・所有者不在／実行 file 欠損を扱い、通常の実 Chromium 工程にも終了と強制終了の子孫回収試験を加える。ローカル native build は行わず、Windows の対象試験から GHA で検証する。Unix の異常終了対策、残る共有 runtime・性能・互換性・配布要件の完了は示さない。

2026-10-24: TUI の `/play`／Preview の Space を browser 単独文書にも拡張する。各自動 frame は保持済み DOM の画像だけを取得し、同じ layout と静的 shader の acquisition は再利用する。既存の1〜10 fps・描画中／未消費結果の間引き・モーダル中の延期・pause／error／静的文書での停止を共通利用し、文書の script を時計で再実行しない。実 renderer の DOM・snapshot・静的 shader ID・PNG と、browser-only の実 PTY による赤→緑→青・停止・下書き／文書保持を先行 retained-page 試験へ組み込む。ローカルでは軽量検査を行い、Rust と実 Chromium は Linux の対象試験成功後に既存の全5環境 CI で検証する。全体の未完了要件は維持する。

2026-10-24: CI を変更箇所の代表1環境／Windows 固有変更なら Windows の focused reproduction が成功してから、従来の5環境・両 feature 構成・Clippy・Material・release/source 検証へ進む構成に変更する。既知の embedded Conversation の helper/MCP 経路を Skia 不要の実プロセス試験で先に検査し、応答は10秒で区切る。Chromium opt-in は通常 native job のビルド済み出力を再利用し、60秒 idle・実 PTY・画素・sandbox 条件は維持する。同じ branch／対象範囲の旧実行は新しい commit で置き換える。待機／ビルド／試験は既存 GHA の job/step 時刻とログで確認する。過去の Linux/macOS 成功実行は待機220〜295秒、再ビルド115〜142秒、idle は各62秒、TUI は98〜99秒だった。Windows の旧失敗は再現試験に到達する前に約6分、単体試験の無期限待ちに約52分を費やした。同じ compiler・runner image・依存入力の新キャッシュが存在する4組について旧版だけを削除し、5,110,872,350 bytes を解放した。新構成の実行結果と速度は GHA で確認する。

2026-10-24: 状態付き page の source `7c09434` は Linux x64 `40053898552`／macOS arm64 `40053900227` の両構成 Clippy・単体・既存 page/shader 試験を通過し、新しい retained DOM 試験の viewport 比較で失敗した。実際の JSON は `40.0` で、追加 assertion が整数 `40` と比較していたため、`as_f64() == Some(40.0)` で数値と型を検査する形に修正した。上限／分離／LRU の実 context 試験と retained 取消・復帰は同じ実行で成功していたが、途中で止まった URL 復元・削除再挿入などの後半と実 PTY の成功はまだ示していない。4段階の小さい retained PNG/report を GHA に保存する経路も追加し、後続で実画素と state ID を独立に照合する。

同日: 旧 Windows の default agent 27件・lifetime 6件の成功と TUI unit 失敗を partial artifact で照合した後、同じ Conversation/Agent/factory code を使う旧専用 `40052567048` と、Windows test だけが残っていた通常 `40050016602` に cancel を要求した。通常の他4 native job と監査はすでに成功していた。実際に失敗した source の対象 code と byte 比較し、起動先修正を push・Windows 後続を投入してから切り替えた。観測の遅さだけを根拠にした再実行ではない。修正後の全体の結果は引き続き確認する。

2026-10-24: Windows `40049415871` は job 上限の1時間で cancelled となり、終了後の log を取得できた。描画に入る前の TUI lib unit で画像入力・custom key・permission の3件が FAILED となり、completion-order fixture が50分近く待ち続けていた。TUI の Conversation factory は libtest executable を Windows supervisor として使っていた。`Conversation::open_with_executable` から実 lazypencil helper を MCP と supervisor に結び付け、TUI factory は Cargo の regular binary を指定するよう修正した。completion fixture の待ちを10秒で区切り、失敗を明示する。構成だけの試験で2経路が指定した同じ helper を使い、起動しないことも確認する。実 Windows の修正結果は次の GHA で確認し、Job guard や既存の画像／permission 条件は維持する。 旧 run の partial artifact `12544825807` の digest・size・ログも検査し、default agent 27件（Windows lifetime 6件を含む）と両構成 Clippy の成功を確認した。全体は cancelled であり、TUI unit と nested Chromium の成功とは扱わない。実 job log の native v2 Windows cache restore も確認した。同じ factory／supervision code は先行通常 CI `c708b89` と byte 一致しており、残る Windows test 待ちの切り替え根拠とする。

2026-10-24: TUI の文書・ノードごとに DOM を保持する page 取得を追加した。CLI/MCP と shader は fresh context を維持し、TUI だけ opt-in とする。保持は8 context の LRU 上限で、URL/device 変更時は作り直し、寸法・scroll・scale・zoom は同じ状態に適用する。snapshot 変更時に raster cache を失効させ、削除／無効ノードを閉じ、viewport 外の有効ノードは維持する。browser 方針変更・root の結び直し・idle・失敗で context が失われる場合は新しい状態 ID となる。context 破棄の失敗を正常完了とせず、Runtime は browser を閉じる。複数 session の main-frame lifecycle を独立に追跡し、文書の遷移も loader/frame ID で判定する。page_state metadata は取得時を表し、TUI summary にも保持する。実 context の上限／LRU／分離、同じ URL の別ノード、DOM の赤→緑、寸法変更、URL の復元・削除再挿入、policy／idle／取消後の復帰、実 PTY の赤→マゼンタ・待機後の赤への復帰を追加検査する。英語 guide は1.0.11。ローカルの128 Python 試験・構文・整形・差分を確認し、Rust と Chromium は GHA で検証する。新機能はまだ検証中で、周期取得と他の未完成要件は維持する。

2026-10-24: 停止表示修正 source `5fd4ddc` は Linux x64 `40052439098`（262試験）と macOS arm64 `40052437100`（260試験）が成功した。artifact `12544386625`／`12544585448` の digest・必須成功・metadata・実画像を照合し、同じ OS の以前の example PNG と一致した。手動時刻 source `9129f17` の Intel Mac `40051535783`（255試験）も、artifact `12543548154` を同様に照合した。いずれも新しい状態付き page 取得の追加前の証拠である。

2026-10-24: 再生・LRU の初回 source `1936598` は macOS arm64 `40052100177`（260試験）と Linux x64 `40052102399`（262試験）が両構成 Clippy と実 PTY を含めて成功した。artifact `12543762414`／`12544141047` の ZIP digest・全成功ログ・PNG と report を検査した。5件の時計／背圧／状態／LRU の必須成功、実 PTY の2fps clock・青→黄の画素・日本語 draft・Space 停止・待機回収を確認し、shader acquisition の `time_dependent` が時刻付き example では true、固定色の supervised MCP では false であることも照合した。通常 example の PNG はそれぞれの以前の画像とバイト一致した。停止表示の後続 source `5fd4ddc` は Linux x64 `40052439098`／macOS arm64 `40052437100` で検証中である。現行 app code はそれ以降の CI 分割 commit と同一である。Windows の新しい後続は `40052567048`。物理端末、性能保証、全 OS とプロダクト全体の完了は示していない。

2026-10-24: Windows の先行専用 job `40049415871` は default agent gate を通過した後、複合 runtime step で長時間実行中であり、完了前の log API は404を返した。browser 接続も環境設定の読み込みで失敗し、共有設定は変更していない。現行 job は取り消さず、後続の専用 workflow では単体（10分）、設定・script（10分）、page／shader／import の実画素（20分）を別 step に分けた。実行する command・試験・pipefail・既存 log 名と `always()` での保存は維持する。actionlint が成功し、native dependency input tree が `c95e917…` のままであることも確認した。これは後続で停止箇所と途中 log を取得するための変更であり、Windows の長時間化の原因や解消を示すものではない。

2026-10-24: 手動時刻 source `9129f17` の Linux arm64 `40051538036` も257試験・両構成 Clippy・実 PTY を成功した。artifact `12543678210` の digest・各成功ログ・画像と report を照合し、時刻操作と supervised MCP の実画素、以前の example PNG とのバイト一致を確認した。新しい再生と LRU の追加前の結果である。再生停止時に未完了フレームがあれば、status は表示中画像の時刻を保持して完了待ちを示し、該当フレームの公開後にだけ時刻を更新する修正と、その中間状態の worker／app 検査も追加した。新しい再生の GHA は実行中で、Rust・実 PTY の結果はまだ未確定である。

2026-10-24: TUI の上限付き shader 再生を追加した。`/play [fps]`（既定5、整数1〜10）と Preview の Space で開始／停止し、単調時計から評価時刻を求める。描画中・待機要求・未表示結果があればフレームを捨て、既存描画を timer で取消さない。静的な画像では開始せず、エラー・静的結果・seek で停止する。停止後は実行中1フレームだけ完了でき、再生 timer は待機しない。入力と modal が優先し、overlay／添付画像中は時計を進めつつ描画を保留する。shader の解析済み `@time` から acquisition の `time_dependent` を記録する。cache の全破棄で page が巻き込まれるのを避けるため、容量・件数を保つ LRU とし、cache hit は idle deadline を延ばさない。時計・背圧・状態遷移・100フレームの cache churn、実 PTY の色と時計・日本語 draft・fps 上限・停止・待機回収を追加検査する。ローカル Python 128試験・構文・整形・差分を確認し、Rust／実 PTY は GHA で検証する。新しい結果は未確認であり、状態を保つ browser 周期取得などの必須スコープは残る。

2026-10-24: source `c95d7ee` の時刻操作は macOS arm64 `40050825072`（255試験）と Linux x64 `40050828187`（257試験）が両構成の全 target Clippy・実 PTY を含めて成功した。artifact `12543222375`／`12543099553` の ZIP digest・全成功ログ・実画像と report を照合した。追加した worker の同じ layout／時刻 poll 抑止、command の不正値・入力優先、実 PTY の青→黄→青・同じ process の再利用・停止中2 poll の再描画抑止・元の待機回収と refresh が成功した。supervised MCP 300×100の実画素も再検査し、通常の1000×640 example はそれぞれの以前の画像とバイト一致した。新たな目視や Windows の nested runtime の成功へは読み替えない。完全に同じ展開済み資料を確認した一時 ZIP 34個（4,998,200バイト）を削除し、資料本体は保持した。

2026-10-24: TUI の手動 shader 時刻を追加した。`/time <seconds>` と Preview の `[`・`]`（0.1秒刻み、後退は0で止まる）から、既定0秒の評価時刻を変更し、完了した画像の時刻を status に表示する。有限の非負値だけを受け付け、文書・会話には保存しない。時刻だけの変更では layout を再利用し、同じ時刻の依存 poll では画像を再 encode しない。入力／help の優先と無効な値の保持、worker の layout 再利用・同時刻 poll 抑止を追加試験し、実 PTY では `@time` による青→黄→青の画素、同じ子 process の再利用、2回の停止中 poll の再描画抑止を検査する。ローカルの整形・Python 構文・全128 Python 試験は成功した。Rust と実 PTY の GHA 結果は未確認で、再生・周期取得は残る。

2026-10-24: source `a05e521` の Linux x64 専用 Chromium `40049820118` が255試験と両構成の全 target Clippy を成功した。artifact `12543206913` の digest・各成功ログ・PNG/report を検査し、supervised adapter →実 MCP →QuickJS／page／shader の300×100画像の緑・青・赤の実画素、診断0件と2取得を確認した。新しい TUI 時刻操作の追加前の証拠である。実 job log には監査済みの旧 native v2 key `native-v2-ubuntu-24.04-20261012.334.1-989dab0cb93154d57dfa89ebdd8b9252928ed8f08ed85c345b20f90e68efa63f` からの restore 成功が記録されており、cache step の終了だけからの推測ではない。Windows の nested runtime は後続 `40049818283` で確認する。

2026-10-24: source `1c26faf` の通常 CI `40047491096` が監査・native 5環境・配布ソース検証をすべて成功した。5 artifact の ZIP digest・各3 PNG/report の寸法と hash・診断0件・各42回の benchmark export・material 試験ログを検査した。通常／描画なしの合計は Linux x64・arm64 各808、macOS arm64・Intel 各804、Windows 730件だった。全15 PNG は同じ OS の以前の画像とバイト一致した。Windows job 管理の追加前の source であり、最新 main の成功や新たな目視を示すものではない。ソース artifact は job 成功・upload metadata の確認にとどめ、archive は取得していない。各環境・scene の20反復から中央値・nearest-rank p95・最大 peak RSS を独立に再計算し、[同一 source の性能観測](performance.md#same-source-hosted-release-observations-2026-10-24) に記録した。

2026-10-24: core の macOS arm64 `40049231515`（source `a8ef359`）も302試験と全 target Clippy が成功した。artifact `12542917201` の digest・source-bound report・Unix group cleanup の実成功ログを検査した。Windows 277／Linux 304と合わせ、共通の process refactor を3 OS の描画なし構成で確認した。通常の Windows agent gate は `40049415871`、新しい3実行系の supervised MCP 画素試験を含む後続は Windows `40049818283`／Linux x64 `40049820118` で進める。ローカルの Rust build/test は行っていない。

2026-10-24: job 管理の内側にある実行系の互換性を確認するため、疑似 ACP adapter →実 MCP →QuickJS／Chromium page／WebGL shader の描画試験を追加した。300×100の PNG の緑・青・赤の実画素、2つの acquisition、診断0件、元文書の無変更を検査する。既定の agent suite では opt-in とし、固定 Chromium を導入した専用 GHA の後段で明示して実行し、PNG・report・成功ログを保存する。これにより、Windows job の nested runtime と通常の動的描画を同じ経路で検証する。新試験の実行結果はまだ未確認である。

2026-10-24: Windows job source `4a8d082` の専用 GHA は Windows `40048681707` の277試験と Linux x64 `40048684209` の304試験、両方の core 全 target Clippy が成功した。artifact `12542547386`／`12542467342` の ZIP digest、全試験ログと source-bound report を検査した。Windows 6件は通常 close、別セッションの保持、非協調 cancel、host/runtime drop、親の強制終了、所有者がない worker、起動エラーを実 process/kernel handle で確認する。detached の子・孫も含む。これは描画を含まない構成であり、通常バイナリーは専用 Chromium workflow の agent／windows_agent gate と通常 CI で検証する。macOS の core `40049231515` も別途検証中である。

2026-10-24: `4a8d082` の依存入力は、以前の監査済み tree と Cargo.toml の Windows 専用 `windows-sys` 宣言・Cargo.lock の root dependency だけが異なることを確認した。全 registry package の版・checksum・native source と build 設定は同じである。この特定 tree・Rust 1.94 の実際の compiler identity・既定の flags に限り、以前の正確な v2 dependency key を restore 候補にする。新しい primary key は維持し、Cargo が feature の変わった Rust crate を再ビルドし、app と旧画像の除去も維持する。未知の入力や compiler／flags の変更では候補を返さない6件の Python 試験が成功した。これにより不要な native engine の再ビルドを避ける方針だが、実際の cache restore は次の GHA で確認する。

2026-10-24: Windows の ACP 子孫プロセス管理を実装した。非継承の job handle を host が保持し、補助 worker が自分を job に参加させ、一時 handle を閉じてから adapter を起動する。起動成否は固定長の非機密 status で返し、その後は既存の ACP と stderr の上限付き drain を使う。adapter の stderr は小さい buffer で転送し、読み取り中に出力 lock を保持しない。setup の期限にはこの起動も含め、subscription 環境の API override 除去は維持する。通常 close・別セッションの保持・非協調 cancel・host/runtime の drop・親の強制終了・所有者がない worker・起動エラーの実プロセス試験を追加した。専用 GHA で Skia をビルドせず core の全 target Clippy と全試験を実行する。新しい Windows 処理のコンパイル・実行結果はまだ未確認である。

同日: `windows-sys 0.61.2` を Windows 専用の直接依存として使った。既に lock にある package であり、全 registry package の版・checksum は変えていない。lock の root dependency と root manifest の hash に合わせ、notice inventory／manifest を再生成し、388 packages・267 files を元の archive と照合した。ローカルでは metadata・書式・Python の構文・workflow lint 等のみを確認し、Rust の build/test は GHA で行う。

同日: 角丸の残る専用検証 source `22a7ceb` は Intel Mac `40046844885` の229試験、Linux arm64 `40046850324` の231試験、Windows `40046847216` の201試験が成功した。全 artifact の digest、両構成 Clippy、4種×2版の本家参照 PNG 比較、1000×640 PNG／report と診断0件を検査した。Intel の selector cancel 試験も初回起動の準備制限を分けた source で成功したが、以前の5秒の起動失敗の全原因が解消したという証拠にはしない。macOS／Linux x64 の `d7d4bf2` の結果と合わせ、同じ描画コードの5環境で比較が終了した。今回の Windows job 実装はこの証拠に含めない。

2026-10-24: 同じ角丸 source `d7d4bf2` は macOS arm64 `40046640047` でも229試験と両構成の Clippy が成功した。artifact `12541474147` の ZIP digest・全成功ログ・本家参照4種×2版の PNG 比較・三角形の編集後の版と hash を検査した。図形の参照比較の平均チャネル絶対差は Linux と同じ0.037〜0.087の範囲だった。macOS の今回の画像を新たに目視したという意味ではない。Intel の browser 起動準備修正と、残る新 source の各環境は検証中である。

2026-10-24: 角丸 source `d7d4bf2` は Linux x64 `40046642679` で231試験（graphics20件を含む）と両構成の Clippy が成功した。artifact `12541407638` の ZIP digest・ログ・PNG／report を検査した。4種類の本家参照 corpus は2.19／2.21の両版で比較を通過し、全画像の平均チャネル絶対差は0〜255の尺度で0.037〜0.087、単一チャネルが20を超えて異なる画素は各125〜324だった。形状と横長の半径比較の実成果物も目視した。許容誤差付きの図形比較であり、完全な画素一致や一般の本家互換性とは扱わない。source `a7e0c15` の Windows `40045868654` も182試験と両構成の Clippy が成功し、三角形の編集 file／PNG の hash は既に本家で再出力した macOS の成果物と一致した。

同日: 前 source `0a9ab12` の通常 CI `40043789588` は監査・5環境・source artifact がすべて成功した。全 native artifact の ZIP digest、試験・Material・release report、各42回の benchmark export を検査し、3種類の描画 hash は各同 OS の前回と一致した。試験集計は Linux x64／arm64 が各797、macOS arm64／Intel が各793、Windows が719。この証拠は後続の版対応・角丸追加の証拠へ流用しない。ローカル native build は行わず、未使用を ps／lsof で確認した旧 task cache のバイナリー・監査ツール約67 MiBを削除した。参照画像・検証記録・フォント等の資料は保持する。

2026-10-24: `.pen 2.19` 対応 source `a7e0c15` は macOS の専用 GHA `40045866542` で210試験と両構成の全 target Clippy が成功した。artifact `12541552327` の ZIP digest・ログ・版を保った編集結果を検査した。その実際の編集後ファイルを本家 CLI0.3.9 で再出力し、変更前の赤／変更後の青の三角形ともに144×144の全20,736 RGBA画素が一致した。PNGの符号化 hash は異なる。この証拠は単純図形の色変更を保った読み書きに限り、一般の版別・動的コンテンツ・style・美観の互換性へ拡張しない。角丸 source `d7d4bf2` の macOS `40046640047` と Linux x64 `40046642679` は別途検証中である。

同日: 前 source `0a9ab12` の Intel 専用 GHA `40043812888` は selector キャンセル試験の初回起動（line227）で `RENDER_TIMEOUT` となった。ポーリングの観測・キャンセルが始まる前の5秒の準備制限だった。準備とキャンセル後の cold launch に製品既定の取得制限を使い、準備時間の診断を追加した。対象ポーリングの5秒と観測3秒、キャンセルとプロセス回収の assertion は変更しない。再試行で隠さず、変更後の実行を GHA で検証する。

2026-10-24: 本家 CLI の公開 headless export で、自作の角丸多角形の参照 corpus を追加した。三角形の半径0/10/25/30/50/100、四角形・五角形、縦横比の異なる形と大きな半径を含む。正方形と横長の三角形を独立した辺・円弧の計算と比較すると、「最短辺の半分で半径を制限する」規則とよく一致した。半径上限を bbox や内接円に置き換えると native pixel から外れた。native source・skill・credential は複製していない。

同日: 角丸 polygon のパスを実装した。元の2辺から接点を個別に計算し、大きな半径で接点が隣の頂点を越える場合も接続を保つ。塗り・一様線・効果・visual bounds が共通のパスを使う。負値／不正な半径と退化した辺の試験、本家 PNG との許容誤差付き比較を追加した。英語ガイドは1.0.10。コンパイルと実画像の比較は GHA で行い、まだ成功とは扱わない。

2026-10-24: 既存の本家 CLI `0.3.9` の公開インターフェースで自作の白フレームと三角形を保存・PNG 出力した。保存された版は `2.19` であり、従来の `2.21` のみの書き込み・描画 gate を改善する必要が判明した。opaque な file token を除いた入力を再出力し、144×144 の参照 PNG と出所情報を fixture に追加した。CLI のライセンスは `UNLICENSED` であり、実装・スキル・認証情報は複製していない。モデルへの要求も行っていない。この三角形は現在の楕円内接の頂点配置と一致し、未確認の tight bounds への配置変更は行わない。

同日: `2.19` と `2.21` を明示した codec を追加した。新規文書の既定は `2.21`、既存文書・ユーザーの library は元の版を保持し、不明な版は読み取り専用にする。生成する sampler 補助 library と組み込み style のみ利用先の版に合わせ、catalog の元の source revision は維持する。保存・復元・無効編集・不明版の拒否・混在版の参照・style／sampler library の版・script header・native fixture の描画と編集を試験に追加した。ローカル native build は行わず、GHA の実行と編集後の本家再出力を待つ。これは全機能の本家互換性の完了ではない。

2026-10-24: 相対ローカル文字列 sampler の独立コピーは source `02b0722` の専用 GHA 5環境すべてで成功した。macOS arm64 `40042846601` と Intel `40042853568` は描画なし各8件を含む164試験、Windows `40042848651` は描画なし7件を含む144、Linux x64 `40042850180` と arm64 `40042851811` は描画なし各8件を含む166試験だった。全 artifact の ZIP digest・成功ログ・1000×640 PNG／report の hash・診断0件を検査し、各同 OS の source `1baf453` の画像と一致した。コピーの明暗2テーマの実 PNG 一致、本文・alias・後続編集、日本語／親／途中の path、dry run、Unix の入れ子の symlink 拒否も確認した。物理端末や本家との互換比較の証拠ではない。後続 source `0a9ab12` の Clippy 配置修正・専用 GHA の追加 lints・通常 CI の結果は別に検証中である。

2026-10-24: 文字列 sampler コピーは Windows の source `ffadaa9`／`40042692046` でも137試験が成功した。source `02b0722` の Linux x64 `40042850180` と arm64 `40042851811` は描画なし8件を含む各166試験が成功した。3 artifact の ZIP digest・各成功・1000×640 PNG／report の hash と診断0件、コピーの本文・テーマ・再編集・日本語／親 path・実 PNG 一致を検査し、画像は各同 OS の前回と一致した。最新の macOS／Windows の追加 core gate と Intel は検証中である。

同日: 通常 CI の source `7d37120`／`40041616334` は先に終了した4 native job が Clippy の `items_after_test_module` で失敗した。Linux x64 と Windows の実ログを確認し、前者の試験90集計は791成功・失敗0だったが、run の成功とは扱わない。`src/project/failed_code.rs` に以前追加した guard の試験 module が後続 impl の前にあったため、試験を file 末尾へ移した。抑制や assertion の変更はせず、専用 Chromium workflow にも両構成の全 target Clippy とログ保存を追加した。full CI と専用 GHA の最新 source で検証する。

2026-10-24: 文字列 sampler コピーの修正済み source `ffadaa9` は macOS `40042690191` の156試験が成功した。元 library を削除した後の明暗2テーマの PNG 一致、本文と別名・テーマ・後続の root 変数編集、日本語の directory／file、親・途中の path、dry run／後段不正操作の無書き込み、入れ子の symlink 拒否を含む。artifact `12540067953` の ZIP digest・成功ログ・1000×640 PNG／report の hash・診断0件を検査し、同じ macOS の前回画像と一致した。source `5dbafc0` の macOS／Windows は正規化済みの path と raw temp path を比較した試験の1件で失敗し、`ffadaa9` は Store の正規化済み directory を使うよう修正した。実コピー画像・親 path は修正前も成功したが、run 全体の成功とは区別する。描画なし fixture も仮のテキストから有効な小さい PNG へ変更した。source `02b0722` は専用 GHA に描画なしの sampler 試験を追加し、Linux x64／arm64 はその gate と default build を成功して描画・MCP／TUI の後段を検証中である。Windows と残る全環境の最新結果は継続して確認する。

同日: 配列の背景取得を修正した source `1baf453` の専用検証は5環境すべて成功し、全 artifact の ZIP digest・成功ログ・PNG／report の hash・寸法・診断0件を検査した。macOS arm64 `40041245040` は152試験、Windows `40041246581` は134、Linux x64 `40041441423` と arm64 `40041444018` は各154、Intel Mac `40041445421` は152。画像配列の向き・scalar 混在・既定値・active tail・短い入力からの復帰、object／background、テーマと切り離し等の新しい成功もログで確認した。macOS・Windows・Intel の PNG はそれぞれ前回の同じ OS と一致した。Linux の前回画像との比較や新しい目視、物理端末、本家 Pencil との比較は主張しない。この source は新しい文字列 sampler コピーの追加前であり、その証拠には使わない。

2026-10-24: 相対ローカル URI を使う文字列 sampler 変数の独立コピーを実装した。root の変数・別名・全テーマ枝を保持し、文字列の literal だけを画像と同じ管理 bundle の通常 `.pen` library へ移す。本文の文字列は元のままで、以後 root の変数を編集する接続も保つ。管理 file の型は元の root・URI・hash を保持して staging と公開を検証し、bundle の一覧 hash・128 MiB／4,096 member／1 MiB path 上限・各祖先の symlink 拒否・同期と no-clobber 公開を加えた。`a/../pixel.png` の a ディレクトリも作り、公開後は元の要求 path と物理 target を再検査する。新試験は dry run／後段の不正操作で無書き込み、元 library の削除後の本文・別名・テーマ・再編集、親 path・途中のディレクトリ、入れ子の bundle の symlink 拒否、実画像の前後 PNG 一致を扱う。英語ガイドは1.0.9。Rust の実行検証は GHA で行う。絶対文字列 path・投影先が衝突する source symlink・case collision 等は全互換の残作業に維持する。

同日: 画像 sampler 配列の背景修正 source `1baf453` の Windows `40041246581` は134試験が成功した。artifact `12539503161` の ZIP digest、成功ログ、1000×640 PNG／report の hash と診断0件を確認した。この証拠には今回の文字列 sampler コピーは含まれない。

2026-10-24: source `e406378` の macOS `40040884141` は、追加した sampler 配列のテーマ別画素・切り離し前後の PNG 一致・独立した literal 画像の画素、非表示の配列が欠損／非ローカル画像を開かない試験も成功した。全体は修正前の背景判定の1件で失敗した。Windows の source `3f60528` も同じ1件の失敗で、通常画像配列と独立コピー等は成功した。背景判定を含む source `1baf453` の検証は macOS arm64 `40041245040`、Windows `40041246581`、Linux x64 `40041441423`、Linux arm64 `40041444018`、Intel Mac `40041445421` の5環境で進める。ローカル native build は行わない。

2026-10-24: sampler 配列の source `3f60528` は macOS `40040697389` でコンパイルが成功したが、実 effect 試験で `pencil:background` が利用不可となって失敗した。背景取得を事前に決める `uses_background` が scalar の値しか調べておらず、配列内の背景指定を見落としていたため、同じ判定へ配列要素を追加した。通常画像の配列・上下方向・scalar 混在・active tail・未指定・短い入力からの復帰、library の要素別依存と切り離し、独立コピーの各新試験はこの run で成功した。全体の成功とは扱わず、修正と source `e406378` の追加ケースを次の GHA で確認する。

同日: 使用中プロセスと open file がないことを確認し、この作業の古いローカル Chromium と ACP adapter の node_modules を削除した。作業 cache は975 MiBから467 MiBへ減った。元 package lock・ソース・検証記録を保持し、Rust／Skia／Chromium の実行検証は GHA で続ける。

2026-10-24: 1次元の画像 sampler 配列を実装した。Rust は文字列リストの長さ・型・符号化済み入力64 MiBの上限を検査し、要素ごとの変数・テーマ・参照元と依存 file を捕捉する。WebGL は active な要素を各 texture unit へ設定し、同じ画像の unit を共有し、未指定要素には透明画像を渡す。切り離し・固定画像を含む独立コピーも配列を扱う。新しい試験は実画像の向き、scalar と配列の混在、active tail、定数式の短い入力からの復帰、object／background、テーマ付き library と切り離し、元 library を削除したコピーを検査する。文字列変数を持つ独立コピー・多次元配列・struct・マクロ宣言・本家の注釈既定値などは元の全対応範囲に残す。英語ガイドは1.0.8へ更新した。新しい Rust コードの実行検証は GHA で行い、成功とはまだ扱わない。

同日: source `5aa27e0` の Windows `40039981583` は、不完全なアイコン guard で capture を作らない追加も含め107試験が成功した。artifact `12539411084` の ZIP digest・各成功ログ・1000×640 PNG／report の hash と診断0件を検査した。source `ff46c24` の通常 CI `40037051278` は監査・native 5環境・source artifact を含め全体が成功した。これは新しい sampler 配列や生成 icon guard の成功を示すものではなく、各 native artifact の詳細検査も別に行う。

2026-10-24: source `5c40092` の生成アイコンを含む失敗コード取得は、macOS `40038350002` の121試験と Windows `40039000949` の107試験で成功した。macOS は元 notice の同一バイトの symlink 付け替えも含み、Windows は同じ byte hash の検査・欠損から導入した際の無効化・enabled icon を含む source 取得を確認した。各 artifact の ZIP digest、成功ログ、1000×640 PNG と report の hash・診断0件を確認し、前回の同 OS の画像と一致した。新しい目視や Windows の物理端末・clipboard の証拠ではない。その後 `5aa27e0` は Registry の integrity／未知エラー等で完全な file／欠損 guard がそろわない場合に capture を作らない条件と、壊れた導入状態で再計算する試験を追加した。macOS `40039247655` の121試験がこの追加も含めて成功し、artifact `12538427996` の画像・ログ・ZIP digest を検査した。Windows の同 source `40039981583` は実行中である。

同日: source `38abd0e` の macOS `40037242989` は重複した guard の末尾を見落とさない回帰試験と取消の単独工程を含む120試験、source `d65d1d4` の Intel `40036985460` は119試験が成功した。両 artifact の ZIP digest・成功ログ・PNG の hash／寸法／診断0件を確認し、各同 OS の前回画像と一致した。Intel には後から加えた `38abd0e` の境界回帰は含まれない。source `ff46c24` の通常 CI `40037051278` は Intel の両試験構成・Clippy・Material opt-in を通過して release 検証中で、他4 native job と監査は成功した。先行 `3ea9c65` の取消時間超過の原因が特定されたわけではなく、run 全体の結果もまだ未確定である。共有文字列を使う部品コピーは本家の参照試験も含めて残す。Pencil 接続は `get_app_state` が transport 未接続で失敗し、標準の `/Applications/Pencil.app`／`pen.dev.app` にも実体を確認していない。参考実行や本家との一致を検証済みとは扱わない。

2026-10-24: 生成した追加アイコンを含む計算が入れ子 JS で失敗した場合、`icon_pack` 依存のため全文 capture が作られなかった組み合わせを修正した。Registry の file binding と欠損 pack の path を小さい guard として取得し、既存の file hash と一緒に保持する。アイコン・font payload や Registry 本体は capture へ残さない。追加 install・元 notice の変更・同一バイトの symlink 付け替えでも capture を無効にし、欠損 path も既存の1MiB path budget と4,096件の上限へ含める。実 MCP 試験は、欠損した disabled icon → 原本 archive からの導入 → enabled icon と失敗 JS → notice の変更／復帰 → Unix の link 付け替えまでを通す。ローカルの127 Python 試験・整形・差分は成功し、新しい Rust 試験は GHA で検証する。共有する文字列変数と画像の独立コピーなど、元の残作業は維持する。

2026-10-24: source `1c84f69` の macOS `40036814772` は、library 48・policy 26・実描画32・新しい失敗コード取得の実 MCP 4・実 MCP 画像1・60秒待機回収1・実 TUI 6の計118試験が成功した。新 MCP 4件では library の入れ子の全文・UTF-8 chunk と修復、成功→失敗→成功の選択、同一バイトの symlink 付け替えの拒否、header failure・排出・削除・無関係な file の拒否を確認した。artifact `12537831445` の ZIP digest、ログ内の各成功、画像と report の hash・1000×640・診断0件を検査し、PNG は前回の同じ macOS と一致した。新しい目視や他 OS の成功は主張していない。その後、依存 guard の重複が上限超過の切り詰めを隠さないよう、重複排除前に超過を拒否する修正と回帰試験を source `38abd0e` へ追加した。この追加と取消遅延の検証は macOS `40037242989`、Intel `40036985460` と通常 CI で継続する。

同日: source `24414fe` の Intel 資料結合 `40035656900` も成功した。artifact `12537900609` の442資料の ZIP digest・size／SHA-256・一覧と nested hash・同じ compiler／revision／run ID、library 59件・registry 原本40件、VEX の finding 保持を検査した。これで資産と標準ライブラリを含む実ビルド資料の結合は5環境で確認できた。最終リンク・system・完全な帰属・署名等を完了とは扱わず、`release_ready: false` は維持する。キー設定を含む通常 CI `40034814779` は Intel の取消時間で全体として失敗したが、ほかの4 native job は成功し、4 artifact の12画像の hash・寸法・診断0件と前回同 OS との一致、各42回の benchmark、Linux 各780・macOS arm64 776・Windows 705件の試験成功を確認した。最新の失敗コード取得機能はこの4環境の証拠に含まれない。

2026-10-24: source `24414fe` の Linux arm64 `40035653283`、macOS arm64 `40035646909`、Windows `40035650051` の実ビルドと標準ライブラリ／資産の資料結合が成功した。各442資料の ZIP digest・size／SHA-256・一覧、nested manifest／inventory／source distribution と build 記録の hash、同じ compiler・revision・run ID を照合した。標準ライブラリ source input は各2,274、registry 原本は40、installed target library は順に55／59／44件で、VEX の finding は保持した。資産は各15宣言・20入力・6パックだった。Intel `40035656900` は継続中で、これらは失敗コード取得機能の追加前という scope を保持する。

同日: 失敗コード取得を含む source `c041b45` の macOS `40036529244` は、取消トークンの参照を要求する `Project::set_cancellation` へ値を渡した型不一致でビルドが失敗した。既存の参照 API に合わせて source `1c84f69` で修正し、macOS `40036814772` は本体ビルドを通過して実行試験中である。先行 source `3ea9c65` の通常 Intel `120692050402` は MCP の取消試験が1.5秒の総時間基準を超えて失敗した。元ログでは対象 tool・実時間が分からず原因は未確定である。source `d65d1d4` は同じ基準を維持し、200ms 待機前後と取消から後続応答までの時間・tool を失敗時に表示する。新 Intel 専用 `40036985460` と次の通常 CI で確認し、期限を緩めて成功とは扱わない。

2026-10-24: MCP の失敗した計算で実際に捕捉した入れ子 JS の全文を、同じ接続の `read_asset` から読む経路を追加した。元 error の `source_hash` と捕捉済みの path／kind を照合し、最大1MiBのコード4件・5分を保持する。失敗時の Project・font・scene やほかの payload は保持せず、bounded な依存 hash と path binding だけを残す。通常依存・runtime file・symlink target の変化を再検査し、成功／失敗の capture を新しい順に選ぶ。error／read に `failure_capture_id`、read に `failed_calculation` を返し、layout snapshot と区別する。UTF-8 chunk・全文取得・外側コード変更・修復、成功→失敗→成功、header failure・4件からの排出・無関係なファイルの拒否、同一バイトの symlink 付け替え、取消後の読取・期限切れと容量上限の試験を追加した。英語ガイドは1.0.7へ更新した。ローカルの127 Python 試験・整形・差分は成功し、Rust の実行検証は GHA で行う。CLI をまたいだ永続 capture や元スクリプト仕様の全互換を完了とするものではない。

2026-10-24: source `3ea9c65` の macOS arm64 `40034819092` は、設定したキーでの実 ACP 画像送信と permission の優先、key parser／実 event matching を含む library 47、runtime／設定／script 26、実 readiness／dynamic／shader 32、実 MCP 画像1、60秒待機回収1、実 TUI 6の計113試験が成功した。TUI の集約試験にはユーザー設定から F5 を読み込み、日本語複数行を送信し、置き換えた Alt+Enter が送信しないことと起動中の設定固定を検査する新 PTY case を含む。artifact `12536912406` の ZIP digest・画像 hash／1000×640・診断0件とログ内の各成功を確認した。新しい目視や物理 clipboard／IME・全 OS の成功を示すものではない。CLI schema／設定の追加試験を含む通常5環境 CI `40034814779` は実行中である。

同日: source `0acce40` の Linux x64 release 資料結合 `40033309721` が成功した。artifact `12537031822` の442資料すべての size／SHA-256・一覧と ZIP digest を検査した。nested 標準ライブラリの manifest／inventory／source distribution の hash が build 記録と一致し、同一 compiler・source revision・run ID、library 55件・registry 40原本と2,274 source input、VEX の finding の保持も確認した。資産は15宣言・20入力・6パックを含む。最終リンク・source-to-binary と公開の完了は保証していない。残る4環境の同じ結合を source `24414fe` で開始した: macOS arm64 `40035646909`、Windows `40035650051`、Linux arm64 `40035653283`、Intel Mac `40035656900`。ローカル native build は行っていない。

2026-10-24: 通常 CI `40029370465` の source `eaaaf79` は監査・native 5環境・配布用ソース検証を含む全体が成功した。5 artifact の ZIP digest、各3枚の PNG と report の hash・寸法・診断0件、各42回の benchmark export を確認した。全15画像はそれぞれ同じ OS の source `18a0092` の前回画像とバイト一致し、今回新たな目視は行っていない。試験ログの描画なし／通常合計は Linux x64・arm64 が各770、macOS arm64・Intel が各766、Windows が695件だった。source artifact `12536537610` は job の検証成功・upload・API metadata を確認し、9.3 MB の archive はローカル取得していない。これは asset／標準ライブラリの結合・今回のキー設定追加前の証拠であり、最新 main の成功とは扱わない。

2026-10-24: ユーザー設定 `[tui.keys]` から送信・貼り付けキーを置き換える実装を追加した。省略した action は既定値を使い、明示した配列は置換する。送信は1〜8件、貼り付けは0〜8件で、空の貼り付け配列はキー経由の clipboard 読み取りを無効にする。Ctrl／Alt／Shift、Enter・ASCII letter・F2〜F24 を扱い、文字入力・中断・command palette・旧端末の制御文字と衝突する指定、重複と action 間の衝突を設定エラーにする。設定は端末を開く前にユーザー設定だけから取得し、起動中は固定する。title と help に実効キーを表示し、idle prompt 以外の focus・検索・modal・permission が優先される。parser／実 key event／CLI 設定・schema／実 ACP 画像送信と permission／Unix PTY からのユーザー設定読み取りと F5 送信の試験を追加した。ローカルでは整形・差分・Python 構文と全127 Python 試験が成功した。source `3ea9c65` の macOS GHA `40034819092` は本体ビルドを通過し実行試験中で、通常5環境 CI `40034814779` も開始した。Rust 試験と実 PTY の結果は未確定である。全キーの変更や物理 clipboard／端末互換の完了とは扱わない。

2026-10-24: source `6e1fce2` の Linux arm64 `40033700360` と Intel Mac `40033703894` でも単独標準ライブラリ収集が成功した。各100資料の ZIP digest・size／SHA-256・一覧の完全性を確認し、各2,274 source input・registry 40原本、Linux arm64 55件／Intel 59件の installed target library を記録した。VEX の finding は保持している。これで単独収集は hosted 5環境で確認できた。実アプリへの結合は Linux x64 `40033309721` で検証中であり、最終リンクや公開の完了とは扱わない。

2026-10-24: `scripts/asset_notice_inventory.py` を release 資料収集へ接続した。ソース内の font／icon の literal include 15件をレビューした一覧へ照合し、Noto Sans JP の font／OFL／publisher metadata と、6アイコンパックの data／index／license／provenance の計20入力を既存の manifest・catalogue の pin へ結び付ける。OFL、各 license の原文、font metadata・manifest・icon catalogue と来歴を17資料として保持し、font や icon payload は重複コピーしない。Phosphor の元 CRLF も変更しない。新規・欠損・重複・計算された include、リンク、古い provenance や変更された入力を拒否する。Python 全121試験と現行 repo の実20入力からの収集・再構成検証が成功した。実資料は121,214バイトで、作業用コピーは削除済み。native のローカル build は行っていない。GHA の実収集はまだ未検証であり、最終リンク・全素材の適用・標準／system・署名等の完了とは扱わない。

同日: 標準ライブラリの資料を hosted runner 上で準備する `scripts/ci/toolchain_evidence.py` と専用 workflow を追加した。Rust 1.94.0 の commit `4a4ef493…` と公式 source archive の SHA-256 `0b53ae34…` を固定し、library・原文 license・unwind の参照資料だけを選択して展開する。library lock の registry archive は元 checksum を照合し、既存の collector で実際の installed target library と原文資料を再構成検証する。コンパイルは行わず、大きい source archive・準備用ツリーは成功後に runner から削除する。新 helper は GHA 上でだけ動作し、ローカルでは構文・workflow 検査と既存 collector の19試験だけを行った。実行と、アプリ実ビルドの同じ compiler／target library への結合はまだ未検証。

同日: source `331c118` の macOS 資産拡張 run `40031829807` が成功した。取得した342資料の ZIP digest・size／SHA-256・一覧の完全性、15宣言・20入力・6パックの記録、OFL／Phosphor 原文の hash と payload を重複させていないことを確認した。また source `466d90c` の Linux x64 標準ライブラリ run `40032401890` が成功し、100資料の ZIP digest・size／SHA-256・一覧を確認した。公式 source から45,054,109バイト・2,278ファイルを選択し、library の2,274入力・registry 40原本・installed library 55件を再構成検証した。fortanix-sgx-abi 0.6.1 の完全な対応ソースも含む。vex-sdk 0.27.1 の publisher VCS `dirty: true` は finding として保持し、pristine と扱わない。

同日: 標準ライブラリ資料をアプリ build へ結合する `scripts/toolchain_build_binding.py` を追加した。manifest の完全性・必要な原文文書・compiler の全識別情報・installed target library 全件の実測バイト hash を照合し、コピー後にも検証する。別 compiler／target、同じ版でも異なる library、別 source 配布物、原文の欠損・改変・余分な資料・既存出力への上書きを拒否する。release workflow は実 build 前に同じ runner で資料を準備し、実 build 後の収集で結合する。Python 全127試験が成功した。実アプリ build の結合はまだ未検証で、最終リンク・source-to-binary・system・全帰属と公開の完了ではない。

同日: 資産追加前の source `1355cfd` の Linux arm64 `40030972290` と Intel Mac `40030975630` が full Skia／QuickJS 資料収集を成功した。取得した各325資料の ZIP digest・size／SHA-256・一覧の完全性、19入力と元 registry archive の照合結果を確認した。Skia 入力は Linux arm64 が2,972、Intel が2,936件だった。これで基本の native／対応ソース収集は5環境で確認できた。新しい資産・標準ライブラリの追加前という scope を保持する。source `331c118` の Windows 資産拡張 `40031833313` も成功し、342資料と15宣言・20入力・6パック、payload を重複していないことを照合した。標準ライブラリ結合は source `0acce40` の Linux x64 `40033309721` で、単独の標準ライブラリ収集は Windows `40033312900` と macOS arm64 `40033316361` で検証を進める。

同日: source `0acce40` の単独標準ライブラリ収集は Windows `40033312900` と macOS arm64 `40033316361` も成功した。各100資料の ZIP digest・size／SHA-256・一覧を検査し、各2,274 source input・registry 40原本、Windows 44件／macOS 59件の target library 記録を確認した。VEX の finding は両環境でも保持した。library の版だけでなく各バイト hash を結合条件にする必要がある。これらは実アプリへの結合前の証拠であり、同じ source の Linux 結合 job `40033309721` と、source `6e1fce2` の Linux arm64／Intel 単独収集を継続する。

2026-10-24: Windows の source `14aa254` の資料収集が、実測 hash と改行診断を伴って停止した。不一致の9ファイルはすべて bare LF／CR が0で、LF に戻した hash は既存原本の pin と一致した。Skia DEPS の zlib commit `646b7f569718921d7d4b5b8e22572ff6c76f2596` から9原本を独立に取得し、元 LF の SHA-256 と Git blob、および LF→CRLF の正確なバイト列が Windows の実測 SHA-256 と一致することを再計算した。これら9件だけの変換後 hash と元 Git blob／取得 URL を policy へ追加した。適用には元 LF hash・Git blob・CRLF のみの改行・Skia DEPS の同じ source origin も必要で、原本 LF の pin は維持する。実際に適用した form と元の入力 hash を記録し、内容変更・混在改行・bare CR・未登録の変換・別の origin は拒否する。Python 全114試験と、既存 macOS 保存資料の9原本への再照合が成功した。修正後の Windows GHA の実収集はまだ未検証。

同日: source `5a9698e` の Windows `40028843720` が Python 114試験、release ビルド、Skia／QuickJS の収集・再構成検証・資料保存まで成功した。artifact `12533599483` の ZIP digest、325資料の size／SHA-256・一覧の完全性を検査した。9参照元すべてが登録した `reviewed_lf_to_crlf` と実測 hash で記録され、対応する元 Git blob と DEPS source origin の検証も通過した。Skia は2,957入力・33原文、QuickJS は MSVC 対象の実19入力・C 4件・15 comment 候補を元 registry archive へ照合した。MSVC の同内容 archive 別名を含め Skia／wrapper 7件・QuickJS 2件を記録する。対応ソース option-ext と MPL 原文の hash も照合した。これで source `74d4a3d` 以来の Windows 参照元不一致を解消したが、`release_ready: false` は維持し、標準／system／素材・完全な帰属・署名と公開の残作業は継続する。

同日: 通常 CI `40024349686` の source `18a0092` は Linux x64・macOS arm64・Windows の各 job が成功した。取得した3 artifact の ZIP digest、release 3画像ずつの寸法・hash・診断0件、各42回の benchmark 出力を確認した。描画なし／通常は Linux x64 が291／479、macOS arm64 が289／477、Windows が259／436試験で、両 Clippy・Material 2 opt-in も成功した。Linux x64 の3画像は目視でも確認した。Intel と Linux arm64 はこの run では実行中であり、source artifact や全体の成功とは扱わない。Linux x64 の v2 native cache 保存を終了ログで確認してから、同環境での full 資料収集 `40028057056` を開始した。

同日: 同じ通常 CI `40024349686` は、その後 Intel と Linux arm64、監査および `Verified source artifact` job を含む全体が成功した。追加2環境の artifact の ZIP digest、release 3画像ずつの hash・寸法・診断0件、各42回の benchmark 出力を確認した。描画なし／通常は Intel が289／477、Linux arm64 が291／479件で、両 Clippy・Material 2 opt-in も成功した。新 image `20261023.149.3` 用の Linux arm64 v2 cache が保存されたことを終了ログで確認した。これは source `18a0092` の証拠であり、最新 main の Python／policy／取消 fixture の変更は次の通常 CI で確認する。

同日: source `e99b62e` の Linux x64 full 資料収集 `40028057056` も成功した。artifact `12534248479` の ZIP digest、325資料の size／SHA-256・一覧の完全性、QuickJS の GNU 対象19入力と元 registry archive、option-ext 対応ソースと MPL 原文を照合した。Skia は2,954入力・33原文資料。Windows、macOS arm64 に続く3環境の実収集であり、残る Linux arm64 と Intel Mac の full 収集を、それぞれ保存を確認した native cache から source `1355cfd` で開始した。標準ライブラリ・system・素材・最終帰属・署名等の残作業は継続する。

同日: source `5a29847` の Intel Mac Chromium `40027340811` が成功した。library 43・runtime 4・設定10・script 12・browser_readiness 5・dynamic_render 17・shader_uniforms 10・実 MCP 画像1・60秒待機回収1・TUI 6の計109件が成功し、runtime archive 用の別 opt-in 1件を除外した。起動済みの browser で対象 selector の問い合わせを実際に観測してから取消・回収・正常描画まで成功した。v2 native cache の復元も終了ログで確認した。artifact `12534221384` の ZIP digest、1000×640の画像 hash・診断0件・2取得・実 browser `HeadlessChrome/156.0.8078.47`・実効 policy・管理 runtime の完全性を検査し、画像も目視した。Intel の hash `b10c0cf5…` は初回の基準であり、他 OS とのバイト一致は要求していない。物理端末や clipboard の証拠ではない。

同日: QuickJS 拡張前の source `74d4a3d` の Linux arm64 資料収集 `40025475997` も成功した。取得した306資料の size／SHA-256・一覧の完全性と ZIP digest を確認した。Skia は2,972入力・8外部部品と Skia・33原文資料、native archive は Skia／wrapper 6件と QuickJS 1件で、Rust 388 package の既存通知を含む。この run は新しい QuickJS 入力照合と対応ソース追加の前であり、それらの Linux 検証完了へ読み替えない。

2026-10-24: QuickJS の native 入力を元 archive へ結び付ける `scripts/quickjs_notice_inventory.py` を追加し、release 資料収集へ接続した。rquickjs-sys 0.14.0 の Cargo.lock checksum とレビューした build.rs の SHA-256 を固定し、選択された manifest・build.rs・C／header 19入力・各対象の Rust FFI binding を原本へ照合する。生成された target 選択 macro は rustc の host と一致する必要がある。コピーされた C／header の全文から原文の著作権・license comment 候補を保持する。bindgen／update-bindings、新しい版・build script・対象はレビューなしに通さない。release 資料の `licenses/` には既存の対応ソース archive・MPL 本文・元通知も相対パスを維持して含める。Python 全108試験と、実際の checksum 検証済み archive を使った5対象のコピー fixture・再構成検証が成功した。fixture は native をコンパイルしておらず、実 GHA の結果は下記に記録する。最終リンクや完全な帰属レビューの完了とは扱わない。

同日: QuickJS 追加後の source `ec6d326` の macOS arm64 `40026880547` が、release ビルド・Skia／QuickJS の元入力からの再構成検証・資料保存まで成功した。取得した artifact `12533666504` の ZIP digest と325ファイルの size／SHA-256・一覧の完全性を検証した。QuickJS は実ビルドの19入力・4つの C source・15 comment 候補・target binding を記録し、元 registry archive を checksum で検証している。同梱した option-ext 0.2.0 の対応ソース archive と MPL 本文も provenance の hash と一致した。実行ファイルの hash は拡張前の macOS 資料と同じ `0f0fe410…` だった。この環境の入力照合の証拠であり、他 OS や完全な法的レビュー・標準／system／素材の監査・署名の完了ではない。

同日: QuickJS 拡張前の source `74d4a3d` の macOS arm64 run `40025472254` が、release ビルドと資料収集・元入力からの再構成検証を含めて成功した。取得した ZIP の digest と306資料の各 size／SHA-256・一覧の完全性を検証した。資料は source 596ファイルの manifest、実行ファイルの hash、Skia の2,954入力・8外部部品と Skia・33原文資料、既存 Rust 388 package の通知を記録する。native archive は Skia／wrapper 6件と QuickJS 1件。これは新しい QuickJS 入力照合の成功や registry 388 archive の現物検証ではなく、標準・システム・素材・最終リンク・完全な法的レビュー・署名も残る。`release_ready: false` を維持する。

同日: source `74d4a3d` の Windows `40025479412` も release ビルドが成功したが、native の参照元ソースが既存 policy の hash と一致せず、資料収集で停止した。元のエラーは不一致のパスを記録しておらず、この時点で未登録の入力・改行差・別の変更のどれかは確定できない。不一致のパス・期待／実測 hash・改行の個数と LF 換算 hash を診断へ加えた。照合条件は変えず、改行だけを変更した入力も引き続き拒否する試験を含め、Python 全109試験が成功した。Linux arm64 の最初の source `18a0092` も global package ID の重複拒否で失敗したことを終了ログで確認した。同ジョブでは runner image が `20261023.149.3` に更新され、正しい image の native cache がなく cold build だった。別 image の cache を混ぜず、後続の同環境 job と通常 CI を継続する。

同日: source `ec6d326` の Windows `40026876238` は新しい改行診断の fixture で、Python の `write_text` によって既に CRLF になった入力へ CR を重ねたため、bare CR の期待値だけが失敗した（108成功・1失敗）。意図する LF 原本を bytes で固定してから CRLF 版を作るよう修正した。アプリや照合条件の変更ではない。Windows の参照元不一致の実入力は、次の資料収集で引き続き確認する。

同日: source `74d4a3d` の Intel Chromium `40025468263` は、library 43・runtime 4・設定10・script 12、browser_readiness の4・dynamic_render 17・shader_uniforms 10が成功した。selector 取消の1試験では、初回の Fonts／layout／browser 起動前から数えた3秒以内にページからの問い合わせ通知を受信できず失敗した。これだけでは取得失敗・起動時間・fixture 通信のどれが直接原因かは確定できない。試験の目的に合わせ、selector を設定しない描画を完了してから対象 selector を設定し、起動済みプロセスでの selector polling を観測するよう修正した。通知も対象 `#never` の問い合わせだけへ限定し、未観測時の error と経過時間を出す。3秒の観測上限・5秒の取得上限・取消・プロセス回収・後続の正常描画は維持する。後段の実 MCP／TUI はこの run では未実行であり、修正後の GHA で改めて確認する。

2026-10-24: release ビルドと通知資料を結び付ける `scripts/release_evidence.py` と手動 GHA を追加した。検証済みのソース配布物からアプリを再ビルドし、Cargo JSON の package identity で native artifact と build-script を対応付け、報告された出力ディレクトリを使う。実行ファイルと Skia・wrapper・QuickJS archive、rustc と runner の識別情報、Skia の実入力・原文通知、および検証済みの Rust 通知を記録する。収集中のソース・成果物・Cargo 記録の変化、未記載ソース、異なる内容の archive 別名、既存出力への上書きを拒否する。MSVC で cc が生成する同内容の `.a`／`.lib` 別名は両方を記録する。ローカルの Python 101試験が成功した。GHA の実収集はまだ未検証。実行ファイルはアップロードせず、`release_ready: false` と残る監査範囲を明示する。標準・システムライブラリ、native／素材の完全な帰属、最終リンク、署名と公開が完了したとは扱わない。

同日: 最初の macOS arm64 の収集 run `40024439088`（source `18a0092`）は依存 cache の復元と release ビルドが成功したが、Cargo の build-script 記録を全 package ID について一意に要求したため、収集工程で停止した。Cargo が異なる host／feature context で依存をビルドできることを考慮し、選択した native package だけを一意に対応付けるよう修正した。対象 native の曖昧さは引き続き拒否する。実際に重複した package の特定はこのエラーログだけではできない。別 package の複数 context と収集中の Cargo 記録の改変を含む Python 全102試験が成功した。修正後の実収集は次の run で確認する。

同日: source `36d1269` の Intel Mac job `120622365013` が成功した。描画なし284・通常470試験、両 Clippy、通常 build、2つの Material opt-in、release の3画像検査と42回の性能出力まで通過した。取得した3画像は report の hash と一致し、日本語・輪郭線・辺別罫線・画像 crop・影・mesh の見た目も確認した。実行後にアプリを prune して v2 native cache を保存したことをログと cache API の両方で確認した。通常 CI 全体は Linux x64 の依存取得失敗を含み、全体の成功ではない。新しい source の全環境 CI と Intel の Chromium 専用検証は別に確認する。[性能の観測](performance.md#hosted-intel-macos-release-build-2026-10-24)

## 必須要件

以下は macOS Apple Silicon・Linux x64／arm64 の3環境について満たす。Windows の追加対応と Intel Mac のサポートは初回の必須要件に含めない。

- [ ] Rust の CLI と共通サービス。機械向け JSON、終了コード、取消、設定と診断。
- [x] コメント・未知フィールド・数値表記を保持する JSONC の読み書きと、形式の版の管理。`tests/document_store.rs` の保持・保存／復元・拒否の4試験を [全3環境の両構成](https://github.com/EkeMinusYou/lazypencil/actions/runs/40595368880) で確認し、現在の該当ソース・試験との一致を照合した。対応版は2.19／2.21、その他の版は読み取り専用。
- [x] 原子的な編集バッチ、ID の再割り当て、参照・型の検証、要求 ID による重複防止。`tests/operations_cli.rs`／`tests/document_store.rs` の関連9試験を同じ全3環境・両構成のログに照合し、現在の operations／service／store と試験の一致を確認した。
- [x] ファイルロック、期待リビジョン、依存の競合、同一ディレクトリでの置換、履歴とクラッシュ復旧。`tests/document_store.rs`／`tests/project.rs` の関連13試験を同じ全3環境・両構成のログに照合し、現在の store／service／project・試験・コンパイル設定との一致を確認した。同一ディレクトリの一時ファイル、置換前の file sync、置換後の directory sync と journal の復旧分岐も実装で確認した。
- [ ] ローカル・外部部品、スロット、上書き・置換・切り離し、変数・テーマ・別名。
- [ ] 固定・内容・親に従う寸法、自動・絶対配置、文字の成長方式、計算結果とあふれの取得。
- [ ] Skia CPU・SkParagraph による図形・パス・文字・画像・塗り・線・効果の PNG 出力。
- [x] 同梱 Noto Sans JP、追加フォント、代替と欠損の診断、固定したライセンスとハッシュ。読込・shaping・代替・欠損・固定入力・CLI／MCP／設定の19試験を [全3環境の検証済みソース `e249f07`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40595368880) の実ログに照合し、該当実装・試験が現在も一致することを確認した。同梱フォント・OFL・metadataの3ファイルは固定したGoogle Fonts原本と全byte・size・SHA-256が一致した。追加形式・制限と、別途残る任意フォントの互換性・遅延探索・再現性の範囲は [フォント](fonts.md) に記載する。
- [ ] Lucide 基本パックと追加アイコンパック、素材の管理とライセンス表示。
- [ ] 隔離した軽量 JS ワーカーによる script ノードの実行と編集。
- [ ] 明示取得する固定版 Chromium、browser ノード、GLSL の塗り・効果、ソフトウェア WebGL。
- [ ] 文書に束縛する MCP、画像ブロック、指針・スタイル・部品・計算結果の取得。
- [x] 英語の独自ガイド・スタイル・編集可能な作例。
- [ ] Claude と Codex の ACP 接続、画像入力、会話、承認、取消、異常終了の処理。
- [ ] ファイル・クリップボードの画像添付、macOS Ctrl+V、確認・削除、複数画像。
- [ ] TUI、日本語入力、ノード選択、パン・ズーム、Kitty・Sixel、ファイルと依存の更新。
- [ ] macOS Apple Silicon・Linux x64／arm64 のビルドと、対応環境・制約の明記。
- [ ] 意味のある自動試験、CLI と MCP の実行試験、描画成果物の確認。
- [ ] CI、配布手順、ライセンス・依存監査、使用例とトラブルシューティング。

## 現在の検証環境

現在の継続検証は対象3環境の GHA・Rust 1.94.0 で行い、ローカルでは native build を行わない。下表は独立に成果物を確認した source `c355f19` の結果であり、後続の描画変更の全検証や、物理端末・最低 OS の保証へ読み替えない。

| GHA 環境 | 通常・release CI | 同じ native job の Chromium 検証 |
| --- | --- | --- |
| macOS 15 arm64 | 両構成・Clippy・Material・release 成功 | 実 page／shader・MCP 画像・60秒 idle・TUI／再生 PTY 成功 |
| Ubuntu 24.04 x64 | 両構成・Clippy・Material・release 成功 | 実 page／shader・MCP 画像・60秒 idle・TUI／再生 PTY 成功 |
| Ubuntu 24.04 arm64 | 両構成・Clippy・Material・release 成功 | 実 page／shader・MCP 画像・60秒 idle・TUI／再生 PTY 成功 |

上記 `40281460362` の各環境の画像・hash・各42回の benchmark・raw linkage と、1,135ファイルの配布ソースを Git の該当 revision に照合した。先行する `7e651fb` では、Linux の実 Kitty／xterm Sixel の明示・自動4モードと debug／release の64画像、および全3環境の隔離した native clipboard→Ctrl+V→ACP の画像入力も独立に確認した。これは物理 IME や各端末の clipboard 設定の保証とは区別する。Windows／Intel の過去の結果は下記の実装履歴と [runtime の検証記録](runtimes.md#hosted-integration-without-local-build-trees) に保持する。過去のローカル検証は macOS arm64・Rust 1.99.1 と Debian 12 arm64・Rust 1.94.0 で行った。対象3環境の実端末・最低 OS・互換性・性能・公開品質全体の確認は残る。比較用成果物は利用条件を確認したものに限り、本家の出力利用を前提にしない。

## 実装記録

2026-10-23: Rust プロジェクトを作成。文書と保存の基盤、および Skia のビルドを開始する。公開可能な完成状態としては未検証。

### 実装済みの部分と検証根拠

以下は部分機能の記録であり、上記の必須要件全体の完了を示すものではない。

| 部分 | 現在の動作 | 根拠 |
| --- | --- | --- |
| 文書 | JSONC を元のソースのまま保持し、対象の値・構造だけを変更する。重複キー・ID、深さ・容量上限を検査する | `tests/document_store.rs` |
| 保存 | 正規化した文書パスによるロック、期待 SHA-256、準備記録の同期、同じディレクトリでの置換、要求 ID の重複排除、履歴と中断復旧 | `tests/document_store.rs` の競合・復旧・権限試験 |
| パスと共有ロック | 履歴保存先を変えても共通のロックを使う。macOS で大小文字・Unicode・シンボリックリンクによる別名を実パスへそろえ、ハードリンクの別パスは保持する。画像出力先が入力画像の別名なら拒否する | `tests/paths.rs`、`tests/document_store.rs`、`tests/render.rs`。macOS の実ファイルで試験 |
| 編集 | 挿入・更新・削除・移動・複製と変数操作。同じバッチ内の ID 参照と、複製時の内部参照の書き換え | `tests/operations_cli.rs` |
| 部品の読み取り | 外部ライブラリ、入れ子の参照、子孫への上書き・置換、スロットの子の差し替え、テーマと変数の解決 | `tests/project.rs`。外部参照の記法を含む本家との互換性保証は別途必要 |
| 部品の編集 | インスタンスへの上書き・解除・置換、スロットの完全な子配列、テーマの軸、import の追加・削除。ローカル・外部・継承したインスタンスの切り離しは、入れ子の参照・変数と素材の元を保持し、子の ID と内部参照を再割り当てする。保存前に解決結果を比較する | `tests/components.rs`、`tests/detach.rs`、`tests/imports.rs`、実 CLI・MCP。部品の link/static copy、素材の準備と保存前の入力検査も実装。外部 script と shader 画像変数の切り離しは下記。独立した script コピー、文字列と画像の変更連動を保つ shader 変数の独立コピー、含まれる定義の再利用は未完成 |
| ノード探索 | 継承した子のアドレスと属性の出所、宣言済みライブラリとその入れ子の読み取り、展開ツリーの検索、依存に束縛したページ。置換・スロットの保存済みノードも ID 検索に含める | `tests/components.rs` の共通サービスと実 CLI、`tests/mcp.rs` の実通信。宣言済み画像・コードの欠損時も保存済みノードを読み、欠損診断と依存リビジョンを返す |
| 計算済みノードの取得 | `read_nodes` の明示した計算と保持 ID の再利用、CLI の `inspect --calculated`。生成・継承ノードの解決済み属性、出所・テーマ、生成元、同じ計算の位置と文字計測を返す。通常の読み取りはコードを実行しない | `tests/calculated_nodes.rs` の7試験、`tests/mcp.rs` の実通信。ライブラリのテーマと入れ子、ページ・容量制限、依存変更、取消と復帰、生成 ID の編集拒否と元文書の保持を確認。全形式・本家との一致は未検証 |
| 欠損素材の修復 | 宣言済みローカル画像・JS・GLSL・shader 画像入力の欠損を依存に含め、部分修復・差し替え・削除・ドライラン・履歴復元を通常の編集から行う。新たな欠損先・用途は拒否し、既知の欠損がある計測・描画は失敗とする | `tests/repair.rs`、`tests/components.rs`、実 CLI・stdio MCP、保存準備中の出現競合。JS の差し替え後の実ワーカーと実画素も確認。同じパスの複数用途、親ディレクトリと `./`、既存・dangling symlink を扱う。ライブラリ・設定フォント・実行時生成素材などの修復と各 OS は未完成。[対応範囲](mcp.md#repairing-missing-assets) |
| 素材追加 | 内容ハッシュによる PNG・JS・GLSL の不変保存、同時追加の収束、既存ファイルの改変検出。公開先の排他ロックで欠損・既存の依存読取と協調し、待機後も内容を検査する。CLI・MCP・部品取り込みの取消を公開前まで接続する。画像の形式・容量・寸法・終端・アニメーションを検査し、JPEG の向きを反映して PNG 化する | `tests/assets.rs`、`tests/imports.rs`、実 stdio MCP。共有ロック保持中の待機、別の文書・data ディレクトリー、待機中の改変、CLI の SIGINT、MCP の取消後の復帰、画像・コードの追加と実描画を確認。`prompt` の登録済み添付 ID は `tests/agent.rs` で実 MCP・PNG 返却まで確認。TUI の添付経路と実 OS のクリップボード検証は下記のように区別する |
| 依存 | 実効ツリーが使う文書・画像・JS・GLSL と、明示した追加フォントの不変バイト列とハッシュを取得。保存直前にも再確認し、協調する書き手に対して共有ロックを保持する | `tests/project.rs` の素材変更・追加フォント・保存直前の競合・復元・重複排除試験 |
| JavaScript の初期実装 | 別プロセスの QuickJS-NG による同期実行、ヘッダーと6種の入力、生成ノードの計測・描画、入れ子、部品参照、文書変数、取消・時間・ヒープ・出力の制限。待機時の終了と TUI の基本プレビュー・明示更新 | `tests/scripts.rs`、`tests/script_render.rs`、`tests/mcp.rs`、実 PTY。旧ヘッダー・本家との互換性・TUI の全操作と各 OS は未完成 |
| スクリプト診断 | ヘッダー・構文・実行時の元ソース位置、1始まりの UTF-8 バイト列、最大3行・各240文字の抜粋。動的コードの呼び出し・終端・位置なしを区別し、所有アドレス・パス・ハッシュを保持。通常表示でも該当行を返す | `tests/script_diagnostics.rs`、実 CLI、`tests/mcp.rs` の外部ライブラリ由来の入れ子の失敗とコード差し替え。5種類の行区切り・BOM・日本語・長い行・偽のメッセージ・表示制御文字・位置ラベルと、内部変数・正規化 API の隔離を確認。例外スタック自体はコードが置き換えられるため位置は手がかりとして扱う |
| 実行時依存 | 生成物が参照する素材を別の不変スナップショット依存に記録し、計算 ID と画像へ反映。再利用・コード読み取り・編集の明示的な前条件で外部変更を検出する。保存前のコード評価はロック外で実行し、保存直前にも素材を検査する | `tests/script_render.rs` の保存・ドライラン・再送と実 CLI、`tests/mcp.rs` の実通信・実行中取消、共通サービスの準備中の素材変更試験 |
| 外部 script の切り離し | 元の JS とライブラリへリンクし、生成物・明示入力・既定入力の解決元を保持する。版付き `metadata.lazypencil.script_scope` は独自拡張。コード・生成物は書き換えない | `tests/script_scope.rs` の実ワーカー・実画素・切り離し前後の PNG 一致、別文書からの再切り離し、入力・コードの変更、同じ JSON 値での上書き。実 stdio MCP も確認。本家への往復描画互換、独立した script コピーは未完成 |
| Chromium 導入 | 固定版 Chrome for Testing の明示 HTTPS 取得または同じ固定 ZIP、全ファイルの検証、段階配置、再利用、改変の検出。描画時には取得しない | `tests/runtime.rs`。macOS arm64 で実 HTTPS 導入と固定 ZIP 導入を確認。ほかの配布対象の実動作、導入取消・host library 診断は未完成 |
| Browser 初期描画 | HTTP(S)・HTML data・about:blank、寸法・ズーム・スクロール・基本デバイス、ページ画像の合成と明示更新、取得 ID・条件・時刻・版・ハッシュ。待機時の終了と TUI の基本プレビュー・明示更新 | `tests/dynamic_render.rs`、実 MCP・PTY。専用認証プロファイル、本家の全デバイス ID、状態保持、取得条件設定・周期更新と各 OS は未完成 |
| GLSL 初期描画 | 別プロセスの SwiftShader WebGL、塗り・線・効果、数値・色・ベクトル・画像・時刻・解像度、対象画像・親レイヤーを含む背景、順次効果、上下方向とアルファ | `tests/dynamic_render.rs` の実画素・UV precision・微分関数試験、実 CLI・MCP。注釈の全既定値、配列等の宣言互換、塗り段階の対象画像、本家との出力比較は未完成 |
| 文字とフォント | 同梱 Noto Sans JP の日本語組版、折り返し、可変ウェイト、欠損の診断。同じ Paragraph を計測と描画に使用する。TTF・OTF・TTC・OTC の全フェイス、静的スタイルの選択、明示ファイルの基本フォールバック、実際の字形 run が使うフェイス・軸・合成書体の記録。CLI の一覧・ライセンス取得 | `tests/text.rs`、`tests/font_sources.rs`、実 MCP。独自の7フォントを再生成し、コレクション由来と単独版のハッシュ・PNG の一致、通常・太字・斜体、CFF・混在アウトライン、日本語・補助面のフォールバック、変更依存・欠損と容量制限を検査。macOS の実 Arial も確認。プロジェクトのファイル・検索先・既定書体・代替列・言語設定も実装。ユーザー共通設定と項目ごとの上書きも実装。全可変軸・絵文字・遅延読み込み・各 OS は未完成。[フォントの実装](fonts.md) |
| ベクターアイコン | Lucide 1.57.0 の同梱、Feather 4.29.2・Phosphor Regular 2.1.1・Material Symbols 3書体の明示導入。CLI・MCP の名前・別名・タグ探索、完全なライセンス本文。Material の実可変軸100〜700と小数、変数・部品・テーマ、塗り・効果をネイティブ形状へ共通化する | `tests/icons.rs`、`tests/optional_icons.rs`、`tests/mcp.rs`。Lucide／Feather の2,441名、Phosphor の3,060名、Material の12,912名を確認。原本 SVG／フォントの実 Chromium 比較、MCP の重み編集と PNG、改変と計算失効、共有キャッシュを検査。パックの版・生のライセンス・来歴は固定。全 OS・カタログ設定と公開準備全体は未完。[アイコンの実装](icons.md) |
| shader 画像変数の参照元 | 別名とテーマ条件の先で値を定義した文書から画像を解決する。塗り・線・エフェクトで共通化し、文字列の用途と元の変数への参照を保つ。切り離しに追加のメタデータは不要 | `tests/sampler_sources.rs` の解決元・条件・文字列・変更後の追従・依存競合、実 SwiftShader の画素と切り離し前後の PNG 一致。実 stdio MCP の参照元と切り離しも確認。相対ローカル URI の独立コピーは通常の変数 library で実装し、5環境の描画なし／実コピー PNG と変更連動を確認した。最新 Clippy 修正の検証は継続中。絶対 path・衝突する投影と本家との規則の互換比較は未完成 |
| レイアウト | 横・縦の自動配置、余白・gap・整列、固定・内容・fill の寸法、絶対配置、文字成長、循環の検出、回転・反転、あふれの警告 | `tests/layout.rs`、`tests/geometry.rs`。未定義の既定値などの互換検証は残る |
| PNG | 矩形、角丸、楕円・リング、ポリゴン、SVG パス、文字、画像、色・線形／放射／角度グラデーション、均一幅の線、クリップ、透明度、ぼかし・外側の影 | `tests/render.rs`。角度グラデーションなど、まだ個別の画素試験がない部分もある |
| 背景ぼかしと文書の切り出し | Skia CPU の背景ぼかし、色と背景の置き換え範囲の分離、半透明・複数の親の透明度、角丸・回転・クリップ、後続の影と GLSL。ノードのプレビューも文書の合成を保つ。CLI の `--region`、画面外ノードの除外、明示的な中間画像の予約上限と解放 | `tests/background.rs` の10試験、共通リソース予約の失敗後の解放、実 MCP の PNG、実 SwiftShader の順序と alpha。中間画像の対象範囲・大きい文書の性能・本家との全体比較は未完成 |
| 文字・罫線の拡張 | SkParagraph による下線・取り消し線と、同じ配置済み字形からの文字の輪郭線。長方形・frame・browser の辺別罫線、角丸と3種類の角の結合、画像・GLSL の罫線、実際の線の領域を含む出力範囲 | `tests/graphics.rs` の日本語・Latin・複数行・太さ・整列・空白・内外の線・鋭角の範囲・無効な罫線、`tests/dynamic_render.rs` の実 GLSL 罫線。装飾自体の輪郭も同じ文字計算から取得し、字形と結合した線へ適用する。ほかの形状は top を全周へ使う新実装と33ケースの比較を GHA で検証する。bitmap 字形の輪郭線は未完成 |
| 画像と内側の影 | 正規化した crop transform、cover/contain/stretch、回転・せん断・source 内への cover 制約。内側の影は対象の alpha を保つ | `tests/graphics.rs` の実画素・外部部品由来の画像と罫線・半透明の影。全体作例 `examples/graphics-study.pen` は1000×760で変換・目視確認。下記の合成モードも実装。本家との画素一致は未検証 |
| 影と塗りの合成 | 18モードの内側・外側の影、実際の背景との合成、影の色と置換範囲の分離、親の透明度と順序。linearBurn と linearDodge は色を混ぜた後に通常の alpha 合成をする。中間画像を出力のピクセル格子へそろえ、通常の子では不要な背景取得を避け、入れ子の画像を共用する | `tests/blends.rs` の独立した計算式と実画素・半透明・クリップ・回転・拡大・領域とノードの切り出し・画像と勾配と罫線・ゼロ領域、実 stdio MCP のテーマと PNG。実 Chromium／SwiftShader では GLSL の前後の影も確認。独自の18モード作例を描画・目視確認。本家の色空間・全条件との一致、release 性能・各 OS は未検証。[実装と条件](graphics.md#shadow-and-paint-blending) |
| Mesh gradient | 色の格子、正規化した点の位置、4方向の相対ベジェハンドル、自動ハンドルを Skia CPU で描画する。半透明、18合成モード、文字と罫線、変数・外部部品のテーマ、既存の効果を共用する。隣接面の分割数をそろえ、過剰な曲率・密度・描画量は明示拒否する | `tests/mesh.rs` の独立した色と曲面の数式・実画素・大きな索引の分割・CLI、実 stdio MCP のテーマ変更と PNG。独自の26ノードの作例 `examples/mesh-study.pen` を1040×900で描画・目視確認。省略値と自動ハンドルは独自に明記した規則で、本家との色空間・省略値・曲面の一致、release 性能・各 OS は未検証。[実装と制限](graphics.md#mesh-gradients) |
| CLI | `init`、`inspect`、`variables`、範囲付き `validate`、`apply`、`layout`、`render`、`history`、`restore`、`recover`、`doctor` | `tests/operations_cli.rs`、`tests/queries.rs`、`tests/render.rs` の実プロセス試験 |
| 変数とテーマの取得 | 生の定義と解決値、定義元と末端の値の文書、宣言済みライブラリ、ノード・属性の参照元、祖先からのテーマ、仮の条件指定、変数ごとの診断、依存・問い合わせに束縛したページ | `tests/queries.rs` の継承・上書き・保存済み置換・入れ子の保存済みインスタンス・別名・未解決値・取消・容量上限、実 CLI・stdio MCP。無関係な壊れたルートがあっても対象の変数を読める。本家の全形式との互換、生成ノードの問い合わせ、全条件の列挙や各 OS は未検証・未完成。[取得の仕様](mcp.md#variables-and-themes) |
| 範囲付き検証 | 構造検証と計算済みレイアウトの検証を選び、MCP は計算 ID を診断・計測・画像で共用する。ページ外も含むエラー・警告の集計、依存の変更・失効の拒否、CLI の失敗終了コード | `tests/queries.rs` の1,001件の実診断、後続ページのエラーによる不成立、実 CLI、`tests/mcp.rs` の実 JS・同一計算・PNG の画素・ソース変更・取消。描画なし構成は計算の指定を明示拒否。すべての塗り・browser・shader の実行や見た目はこの検証の成功に含めない。[検証の仕様](mcp.md#validation-scopes) |
| 編集の取消 | CLI の Ctrl+C・MCP の要求トークンを、通常編集、実行時依存の事前条件、ドライラン、部品の探索・コピー・展開、素材公開前、文書・依存のロック待ち、置換直前まで伝える。置換開始後は履歴確定まで完了する | `tests/operations_cli.rs`、`tests/imports.rs`、`tests/mcp.rs`、`tests/script_render.rs` と共通サービスの単体試験。取消後の無保存・次の要求での再開・完了した計算への取消の非伝播を確認。すべての静的計算段階と各 OS の遅延保証は未完成 |
| MCP の基本経路 | 公式 Rust SDK による文書固定の stdio。編集・復元・探索・計測・PNG 画像コンテンツ、計算スナップショットの保持、入力スキーマ、待機中の取消 | `tests/mcp.rs` の実プロセス通信。`2025-11-25` を広告して試験。実 ACP 2種とのセッション作成に加え、`tests/agent_workflow.rs` で両実エージェントの素材登録・編集・レイアウト取得・PNG 取得を確認。全クライアント・プロトコル版の検証ではない |
| ACP ホストと prompt | 公式 Rust SDK 3.5.0 で ACP v1 を交渉し、同じ接続の複数要求、画像 capability、会話の保存・再開、承認、取消、接続エラーと未知通知、NDJSON を扱う。認証方式の通知を正規化し、不一致を送信前に拒否、実行中の変更では取消。公開されたモデル・モードを選択し、文書の登録済み MCP ツールだけを一度ずつ許可する | `tests/agent.rs` と権限の単体試験で、認証の不一致・通知の欠落・実行中変更・個人情報の除外、モデル・モード、セッションとツール ID による承認の範囲を検査。macOS arm64 で Codex ACP 2.2.2 と Claude ACP 0.91.0 の実サブスクリプション生成も成功。実 API/provider、TUI、各 OS、診断と会話管理の全体は未完成 |
| 会話のファイル画像添付 | PNG・JPEG・WebP の正規化と不変保存、実 ACP 画像ブロック、順序と登録 ID、文書に束縛した MCP マニフェスト、素材への取り込み、通常終了での権限破棄 | `tests/attachments.rs` の別文書・未登録 ID・改変・失効、`tests/agent.rs` の疑似プロセスと実 MCP、`tests/agent_workflow.rs` の両実エージェントの画像入力・素材登録・編集・PNG まで。TUI の削除・再送と失敗時保持は追加試験で確認。実クリップボード、保持方針全体、強制終了後の権限回収は未完成 |
| 英語ガイドとスタイル | 独自の英語ガイド 15 本、スタイル 3 種、編集可能な `.pen` 作例と実 API の操作例。CLI・MCP が同じ内容・版・ハッシュを返す | `tests/guidance.rs`、`tests/mcp.rs`。全スタイル作例の計測・描画と、操作例の適用を実行。スクリプト作例は `tests/script_render.rs` で検証。動的な作例の変換と画像確認は下記 |
| プロジェクトの設計ルール | `[design]` の独自ガイド・JSON スタイル・既定スタイルを `project:ID` で公開する。CLI・MCP と CLI／TUI 共通の会話サービスで使用し、ユーザーのローカル承認へ束縛する。指示の変更と作例データの更新を区別する | `tests/project_guidance.rs` の10試験、実 stdio MCP、疑似 ACP とその子の実 MCP、実 CLI。指示変更後の起動前拒否、相対データ保存先、ページ・依存の失効、copy/link の素材の参照元、設定破損時の保存済み文書の読み取りを確認。今回の追加を使った課金付き実生成・物理 TUI の試験は未実施 |
| 要求中に取得した指針の記録 | 所有する MCP がガイド・スタイル・一覧の取得結果の識別情報を同期保存し、共通の会話ホストが要求の終了・取消・失敗時に会話ログへ取り込む。開始時の資料と追加取得を記録し、本文・作例・画像は複製しない | `src/guidance/usage.rs` の並行書き込み・取消・古い要求・破損・容量・symlink/FIFO、実 stdio MCP、疑似 ACP と実 CLI、実 TUI PTY の同じ接続の2要求。ログの最終書き込み失敗時も失敗として通知し下書きを復元する。取得はモデルの理解・適用の証明ではなく、親の異常終了後の回収と保持方針全体は未完成。[形式と上限](agents.md#design-resource-records) |
| TUI の初期実装 | 共通の生きた ACP 接続、プレビュー、ノード検索・対象指定、会話、承認、パン・ズーム、添付の確認・削除、入力保持。Kitty・Sixel の明示指定と Unix の自動検出、halfblocks と外部 PNG 閲覧。コマンドパレットと、履歴のページ取得・確認付き復元 | `src/tui/tests.rs` と `tests/tui.rs`。注入した画像入力から実 ACP の RGBA/alpha、同じ接続の2要求、失敗・再開・承認・取消、完了通知と次の入力の競合を検査。実 PTY の日本語・複数行・リサイズ・端末復元・検出応答なしの最初の入力、Kitty と Sixel の独立復号、チェックポイントの確認・復元・競合後の再取得を確認。対象3環境の実端末・実クリップボード・IME・設定等の保証は未完成。Windows の追加対応は後日に回す。詳細は [TUI](tui.md) |
| 待機時の解放 | JavaScript・ページ・シェーダーを最終実行から各60秒で終了・回収し、画像と計算結果を保持する。MCP・TUI は要求がなくても最も早い期限で起きる。MCP の5分のスナップショットも待機中に解放する | 実ワーカーの期限・再利用・回収・入力検査失敗、実 Chromium の個別期限と画素、MCP の失効後の画像要求・無保存を検査。追加要求なしの実 MCP と実 PTY の60秒試験も成功。macOS arm64 の結果であり、設定・プロセスツリー全体と各 OS・全性能は未完成 |
| 実画像 | 編集可能な `examples/welcome.pen` を CLI で 960×640 PNG に変換し、日本語、余白、ボタンを目視確認した | 作例の変換は `tests/render.rs` でも実行する。本家との品質比較の成果物ではない |
| ソース配布 | ローカル Skia binding patch を含む、再現可能なソースアーカイブの作成・完全性検証・新規ディレクトリーへの展開。ファイルごとの SHA-256、Git の来歴、パス・リンク・容量の検査。MPL 依存の原本ソースとライセンス | `scripts/dist.py` と `scripts/test_dist.py` の12試験。macOS と Linux で成功。`option-ext` の元の registry archive は Cargo.lock の checksum と一致。展開したソースから Linux の実ビルド・試験も確認。署名・バイナリー配布・crate 公開は未完成 |
| CI と依存ポリシー | Rust 1.94 で配布ソースから両構成を試験し、release の mesh と1,000ノードの日本語・部品・画像を描画する構成。SPDX・取得元・RustSec の検査 | `.github/workflows/ci.yml` は actionlint 成功。ローカルの all-features・locked の cargo-deny は3検査とも成功。対象3環境での最新 source の全検証、native 依存の完全な表示・ソース提供・バイナリー配布条件は未完了 |
| Linux release の初期検証 | Debian 12 arm64・Rust 1.94 の実ビルドと CLI。初回ソースの通常354試験・描画なし205試験、条件式修正後の関連32試験・両構成の Clippy。release で mesh と1,000ノードの描画 | 単純な影の文書は画像の参照を早く解放する修正で中央値約2.1秒から約0.31秒になり、PNG は一致した。端末バックエンド変更後の通常354試験・描画なし205試験と両構成の Clippy も成功し、release の mesh と1,000ノードの描画も成功した。計算済みノードの取得とスクリプト診断の追加は、その検証ソースには含まれない。各 OS・全プロセス群・操作応答の性能達成とは扱わない。[条件と数値](performance.md) |

2026-10-23: 上記の macOS arm64 環境で 61 試験と `cargo clippy --all-targets -- -D warnings` が成功した。Skia を除外する構成も継続して確認する。

2026-10-23: MCP、ガイド・スタイル、パスとロックの追加後、`cargo test` の 78 試験と `cargo test --no-default-features` の 52 試験が成功した。両構成の Clippy も警告なしで成功した。Windows・Linux の実行試験を済ませたという意味ではない。

2026-10-23: 部品編集・切り離しの初期実装、継承とライブラリの探索、不変素材の追加後、`cargo test --quiet` の 107 試験と `cargo test --no-default-features --quiet` の 77 試験が成功した。両構成の `cargo clippy --all-targets -- -D warnings` と `cargo fmt --check` も成功。MCP で PNG をアップロードし、原子的な参照設定を経て描画画像の色を検査した。これは全機能・全環境の完成や本家との品質同等性を示すものではない。

2026-10-23: JavaScript の実行・描画を追加し、`cargo test --quiet` の130試験、描画を除外した構成の85試験が成功した。両構成の Clippy と書式検査も成功。実 MCP でコード作成、入力変更、新しい不変コードへの差し替え、実行中の取消、生成画素の変化を確認した。実行時だけ分かるコード・画像の変更で古いプレビューを拒否し、CLI 出力による実行時コードの上書きも拒否する。独自の `examples/script-chart.pen` を960×600 PNGに変換し、日本語・曜日ラベル・棒グラフ・余白を目視確認した。3機能全体、本家との互換性、全 OS、公開品質の完成を示すものではない。

2026-10-23: Chromium 導入・browser・GLSL と実行時依存の編集前条件の初期実装後、既定構成の137試験、描画なしの87試験が成功した。通常テストから分けた実機試験も11件成功した（動的描画9、実 MCP1、固定 ZIP 導入1）。実 MCP の通常試験では、実行時依存を評価中の編集要求の取消・無保存・後続要求への復帰も確認した。macOS arm64 で公式配布元からの実 HTTPS 導入を実行し、独自の `examples/dynamic-panel.pen` を1000×640 PNGへ変換して日本語・Web ページ・GLSL の合成を目視確認した。全機能・全 OS の完成や本家との品質同等性は引き続き未確認である。

2026-10-23: 文字・罫線・画像の crop transform・normal の内側の影を拡張し、既定構成150試験（新しい描画試験13件を含む）、描画なし87試験、実ブラウザーを使う11試験（動的描画10、実 MCP1）が成功した。両構成の Clippy と書式検査も成功。空白と複数の字形 run を扱い、計測と同じ配置から文字の輪郭線を作る。小さい行高の上下にはみ出す字形・装飾を直接の Paragraph 描画と比較し、マスクと出力範囲の切れを修正した。辺別罫線、内外の線、鋭角の miter、画像の回転・せん断・切り抜き、外部部品由来の画像・GLSL 罫線、半透明の内側の影、無効な線の無実行を画素で確認した。独自の `examples/graphics-study.pen` と元画像の編集可能な `.pen` を追加し、1000×760 PNGを目視確認した。全描画機能・本家との互換性・各 OS の完成を示すものではない。

2026-10-23: 背景ぼかしと、色・背景を置き換える範囲を分ける合成を追加した。既定構成163試験、描画なし87試験、実ブラウザー12試験（動的描画11、実 MCP1）が成功し、両構成の Clippy と書式検査も成功した。背景の alpha、複数の親の透明度、角丸・回転・クリップ、後続の影、GLSL の効果順序、拡大時の領域・ノード切り出し、CLI と MCP の実画像を確認した。CLI の `--region X Y WIDTH HEIGHT` は負の座標も扱う。画面外の browser・GLSL を除外し、MCP のページ取得 ID は対象ノードを指定して検証する。明示的な作業画像に共有予約上限を設け、失敗後の解放も試験した。独自の `examples/backdrop-panel.pen` を1000×680 PNGへ変換し、背景だけがぼけ、日本語の内容と線が鮮明であることを目視確認した。英語の `effects` ガイドを含む14本を公開インターフェースから取得できる。35ノードのデバッグ版を新規 CLI プロセスで3回測った予備値は約1.91〜2.04秒、peak RSS 約101 MiBで、基準の1,000ノード・release版・各 OS の性能保証ではない。条件と残る範囲は [性能の観測](performance.md) に記録した。全機能・公開品質の完成は引き続き未確認である。

2026-10-23: ACP v1 の共通ホスト、CLI `prompt`、会話記録、ファイル画像添付と登録 ID の初期実装を追加した。既定構成180試験、描画なし103試験が成功し、両構成の Clippy と書式検査も成功した。通信フレーム検査と添付の実バイト数による上限の調整後にも、関連する17試験（描画なし16試験）を再確認した。疑似 ACP プロセスで複数要求・画像 capability・再開・未知通知・承認拒否・取消・応答しないプロセスの終了を検査し、実 MCP で画像の受け渡しから素材登録・編集・PNG 返却まで確認した。CLI の SIGINT 後に保存済みの編集・履歴・実リビジョンが残ることも試験した。macOS arm64 で Codex ACP 2.2.2 と Claude ACP 0.91.0 の実接続試験2件が成功した。準備中に認証状態などの通知が挟まる実際の順序も扱う。この2件では LLM へプロンプトを送っておらず、両実エージェントの生成全体・課金経路・TUI・各 OS の完了を意味しない。構成・操作・制限は [AI 連携の実装](agents.md) に記録した。

2026-10-23: 認証方式の read-only 通知を正規化し、サブスクリプションと API/provider の指定との不一致を送信前に拒否、実行中の変更では取消するようにした。認証通知からメール・トークン・任意の vendor 情報をログへ残さず、再接続では以前の方式を確認済みとして流用しない。ACP の公開済みモデル・モードの設定と応答確認、セッション・ツール名または MCP の相関 ID による文書操作だけの一度限りの許可も追加した。既定構成188試験、描画なし110試験が成功し、両構成の Clippy と書式検査も成功。実アダプター2種ではモデル・モードの設定を LLM を呼ばずに確認し、別の実生成試験では画像の参照、添付 ID の素材登録、編集保存、レイアウト・PNG 取得、保存文書の再描画が成功した。Codex は `read-only`、Claude は `default` モードで文書固定 MCP だけを許可した。両方の PNG で日本語・参照画像・保存ボタンを目視確認した。初回の Claude で画像モードの誤りと部分的な paint 更新が発生したため、公式スキーマに照らして画像の操作例・プロンプト文脈・診断を改善し、再試験では同じエラーが出なかった。試験用 PID ファイルの公開前の読み取り競合も原子的な置き換えで修正した。Codex の生成結果は、画像 URI を同梱素材へ変更した編集可能な `examples/agent-settings.pen` として追加し、640×360、13ノード、診断なしで再描画した。これは実 API/provider、TUI、全形式・OS、性能・公開品質、総合的なデザイン品質の同等性まで検証したという意味ではない。

2026-10-23: CLI と TUI の会話・添付・承認を共通サービスへ移し、要求間で ACP 接続を維持する TUI の初期実装を追加した。画像の確認・削除・画像だけの要求、失敗・取消後の保持、会話の再開、パン・ズームとノード検索・対象指定を扱う。クリップボードとファイル入力は取消可能な別プロセスにし、Unix では子孫も終了・回収する。端末検出に使うライブラリのタイムアウト後に stdin 読み取りが残る実装を避け、350 ms の期限付き検出へ置き換えた。モーダルを閉じる再描画でも、イベント読み取りと競合するカーソル位置問い合わせを避ける。完了記録と通知の間に次の入力を消す競合も修正し、再現試験を追加した。実 PTY で日本語・複数行・2要求・リサイズ・端末復元、Kitty と Sixel の明示指定・自動検出・独立した画素復号、検出応答がない場合の最初の入力を確認した。全体の既定構成198試験、描画なし110試験、両構成の Clippy と書式検査が成功。共通会話への変更後にも、実 Codex・Claude の画像付きサブスクリプション生成2件が成功し、両 PNG の日本語・画像・保存ボタンを目視確認した。これは実クリップボード、実端末・IME、動的コンテンツの TUI 統合試験、Windows/Linux、性能・配布・公開品質の完成を示すものではない。操作と残る範囲は [TUI](tui.md) に記録した。

2026-10-23: TUI の Ctrl+P コマンドパレットと `/history` を追加し、50件ごとの履歴、古い・新しいページへの移動、復元前の確認、一覧への取消、明示的な再取得、共通サービスによる新しい履歴としての復元を実装した。候補の取得・復元は非同期にし、下書きと添付を保持する。候補取得後の文書・画像依存の変更は競合として停止し、期待リビジョンを自動更新して再試行しない。文書・依存のロック待ちと保存確定前の取消を共通化し、CLI の SIGINT、MCP の取消通知、TUI の Ctrl+C を試験した。保存確定に入った処理は履歴確定まで完了させる。終了時はアダプターの終了待ちより前に各ワーカーの取消を要求する。通常ファイルの検査では Unix の nonblocking open を使い、文書の FIFO が開くだけで停止しないことも確認した。実 PTY では確認だけで保存されないこと、Esc の戻り、復元後の下書きと履歴、並行する CLI 編集が明示的な再取得・再確認まで保護されることを確認した。既定構成207試験、描画なし115試験、両構成の Clippy と書式検査が成功。設定・素材カタログ、動的内容の TUI 統合試験、実クリップボード・各端末・各 OS、性能・配布・公開品質の全体は引き続き未完成。

2026-10-23: Lucide 1.57.0 のベクター形状を同梱し、Feather 4.29.2 を固定版の明示導入へ対応した。CLI・MCP で名前・別名・タグ・版・ハッシュ・ライセンス・導入状態を取得し、英語の `icons` ガイドを追加した。取得元・正規化データ・完全なライセンス本文を固定し、`icons license` で未導入でも本文を読める。全2,441名の形状、透明な穴、交差部分の半透明、縦横比、勾配、変数・部品・生成ノードの解決、依存の改変と導入後の無効化を試験した。元 SVG と実 Chromium の代表比較1件、実 HTTPS 導入、実 Codex・Claude のサブスクリプション生成2件も成功した。独自の `examples/icon-study.pen` を960×700 PNGへ変換し、日本語ラベル・12種類のアイコン・勾配・影を目視確認した。オフライン再生成したカタログ・形状・両ライセンスは同梱リソースとバイト単位で一致した。全体の既定構成217試験、描画なし120試験が成功し、その後のライセンス取得の追加と Clippy 指摘の修正後にも関連27試験（描画なし5、描画のみ7）を確認した。両構成の Clippy と書式検査が成功。Phosphor・Material Symbols・重みのあるパックの解決、全描画互換性、各 OS、性能・配布・公開品質の全体は引き続き未完成。[導入と検証](icons.md)

2026-10-23: `--json` の指定がある CLI の引数解析エラーも `INVALID_ARGUMENT` の構造化結果として stdout に返し、終了コード2を維持するようにした。未知のコマンド・オプション、不正な数値、必須引数の不足、サブコマンドの前後の指定、色の強制時の ANSI 非混入を実プロセスで検査した。help・version は終了コード0の通常テキストを維持し、`--` 以降の `--json` を出力設定として解釈しない。更新後の全体は既定構成219試験、描画なし122試験が成功し、両構成の Clippy と書式検査も成功した。CLI 全体の設定・診断・公開準備が完了したという意味ではない。

2026-10-23: JavaScript・ページ・シェーダーの子プロセスを、最終実行から各60秒の待機で終了・回収するようにした。MCP は最も早い期限か要求が届くまで待ち、TUI は条件変数の待ち時間へ期限を反映する。専用の回収スレッドや定期的な全描画は追加しない。キャッシュのヒットは期限を延長せず、画像と未失効の計算結果を保持する。MCP の作成から5分のスナップショットも待機中に解放する。実 JavaScript ワーカーで期限直前・到達、再利用、回収、再起動と入力検査だけの失敗で期限が延びないことを確認した。実 Chromium で2プロセスの個別終了・回収、画像の保持・再取得と同じ画素を確認した。通常試験ではスナップショットの時刻を進め、失効後の描画要求が拒否され文書が保存されないことも検査した。別の実時間試験では MCP へ追加要求を送らずに60秒待ち、3子プロセスの終了と回収、画像の再利用、再計算と明示更新による復帰を確認した。実 PTY でも JavaScript・ページ・GLSL の赤・緑・青の Kitty RGBA を独立復号し、実待機後の3子プロセスの回収、プレビューと日本語下書きの保持、明示更新での新しい3プロセスと無保存を確認した。最終版の実 MCP は約62.81秒、実 PTY は約65.77秒で成功した。既定構成221試験、描画なし123試験、両構成の Clippy と書式検査が成功。各 OS・物理端末・プロセスツリー全体・設定・性能目標全体の完成は引き続き未確認。[動的ランタイムの待機時解放](dynamic-content.md#待機時のプロセス解放)

2026-10-23: フォントをファミリー単位からフェイス単位の登録へ改め、TTF・OTF・TTC・OTC、静的な太さ・斜体の選択、補助面の字形を含む基本のフォールバックを追加した。macOS の Skia CoreText が非ゼロのコレクション番号を直接受け付けないため、テーブルの範囲を検証し、オフセットとチェックサムを再構築した単独 SFNT をメモリ上で読み込む。元ファイルは変更せず、元のハッシュ・番号と実際に渡すフェイスデータのハッシュを区別する。通常の単独入力は再構築用の余分なコピーをしない。組版後の未解決字形を検査し、実際の字形 run から使用フェイス・軸座標・合成 bold/italic を記録する。使用しなかった主書体やフォールバックを使用済みとして列挙しない。`fonts list` と MCP の一覧、`fonts license` の埋め込み OFL 全文を追加し、入力・保持データの容量とフェイス数を制限した。元のフォントへの後置バイトの追加でも、MCP のスナップショットは競合として拒否する。独自の7バイナリとマニフェストは標準 Python だけでバイト単位に再現した。コレクションと単独版で PNG と派生ハッシュが一致し、CFF・静的スタイル・合成書体・日本語と補助面・欠損診断・不正ヘッダーとテーブル・総量上限・CLI・MCP・FIFO 非待機を確認した。実 Arial の通常・太字・斜体でも元データと PostScript 名の選択を確認した。全体の既定構成231試験、描画なし123試験、両構成の Clippy と書式検査が成功。大文字・小文字が異なる同名ファミリーのスタイル統合、合成書体、長いフォント名の診断の検査を追加した最終版でも、既定構成231試験と Clippy が成功した。`examples/welcome.pen` を960×640 PNGへ再描画し、日本語・混在文・見出し・ボタン、診断なしを目視確認した。プロジェクト設定、フォント検索、利用者向けの任意フォールバック設定、全可変軸・絵文字・遅延読み込み、各 OS と公開品質全体は引き続き未完成。[フォントの実装と制約](fonts.md)

2026-10-23: `lazypencil.toml` v1 の共通設定と `config init/show/schema`、`fonts list --file` を追加した。設定元の不変バイト列からファイル・検索先・既定書体・代替列・言語を読み、CLI・MCP・TUI で同じ優先順と組版を使う。未知のキーや版・容量・型を拒否し、エラーへ設定本文を混入させない。近い設定の追加、検索先の増減、シンボリックリンクの向きの変更も依存競合として検出する。存在しない設定候補への共有ロックと原子的な初期化の排他ロックを合わせ、待機中の取消でファイルを作らない。保存済み要求の再送は設定が後から壊れても二重編集せず以前の結果を返す。実 MCP で設定だけによる書体・言語・PNG と古い計算の拒否、実 PTY でフォント変更後の Kitty 画素の自動更新と日本語下書きの保持を確認した。既定構成242試験、描画なし132試験、描画のみの関連44試験、両構成の Clippy が成功。設定全体、実行ファイル設定の信頼、利用者全体の設定・フォントの遅延読み込み・各 OS・公開品質の完成は引き続き未確認。[設定の実装と制約](configuration.md)

2026-10-23: `detach_instance` をローカルの保存済みインスタンスから、宣言済み外部ライブラリと継承したインスタンスの対象へ拡張した。親・元定義・兄弟を変更せず、定義チェーンと外側の上書きを一段だけ実体化し、折り込み済みの深い上書きを除去する。元文書を含む一時 ID で同名の所有ノードを区別し、新しい保存 ID と内部参照を割り当てる。入れ子の参照は維持し、外部の変数・画像・shader の直接参照は元の名前空間・ファイルへ結び付ける。必要な既定テーマの差を明示し、相対 URI は `/` 区切りで保存する。存在しないスロット候補も配置の制約にしない。保存前にはプロパティ・実効テーマ・子構造・入れ子の参照境界・正規化した素材パスを比較し、意味が変わる場合はバッチを拒否する。実 CLI・MCP、要求の再送、依存競合と無保存、外側の上書き、同名 ID を持つ部品と追加スロット内容の別々の内部参照を確認した。外部・継承それぞれの切り離し前後で PNG と画像依存一覧が一致した。最終版の既定構成255試験、描画なし144試験、TUI を含まない描画構成の関連31試験、両構成の Clippy と書式検査が成功。外部 script の生成参照の名前空間、変数による shader 画像の参照元など、移動後の意味を保てないケースは引き続き明示的に拒否する。部品の link/copy 取り込み、全互換性・各 OS・性能・公開準備は未完成。[操作の実装と制約](mcp.md#components-and-inherited-nodes)

2026-10-23: `import_component` の link と静的依存の copy を共通編集バッチへ追加した。ローカル定義、宣言済みライブラリ、許可領域の明示 `.pen`、同梱スタイルを選び、link は参照を登録・維持し、copy は必要な定義・変数の別名・テーマ・画像・直接指定した shader 素材を取り込む。元文書ごとの ID を区別して参照・上書きの経路を割り当て直し、定義を非表示フレームへ保存して配置したインスタンスだけを表示する。名前の衝突は既定で拒否し、明示的な名前変更・型を確認した変数の再利用・テーマや独立した定義の再利用を扱う。コピー元が利用側を import している場合も、保存する依存グラフへ逆向きの辺を追加しない。素材はメモリ上で準備し、ドライランと後続操作の失敗ではファイル・履歴を作らない。結果から除去される元文書と未使用だった画像も保存直前まで検査・共有ロックの対象にし、ライブラリ・素材のリンク先の付け替えは同一バイト列でも競合にする。重複した読み取り状態を保持し直さず、準備した入力と出力をそれぞれ128 MiBへ制限する。取り込みとアップロードは同じ原子的な素材保存を使い、保存先の親・管理ディレクトリの正規化した実パスを確認する。既存ライブラリのコメント・URI のエスケープ・数値表記を保持することも試験した。

実 CLI・MCP、元ライブラリ削除後の描画・要求の再送、変数・部品・テーマの衝突、依存・上書き・画像の閉包、同名 ID と名前空間を含む ID、素材の改変・リンク先の変更・許可領域の逸脱・保存先の付け替えを確認した。link/copy の PNG は一致した。MCP が受け取れない大きさの編集結果は、ファイルや文書の公開前に拒否する。英語の部品ガイドに実行可能な取り込み例を追加し、3スタイルすべての再利用部品も取り込んだ。既定構成278試験、描画なし166試験、TUI を含まない描画構成の関連48試験、両構成の Clippy と書式検査が成功。別の実連携2件（計155.54秒）では Codex・Claude のサブスクリプション経路で、実画像の入力、スタイル取得、同梱ボタンの copy と編集、Lucide の配置、画像素材の登録、計測と PNG 確認まで成功した。両方でコピーした部品1件と表示対象のアイコン2件を確認し、レイアウト診断は0件。保存成果物を明示した再検証でも、表示対象に残るコピー済み部品と、元の生成 PNG とのバイト一致を確認した。これは本家との美的品質の同等性の証明ではない。script のコピーと外部切り離し、文字列変数による shader 画像の参照元の維持、含まれる再利用定義の再利用、全互換性・各 OS・性能・公開準備は未完成。[取り込みの仕様と制約](mcp.md#importing-components)、[実連携と成果物の再検証](agents.md#verification)

2026-10-23: 通常の編集が要求の取消を保存処理へ伝えていなかった経路を修正した。CLI と MCP は同じトークンを、ドライラン、実行時依存の再評価前の要求再送チェック、編集バッチ、部品の依存探索・コピー・展開、読み取り元の再検査、素材公開前、文書と依存のロック待ち、文書置換直前まで渡す。完了した計算スナップショットからは要求のトークンを外し、後の取消で再利用を壊さない。取り込み準備後の取消で文書・素材・履歴を作らず、新しい要求で再開できることを確認した。別の実 CLI 試験では、画像の公開後に元ライブラリのロックで待機させ、SIGINT で元文書・履歴を変更せず終了し、残った完全な画像を次の編集で再利用した。実 MCP では文書ロックを保持したまま通常編集と実行時事前条件付き編集を取り消し、後続の文書取得へ復帰した。既定構成283試験、描画なし170試験、両構成の Clippy、書式検査が成功した。既定構成の23件と描画なし構成の3件の opt-in 試験は今回再実行していない。取消は協調的で、単独のパース・ハッシュ・ファイル書き込みが終わるまで次の確認を待つ場合がある。文書置換開始後は従来どおり履歴確定まで完了させる。静的レイアウト・描画全体の取消、各 OS の遅延保証、全機能・性能・公開準備は未完成。[取消と再送](mcp.md#snapshots-limits-and-cancellation)

2026-10-23: 外部・継承したスクリプトの切り離しに、生成物と参照入力の解決元を保持する処理を追加した。JS ファイルを元の位置へリンクしたまま、`metadata.lazypencil.script_scope` v1 に元文書の宣言済みライブラリ別名と、必要な場合だけ入力の値のハッシュ・指定元を記録する。コードの書き換えや生成ノードへの静的な置き換えは行わない。切り離しの構造比較でも実行元・入力元の文書を別々に検査する。実ワーカーで、同名の部品・変数・画像が利用側にもあるケース、既定と明示の ref 入力、テーマ、文字列入力と組版、入れ子のスクリプト、別文書からの再取り込みと再切り離し、同じ JSON 値による入力・コードのインスタンス上書きを確認した。切り離し前後の PNG と実行時依存が一致し、実 stdio MCP の新しい計算と画像でも一致した。既存メタデータを保持し、未知の版・形式・未宣言の参照元は拒否する。既定構成289試験、描画なし171試験、両構成の Clippy と書式検査が成功。英語ガイドの最終修正後にも関連28試験と描画なしのガイド5試験が成功し、既定バイナリから新しい本文を取得した。既定構成の23件、描画なしの3件の opt-in 試験は再実行していない。これは lazypencil 独自のメタデータ拡張であり、本家への往復描画互換と独立した script コピーは未完成・未検証である。各 OS・性能・公開準備全体も引き続き未完成。[参照元の仕様](dynamic-content.md#外部スクリプトの切り離しと参照元)

2026-10-23: shader の画像 uniform に変数を使う場合、別名とテーマ条件をたどり、最後に値を定義した文書からパスを解決するようにした。通常の文字列の用途では値を書き換えず、塗り・線・エフェクトの画像の参照元だけを保持する。切り離し後も変数への参照が残り、その後の値の編集が文字列と画像へ反映される。追加のメタデータは使わない。解決時にはパスを結び付けるだけにし、実際に使う素材の取得・描画時に正規化・許可領域・依存を検査する。非表示の親・ノードや無効な塗り・線・効果にある欠損・非ローカル URI を開かず、再有効化した場合はエラーにすることも確認した。これらの非表示・無効なケースでも、外部ランタイムなしの PNG 出力が成功した。実 stdio MCP では画像依存の取得元と切り離し後の参照を検査した。既定構成294試験、描画なし176試験、両構成の Clippy と書式検査が成功。別途、macOS arm64 の実 Chromium／SwiftShader 1試験が成功し、明暗両テーマの画素と切り離し前後の PNG 一致を確認した。最終版の実画素試験は2.31秒。通常試験で無視した24件のうち、この新しい1件を個別実行し、残る23件と描画なしの3件の opt-in 試験は今回再実行していない。本家の画像変数の参照元の規則との互換比較、および文字列と画像の変更連動を維持する独立した部品コピーは未完成である。各 OS・性能・公開準備全体も引き続き未完成。[画像変数の参照元](dynamic-content.md#変数で指定した画像の参照元)

2026-10-23: 宣言済みローカル画像・JS・GLSL・shader 画像入力の欠損を、参照元・予定する実パス・用途とともに依存へ記録し、通常の CLI・MCP 編集から修復できるようにした。部分修復・画像とコードの差し替え・削除・ドライラン・要求再送・チェックポイント復元を確認した。欠損が途中で解消した場合やシンボリックリンクの参照先が変わった場合は、元文書・履歴を変更せず競合にする。同じパスを複数用途で使う場合を保持し、`./` の正規化と通常の親ディレクトリ作成だけでは欠損依存の同一性を変えない。既知の欠損がある検証・計測・描画は `ASSET_NOT_FOUND` とし、`assets_complete` を描画成功の保証として使わない。修復後の JS の実ワーカーと実画素、実 CLI、実 stdio MCP の画像差し替え・PNG 返却も確認した。会話のロックについては、複製した記述子が残っても所有者の破棄で明示解放するようにし、修正前の失敗と修正後の再開・排他を回帰テストで確認した。通常構成312試験、描画なし193試験（`--test-threads=2`）、両構成の Clippy と書式検査が成功した。並列ビルド中の描画なしの試行では疑似 ACP 接続5件が失敗し、ビルドと分離した再試験が成功した。高負荷時の診断・試験安定性は別途確認する。今回、通常構成の24件・描画なしの3件の opt-in 試験は再実行していない。欠損ライブラリ・設定フォント・実行時生成素材などの修復、各 OS・性能・公開準備全体は未完成。[修復の契約と制約](mcp.md#repairing-missing-assets)

2026-10-23: `get_variables` に定義・解決値・定義元と値の元の取得、宣言済みライブラリ、実際の祖先テーマ、インスタンス上書きの属性の参照元、取得時だけのテーマ指定、変数ごとの失敗診断とページ取得を追加した。CLI の `variables` も同じサービスを使う。保存済み置換ノードと、その中の保存済みインスタンスを直接指定しても実効ツリーのテーマを維持する。無関係な壊れたルートがあっても選択した正常なノードの値を取得でき、全体の解決失敗は別途返す。単一の変数を読む際に全定義を複製せず、ページの同一性から期待リビジョン指定の有無を除いて、実際の内容と問い合わせ条件で束縛する。

同日: `validate_document` に `structure` と `layout` の範囲指定を追加した。計算の検証は script・文字・配置の実処理を使い、MCP では診断・`get_layout`・PNG が同じ計算 ID と実行時依存を使う。1,001件の実診断をページ分割し、ページ外のエラーを含む集計、依存変更・不明な計算 ID の拒否、JS 実行の取消と復帰を確認した。CLI は計算を含む検証と診断エラー時の終了コード4を扱う。通常構成327試験、描画なし205試験、両構成の Clippy と書式検査が成功した（ビルドの同時実行1、試験の同時実行2）。同梱作例で変数の取得と計算の検証が成功し、エラー・警告は0件だった。英語ガイドの新しい本文とハッシュも実バイナリから取得した。通常構成24件・描画なし3件の opt-in 試験は今回再実行していない。`complete` は指定範囲の検証の完了であり、全描画機能・見た目・本家互換・各 OS の完成を示さない。生成ノードの詳細取得、残る描画・設定・素材・AI 接続・性能・配布と公開準備は引き続き未完成。[変数の取得](mcp.md#variables-and-themes)、[検証の範囲](mcp.md#validation-scopes)

2026-10-23: 塗りと内側・外側の影に18種の合成モードを実装した。半透明の背景を二重に合成しないよう、影の色と背景を置き換える範囲を分ける。linearBurn・linearDodge も色の計算後に通常の alpha 合成を行う。回転した中間画像を出力のピクセル格子へそろえ、切り出し・拡大で端の alpha が変わる問題も修正した。背景を必要としない子では親の全面取得を省き、入れ子の描画で画像の読み込みキャッシュを共用する。不正なモードの診断は入力文字列の表示量を制限し、ノード・属性・参照元と使用可能な値を返す。

独立した計算式と実 PNG の画素で、18モード、半透明、親の透明度、クリップ、効果の順序、回転・拡大、領域とノードの切り出し、画像・勾配・罫線を確認した。実 stdio MCP ではインスタンスのテーマ変更と合成画素を確認した。独自の165ノードの作例 `examples/shadow-blends.pen` は1040×1000で描画・目視確認し、レイアウト診断・外部素材・動的ランタイムは0件だった。通常構成340試験、描画なし205試験、両構成の Clippy と書式検査が成功した。静的検査で試験中の不要な型変換を除いた後、影の関連13試験も再実行して成功した。この13件には、OS のサンドボックスを有効にした実 Chromium／SwiftShader で GLSL の前後の影を比較する opt-in 試験1件を含む。全体試験では通常構成25件・描画なし3件の opt-in 試験を除外しており、今回再実行した opt-in はこの1件だけである。本家の色空間・画素との一致や美的品質の同等性は未検証。固定した単純な1,000ノードの debug 観測は約1.5秒・145 MiBだったが、release・代表的な複合文書・全プロセス・各 OS の性能達成を示さない。残る機能と公開準備全体は未完成。[合成の実装](graphics.md#shadow-and-paint-blending)、[性能の条件](performance.md)

2026-10-23: `mesh_gradient` をネイティブの塗りとして実装した。行優先の色と点、正規化した位置、4方向の相対ベジェハンドル、隣接点からの自動ハンドルを Coons 曲面として描く。面ごとの幾何と色の誤差から分割数を決め、共通境界の分割数と評価をそろえる。大きな面は16 bit の索引を使う帯へ分け、重複した境界頂点も生成数上限に含める。色は alpha を含む premultiplied sRGB の補間とし、通常の形状・文字・罫線、18合成モード、親の透明度と効果を共用する。有限のハンドル値でも拡大後に表現範囲を超える場合は Skia へ渡す前に拒否する。格子・分割数・総頂点数・ラスターと共有作業量の上限を設け、分割の過剰は `RESOURCE_LIMIT` と終了コード4で返す。色・点・ハンドルの不正と無効な塗り、CLI の失敗時の画像未保存も確認した。

独立した双線形色・alpha の数式と、解析的に逆算できる曲面で実 PNG の画素を比較した。点の移動、自動と明示ハンドル、半透明の面の境界と大きな索引の帯の内部、全合成モード、文字と罫線、回転・拡大・切り出しと背景効果を検査した。実 JS ワーカーの生成物は保存済みノードと同じ PNG になった。実 stdio MCP では外部部品のテーマ変更と保持した計算からの画素を確認し、元ライブラリは変更されなかった。実 Chromium／SwiftShader の OS サンドボックスを有効にした試験では、背景ぼかし・影の前後の GLSL が native mesh の色と alpha を保った。独自作例 `examples/mesh-study.pen` は26ノード・1040×900で描画・目視確認し、文字のあふれと文字色を修正した最終版の診断・外部素材・動的取得は0件だった。英語ガイドと能力表示も更新した。省略値、自動ハンドル、色空間、交差した面の扱いは [現在の規則と制限](graphics.md#mesh-gradients) に明記し、本家との一致とは扱わない。

全体試験の途中で TUI のフォントプレビュー試験が一度タイムアウトした。保存した出力を独立に復号・再生し、完成した Kitty 画像と画面上の診断がある一方、端末の差分更新で raw の `diagnostics` が `di` と `gnostics` 等へ分割されることを再現した。試験は VT 画面の実際の状態行を確認するよう修正し、パスに同じ単語が含まれる場合の誤成立も防いだ。待機上限と画像・設定変更・下書きの検査は維持した。最終 Rust コードとこの修正で、通常構成354試験、描画なし205試験、両構成の Clippy、書式・差分検査が成功した。通常構成26件・描画なし3件の opt-in 試験は全体試験で除外し、別途そのうち実 Chromium の mesh/GLSL 試験1件を実行して成功した。他の opt-in は今回再実行していない。作例の文字色を最後に調整した後も、その作例の試験と実 CLI 描画を再実行した。本家互換性・美的品質の比較、各 OS、release 性能、全機能と配布・公開準備は引き続き未完成。[TUI の検証](tui.md#verification-and-remaining-scope)

2026-10-23: 配布用ソースの作成・検証・展開を実装した。Cargo の manifest・lock とローカル `skia-bindings` patch、フォント・アイコン・既存ライセンス、作例・試験・文書・CI を含め、取得済み Skia ソースとビルドキャッシュは除く。マニフェストの SHA-256 とサイズ・モード・対象ファイルを照合してから展開する。上書き、パストラバーサル、リンク、大小文字・Unicode の衝突、圧縮展開量の過剰を拒否する。11試験は macOS と Linux で成功し、実際の一時 Git リポジトリーで clean なコミットの記録、変更済み・未追跡ソースの拒否、明示した開発用スナップショットも確認した。Git が無視するローカルデータが収集される問題は再現試験で確認し、release は追跡済み、開発用は追跡済みと無視されていない未追跡ファイルに限定して修正した。Git がない明示的な開発用取得だけは、定めたディレクトリーをファイルシステムから収集する。本プロジェクトのコミットや push は行っていない。来歴と完全性の記録は署名による真正性の証明ではなく、ソース配布は依存全体のオフライン vendor bundle でもない。

同日: 検証済みソースアーカイブから独立した Debian 12 arm64・Rust 1.94.0 の環境でビルドし、通常354試験・描画なし205試験が成功した。通常26件・描画なし3件の opt-in 試験はこの全体実行で除外した。実 PTY、日本語入力、Kitty／Sixel、フォント設定の更新、実 MCP と QuickJS ワーカーを含む。最初の静的検査でコンテナーの Clippy 不足が判明し、Dockerfile に明示導入を追加した。Rust 1.94 の Clippy が指摘した条件式は等価な `is_none_or` へ変更し、その後は両構成の Clippy、書式、文書保存・CLI の関連32試験が成功した。macOS でも変更後の関連32試験と両構成の Clippy が成功した。全体試験に使ったアーカイブの SHA-256 は `1f6253b3247053a5057deaefb3a97eeb3181987b3bc01ea6ffa52d7800140276`、条件式修正後のものは `225c47b10ef5f404de64d1fbc60d510786d557b54327420875fc77232abbfbba`。

同日: 条件式修正後のソースで thin LTO・codegen-units 1 の release ビルドと実 CLI を確認した。mesh の26ノード・1040×900 PNG は診断・外部素材・動的取得0件で、実ファイルのハッシュは返却された値と一致した。日本語・文字と罫線の mesh を目視確認した。Skia の C++ には GCC が無視する Clang 属性の upstream 警告が残る。新しい基準文書は131保存ノードを部品参照から1,000ノードへ展開し、日本語・English・変数・ローカル画像・角丸・影を1920×1080へ描画する。修正した fixture の各3回の export は診断0件、画像ハッシュも一致した。ダッシュボードの release 観測は約1秒・79〜81 MiB、別の単純な影の文書は約2.1〜2.2秒・102 MiBだった。後者の2秒超過は未達として記録し、目標を変更していない。CI の release smoke にも新しい文書の実描画と画像・診断・部品展開数の検査を接続し、Linux の実バイナリーで成功した。[計測の全条件](performance.md)

同日: GitHub Actions に Linux x64/arm64、macOS Intel/arm64、Windows x64 の配布ソースからの試験・静的検査・release smoke、依存監査、検証後のソース artifact を構成した。公式 Action はコミット SHA で固定し、actionlint は成功した。GitHub 上のジョブはまだ実行していない。cargo-deny 0.20.2 の all-features・locked で RustSec・SPDX・取得元の3検査は成功した。これは native Skia・QuickJS・標準ライブラリー等を含む再配布表示の完成ではない。cargo-about 0.9.2 の試行では、成功終了でも著作権者のプレースホルダーや原文取得失敗後の合成本文が残ることを確認し、そのまま配布用表示へ採用していない。MPL-2.0 の `option-ext` の版付き例外、ソースの入手方法、元の著作権・追加表示も公開前の確認対象として維持する。

同日: `option-ext` 0.2.0 の原本 registry archive、同じライセンス本文、来歴と source notice を `resources/licenses` に追加した。Cargo.lock の checksum、元の package manifest のライセンス、アーカイブ内と単独のライセンスのバイト一致を試験する。新しい検査を含む配布関連12試験は macOS と Linux で成功した。プロジェクト本体の MIT は変更しない。バイナリーを配布する際も、この原本ソース・表示を保持する必要があり、他の Rust・native 依存の表示の監査や配布物への組み込みは引き続き未完成。[原本の表示](../resources/licenses/SOURCE-NOTICES.txt)

同日: 背景合成の子・罫線の読み取り用 renderer が、親 coverage の画像参照を親への描画まで保持していた問題を修正した。読み取りが終わった時点で解放し、兄弟ごとの全面コピーを避ける。Linux release の単純な1,000ノード文書は各3回の中央値で2104.281 msから308.267 msになり、もう一方の日本語・部品・画像の文書を含めて PNG は修正前とバイト一致した。全6回の画像出力は診断0件で成功した。ダッシュボード側は約1.19〜1.91秒と変動し、同じ高速化を主張しない。この計測は後述の端末バックエンド変更前の実行ファイルであり、最終 release 全体の性能確認ではない。

同日: 描画変更後の macOS は通常354試験が成功したが、Linux は353成功・1失敗だった。失敗はリサイズ直後の F1 入力で、15秒の期限後も3バイトが未読のまま残り、診断用の別キーを送ると初めて F1 が処理されることを再現した。Unix の Crossterm は `use-dev-tty` の poll バックエンドへ切り替え、未読の入力通知が次の入力を待たず処理されるようにした。Windows は従来のバックエンドのままであることを feature graph で確認した。lock の追加は `filedescriptor` 0.8.3 の1件で、SPDX・RustSec・取得元の監査は成功した。既存の版付き依存を一括更新していない。

再現試験はリサイズと F1 の開閉を8回続け、各回の表示を確認する。復旧用の入力や待機期限の延長は加えない。修正前は2回目で同じ3バイトの停滞を再現し、修正後の Linux は成功した。macOS でも Kitty／Sixel・自動検出・復元・フォントを含む全9ケースが成功した。終了時の端末状態では、macOS が canonical 復元時に加える一時的な PENDIN だけが違うことを、lazypencil を使わない Python の raw/restore 往復でも再現した。設定値の比較に加え、終了後の行入力・CR/LF 変換・エコーを実際に確認するようにした。失敗時は生成した端末ログと出力を保持する。[試験とバックエンドの根拠](tui.md#verification-and-remaining-scope)

端末変更後の macOS arm64・Rust 1.99.1 では、通常354試験・描画なし205試験、両構成の Clippy、書式検査が成功した。全体では通常26件・描画なし3件の opt-in を除外し、別途そのうち実 Chromium／SwiftShader の影・mesh の2件と、実 TUI の60秒待機・JS／ページ／GLSL 子プロセス回収・画像保持・更新の1件が成功した。後者は約68.38秒だった。課金を伴う実エージェント生成や他の opt-in を今回再実行したという意味ではない。

端末変更を含む検証用ソースは254ファイル・SHA-256 `10f5cad723407968faf35829260cbbe1989baa9709d711f97182cfd3539b82aa` の開発用アーカイブとして検証した。Linux では、このソースの通常354試験・描画なし205試験、両構成の Clippy、release ビルドと描画が終了コード0で成功した。mesh は1040×900・26ノード、ダッシュボードは1920×1080・展開後1,000ノードで診断0件だった。後者の PNG SHA-256 は `eac15be8a871dca4cafa59de7a157083114bc02e3847a5f1e1bdf8ac18060046` で、以前の描画結果と一致した。計算済みノードの取得とスクリプト診断の追加は含まれない。検証途中で Docker のイメージと作業ボリュームの消失を確認したため、保存したアーカイブから作り直し、ソース・Cargo キャッシュ・結果をホストの専用キャッシュへ保持する構成にした。この固定ソースの成功を、その後の変更や各 OS 全体の成功へ読み替えない。

同日: 計算済み・生成ノードの詳細取得を追加した。MCP の `read_nodes` は `calculated: true` で通常の JS・レイアウトを明示実行し、`snapshot_id` で保持した同じ計算をコードの再実行なしに読む。解決済み属性と属性の出所、テーマ、生成元、同じ計算のレイアウトを返す。継承アドレスと不透明な生成 ID、検索・深さ・ページ指定を扱う。計算 ID と検索条件へページを束縛し、応答は借用データの直列化で4 MiB以内と確認してから構築する。通常の保存済みノード取得は、壊れたコードを実行せず修復用に使える。CLI・MCP・英語ガイドと公開説明も更新した。外部ライブラリの暗色テーマ、入れ子の生成元、実行時素材の変更、過大応答後の小さい読み取り、生成 ID の編集拒否、実 MCP の取消と後続要求、ページ分割と古い計算の拒否を確認した。関連37試験が成功した後、最終の Rust コードで macOS arm64・Rust 1.99.1 の通常361試験・描画なし206試験、両構成の Clippy、既定バイナリーの最終ビルドと書式検査が成功した。通常26件・描画なし3件の opt-in は今回の全体試験から除外し、課金を伴う実エージェント生成や実 Chromium の試験を今回再実行したという意味ではない。配布関連12試験も成功した。同梱の週間グラフを実 CLI の取得・計測・PNG 出力で照合し、取得した16ノードのレイアウト、計算 ID、画像のハッシュと960×600の寸法が一致した。診断は0件で、日本語・曜日ラベル・棒グラフを目視確認し、元文書のバイト列は変わらなかった。Linux の上記254ファイルの検証用ソースには今回の追加を含めない。旧形式と本家との互換性、各 OS、全機能と公開準備全体は引き続き未完成である。

同日: スクリプトの元ソース上の位置と抜粋を追加した。元の実行では内部ラッパーが行番号へ加算されることを再現し、元コードと内部処理の名前を分けた。固定した QuickJS の関数ラッパーとグローバルな参照範囲を保ち、正規化 API の取得・一時変数の削除を終えてから元コードをコンパイルする。ヘッダー、構文、実行時の位置、動的コードへの呼び出し、終端と位置のない例外を区別する。行区切り5種類、BOM、日本語の UTF-8 バイト列、長い行の窓、メッセージ内の偽のフレーム、内部変数と正規化関数の隔離を試験した。実 CLI の JSON と通常表示、失敗時の画像未保存、実 MCP の外部ライブラリ由来の入れ子の失敗から、新しい不変コードへの差し替え・再計算まで確認した。関連38試験の後、macOS arm64・Rust 1.99.1 で通常369試験・描画なし212試験が成功した。続く Clippy の表示ラベル選択への指摘は、許可したラベルだけを表示する条件を維持した等価な表現へ修正した。修正後は制御文字・未知ラベルの回帰試験も追加し、通常構成の関連70試験・描画なし構成の関連39試験、両構成の Clippy、書式検査と既定バイナリーの最終ビルドが成功した。配布関連12試験も成功した。全体試験の通常26件・描画なし3件の opt-in は除外し、課金を伴う実生成や実 Chromium を今回再実行したという意味ではない。旧 Linux バイナリーと今回の macOS バイナリーで独自の7入力の返却値を照合し、すべて一致した。同梱週間グラフの PNG も変更前とバイト一致し、診断は0件だった。既定バイナリーから、診断手順を含む英語の scripts ガイドと内容ハッシュも取得した。これらは本家との互換性や全環境の完成を示さない。

最終のアプリケーション入力を含む258ファイルの開発用ソースは、同じ内容から2回作成したアーカイブが一致し、SHA-256 `9e05c4d5ea6c6e45f2fe49234194f80f2b2664d3bf1000162ad86b10529648f4` として検証・展開した。Linux arm64・Rust 1.94 で、この修正と計算済みノード取得を含む通常370試験・描画なし213試験、両構成の Clippy、書式・配布関連12試験、release ビルドと mesh／1,000ノードの描画が終了コード0で成功した。両画像の寸法と診断0件も確認した。ひとつ前の診断用ソースのジョブは、Clippy の修正と追加回帰試験を含むソースへ揃えるため明示的に停止した。観測のタイムアウトによる再起動ではない。最終ジョブはコンテナー内のビルド先を以前とそろえ、同一ハッシュを確認した60ファイルの binding patch と既存 native ソースを使ったが、実際には Skia の native ビルドが再実行された。キャッシュを再利用できたという実績にはしない。この固定ソースには、次のプロジェクト独自ガイド・スタイルの追加は含まれない。

同日: プロジェクトの独自ガイド・スタイル・既定スタイルを実装した。`config show` の参照先とフィンガープリントを確認し、`config trust` でユーザーが指示利用を承認する。MCP へ承認変更のツールは公開しない。ガイド・スタイル指示・選択の変更は再承認とし、作例・フォントのみの更新と TOML コメント変更は承認を維持する。作例を含む全体の版は別に追跡し、依存・計算・ページの古い結果を拒否する。英語の入口と任意言語の自動ガイドを分け、参照用ガイドは要求時に読む。初期プロンプトはスタイルの指示・トークン・作例の識別情報までとし、作例全体は MCP から取得する。設定のない既存のガイド応答は維持した。新しい設定の破損や作例の削除で保存済み文書の読み取り・描画まで失敗させず、指示・スタイルを要求した経路ではエラーを返す。copy/link は作例の素材の参照元を保持し、copy 後に元の作例と画像を削除しても描画できる。相対 `--data-dir` は子プロセスの作業ディレクトリを変える前に解決する。

最終の動作変更で macOS arm64・Rust 1.99.1 の通常382試験・描画なし225試験、両構成の Clippy、既定バイナリーのビルドと書式検査が成功した。通常26件・描画なし3件の opt-in は全体試験から除外した。今回の機能は疑似 ACP と実 MCP・CLI で検証し、課金付きの実エージェント生成と実 Chromium の試験を再実行したという意味ではない。設定文書の TOML・JSON・`.pen` の例をそのまま配置し、実 CLI で設定確認・承認・ガイド／スタイル取得・120×44 PNG 描画・承認解除まで実行し、診断0件を確認した。各 OS、本家互換性・美的品質の比較、残る設定・素材・描画・会話管理・性能・完全な依存表示・配布と公開準備は引き続き未完成である。

同日: プロジェクトのガイド・スタイルを含む261ファイルの開発用ソースは、2回作成したアーカイブが一致し、SHA-256 `efbcf1e29d26f19757b2f60316a53f6581c2c0780a3fe8206c1fc29ffd8ec292` として検証・展開した。Linux arm64・Rust 1.94 で通常382試験・描画なし225試験、両構成の Clippy、書式・配布関連12試験、release ビルドと mesh／1,000ノードの描画が終了コード0で成功した。両画像は所定の寸法・ノード数で診断0件だった。この固定ソースには、次の要求中の資料取得記録の追加は含まれない。

同日: 所有する MCP のガイド・スタイル・一覧の取得結果を、要求 UUID と接続 UUID に束縛した記録へ同期保存するようにした。会話ホストは正常終了・取消・失敗時に記録を凍結し、メタデータだけを `design_resource` として会話ログと NDJSON へ取り込む。入口に含まれる自動ガイドと、参照用ガイドの追加取得も個々の版・ハッシュで追跡する。一覧のハッシュは返却ページ全体のハッシュと明記し、全スタイルの本文を読んだことにしない。別文書の grant、古い要求、終了済み・失効した接続、取消、容量超過、破損、並行書き込み、symlink／FIFO を検査した。通常の外部 MCP にはログを追加しない。会話ログの最終書き込みに失敗しても `complete` を出す経路も修正し、失敗通知と下書き・添付の復元を行う。

関連58試験と通常構成の Clippy の後、最終の動作変更で macOS arm64・Rust 1.99.1 の通常389試験・描画なし232試験、両構成の Clippy、書式検査と既定バイナリーのビルドが成功した。配布関連12試験も成功した。実 stdio MCP と疑似 ACP の取得結果を返却ハッシュに照合し、ログ破損やエージェント失敗でも保存済み編集を保持することを確認した。実 TUI PTY は同じ MCP 接続の2要求で異なるガイドを取得し、要求ごとの3記録と実際に受け取ったハッシュを照合した。通常26件・描画なし3件の opt-in は全体試験から除外し、そのうち次の実生成2件は別途実行した。取得記録はネットワーク配送やモデルの理解・適用、同一ユーザーのファイル操作に対する改ざん防止を証明しない。親の異常終了後の回収・保持方針全体、各 OS、全機能と公開準備の完成は引き続き未達である。

同日: 今回の取得記録を含む既定バイナリーで、Codex ACP 2.2.2 と Claude ACP 0.91.0 の実サブスクリプション画像生成2件も成功した。Codex は `read-only`、Claude は `default` とし、文書に束縛された MCP の操作だけを事前許可した。各成果物にはコピーした style 部品の有効なインスタンス1個と Lucide アイコン2個が残り、添付画像も登録 ID から取り込んだ。Codex の13件、Claude の4件の資料取得記録を会話ログと NDJSON で照合し、含まれるガイド・スタイルの全ハッシュを現在の CLI の値と比較して一致した。両保存文書のレイアウト検証は診断0件で、別途描画した640×360 PNGの日本語・参照画像・アイコン・保存ボタンを目視確認した。画像 SHA-256 は Codex が `f81bd4810616b0ae24ff893d1eea9a640a9d15f344976e0743c053fd59f7a674`、Claude が `0fa4c5ae5d2f15a1de71de1242518c7acb4af6c835a1a7c86898e656232653de`。これは本家との美的品質の比較、独自プロジェクトルールでの実生成、実 API/provider、他 OS の保証ではない。

同日: 素材の公開処理が保存中の依存ロックへ参加していないことを、実 CLI の回帰試験で再現した。欠損先の共有ロック保持中でも `assets put` が成功し、保存先を作成していた。アップロードと部品取り込みの共通 publisher は、同じパス・同じロックディレクトリーの排他ロックを取得してから公開するようにした。待機後にディレクトリーと既存バイト列を再検査する。文書ロックは追加取得せず、公開前の取消と256 KiBごとの出力処理を接続し、公開済みの完全なファイルの検査・同期は最後まで行う。文書・依存・設定のロック取得も共通処理へ揃え、履歴保存先の違いで回避されないことを確認した。CLI の素材入力は、上限付き・Unix では nonblocking open の通常ファイル読み取りへ揃えた。

素材関連14試験、部品コピーの公開待ち取消、実 MCP の画像・JS・GLSL の公開待ち取消と後続要求の復帰、通常構成の Clippy が成功した。その後、最終の動作変更で macOS arm64・Rust 1.99.1 の通常395試験・描画なし238試験、両構成の Clippy、既定バイナリーのビルド、書式・差分検査と配布関連12試験が成功した。通常26件・描画なし3件の opt-in は除外し、実エージェント生成・実 Chromium はこの修正では再実行していない。取消前に完成した不変素材が残る場合と、協調しない外部ファイル操作に対する限界は維持する。各 OS の実行、残る機能・性能・配布・完全な依存表示と公開準備全体は未完成である。

同日: 要求中の資料取得記録と会話ログ失敗時の処理を含む262ファイルのソースは、2回作成したアーカイブが一致し、SHA-256 `726231b73234a80fd79f292af8d38879056701d9b12948bf68b0179221c6b836` として検証・展開した。Linux arm64・Rust 1.94 でも通常389試験・描画なし232試験、両構成の Clippy、書式・配布関連12試験、release ビルドと mesh／1,000ノードの描画が終了コード0で成功した。両画像は所定の寸法・ノード数で診断0件だった。同じ MCP 接続の2要求の資料取得記録を含む PTY の全9ケースも成功した。この固定ソースは、上記の素材公開のロック・取消の修正より前であり、その修正の Linux 検証へ読み替えない。

同日: 素材ロックの横断確認中、Windows のロック競合を `ErrorKind::WouldBlock` だけで判定していた不備も見つけた。固定版 fs2 の実装と Rust 1.94 の Windows エラー分類を確認し、fs2 が提供する競合エラーの OS コードで判定する共通処理へ変更した。文書・依存・設定、資料取得記録、アイコン導入の待機処理に適用する。実際の競合から得たエラーの判定と、無関係な I/O エラーを再試行しない回帰試験を追加した。変更後は通常構成の関連77試験・描画なし構成の関連55試験、両構成の Clippy、書式検査と既定バイナリーのビルドが成功した。上記633試験の全体結果はこの最後の分類変更より前のものである。Windows の実機動作を確認したという意味ではなく、native CI と実行検証は引き続き必要である。[根拠と適用範囲](mcp.md#managed-assets)

同日: 素材公開のロック・取消と最後の OS コード判定を含む262ファイルのソースも、Linux arm64・Rust 1.94 で正常終了した。固定アーカイブの SHA-256 は `72276f7e83545e3d80627419217f82b36ac1f12f2cbb4bcc9ebb5049da9346e7`。描画なし239試験・通常395試験、両構成の Clippy、書式・配布関連12試験、release ビルド、mesh と1,000ノードの描画が成功し、両描画の診断は0件だった。このソースには、次のユーザー共通フォント設定を含めない。

同日: ユーザー共通の `config.toml` と、フォントの項目ごとのプロジェクト上書きを追加した。明示配列は空でも置換し、値を定義した物理ファイルのディレクトリを基準にパスを解決する。`config init/show/schema --user`、`LAZYPENCIL_CONFIG_DIR`、設定の出所表示、文書を指定しないフォント一覧へ接続した。両設定のバイト列と有効なフォントを依存・保存ロックへ含め、設定の変更・出現・削除と検索先の変化で古い計算を拒否する。共通フォントの変更はプロジェクト指示の承認を失効させない。ACP の子 MCP へ絶対設定先を渡し、起動時の相対設定先が文書ディレクトリへの移動で変わらないようにした。テスト用の標準スクリプトは、一時設定先へ隔離して開発者の実設定を読まない。

CLI の継承・画像出力、実 MCP の外部ユーザーフォント・古い依存での編集と計算の拒否、疑似 ACP と実 MCP の相対設定先、Unix PTY のプロジェクト／ユーザー別の書体変更と Kitty 画素・日本語下書きの保持を確認した。通常のユーザー設定読み取りでは設定ディレクトリを作成せず、明示初期化は共有ロック待機と取消に従う。フォント以外の共通設定、巨大なフォント集合の遅延読み込み、各 OS の実行と公開準備全体は未完成である。

macOS arm64・Rust 1.99.1 では、ユーザー設定追加後の通常410試験・描画なし251試験が成功した。その後の Clippy が指摘した不要な借用と条件式を等価に整理し、両構成の Clippy、修正後の関連97試験、書式・差分検査、既定バイナリーの最終ビルドが成功した。全体試験の通常26件・描画なし3件の opt-in は除外し、今回の機能で課金付き実エージェント生成や実 Chromium を再実行したという意味ではない。配布関連12試験も成功した。実 CLI でユーザー設定の Arial と同梱 Noto Sans JP を混在させ、プロジェクト側の言語指定が反映された960×560の画面を描画し、選択フェイス・設定の出所・文書バイト列の保持・診断0件と実画像の文字を確認した。PNG の SHA-256 は `7633d06e03838f60fcf671abea979ffbf1aa6aa72fc9159a162bc3b01d102eb4`。Arial はホストのファイルを参照し、配布物へ取り込んでいない。

このユーザー設定を含む263ファイルは、2回のアーカイブがバイト一致し、全ファイルと共有する60個の Skia binding ソースを照合した。固定アーカイブの SHA-256 は `f84b7659aacd1601d6ed8064ed278473c27cb74da0879a005181577ce990aaf0`。Linux arm64・Rust 1.94 の検証をコンテナー `lazypencil-user-config-f84b7659aacd` で開始し、ソース検証・配布12試験・書式検査の成功とビルド継続を確認した。その後、コンテナーは終了コード0で完了した。通常410試験・描画なし251試験、両構成の Clippy、release ビルドと mesh／1,000ノードの描画が成功し、両描画の診断は0件だった。試験後にも263ファイルのハッシュを照合した。この固定ソースには、次の文字装飾の輪郭線を含めない。

同日: 下線・取り消し線が文字の輪郭線へ含まれず、線だけを描くと装飾が消える不備を実試験で再現した。同じ Paragraph の native 描画から装飾の位置・太さを取得し、配置済み字形と領域の和を取って通常の線へ渡す。字形と装飾の交差で半透明の線を重ね塗りせず、内側・中央・外側の線と出力範囲を共通化した。記録には既存 Skia の SVG canvas を使い、字形のテキストを除外して固定版の装飾矩形だけを取得する。ブラウザー・追加依存・外部 SVG の読み取りは増やしていない。装飾と有効な線がある文字だけで実行し、一時記録は16 MiBを超えると解析前に拒否する。

72通りの混在書体・行・整列・小数サイズ・行間・字間・装飾を native Paragraph の画像と照合した。native の隣接矩形間の alpha seam は拡大した参照で分離し、アンチエイリアス境界の許容と不透明領域の厳しい検査を区別する。実 PNG で装飾の新しい線、内側マスク、半透明、画像・グラデーション、実 stdio MCP の編集と保持計算からの PNG を確認した。同梱の文字・罫線・画像・影の作例にも装飾付き輪郭を含めた。bitmap 字形の線、本家との描画比較、各 OS と公開準備全体は未完成である。[実装と検証条件](graphics.md#text-and-outlines)

最終の動作変更で macOS arm64・Rust 1.99.1 の通常415試験・描画なし251試験、両構成の Clippy、既定バイナリーのビルドと書式・差分検査が成功した。通常26件・描画なし3件の opt-in は除外し、課金付き実生成・実 Chromium はこの修正では再実行していない。配布関連12試験も成功した。同梱の文字・罫線・画像・影の作例を実 CLI で1000×760に描画し、診断0件と装飾付き輪郭を含む日本語・画像・影を目視確認した。PNG の SHA-256 は `920f455a77fa315f7061997dfeff7b9430b2d14e02b77dcfca9513cdfca00379`。release 検証へも固定サイズの作例と、塗りを空にした装飾線の有無による画像差を追加した。release スクリプトもユーザー設定を一時領域へ隔離する。macOS の最適化ビルドも8分50秒で成功し、mesh、1,000ノード、文字・罫線・画像・影の3描画と、塗りなしの装飾線の比較がすべて正常終了した。ビルド後にも246ファイルの一致を確認し、作例の release PNG は上記 debug PNG とバイト一致した。

文字装飾の修正と新しい release 検証を含む264ファイルも、2回のソースアーカイブがバイト一致した。SHA-256 は `87adc9224408217085ca6988342f2ac1af278414fb2ee3b875c6e429ae81cb52`。全ファイルと共有する60個の Skia binding ソースを照合した。Linux arm64・Rust 1.94 のコンテナー `lazypencil-decorations-87adc9224408` は、その後、内側輪郭線の画素検査に失敗して終了コード101となった。macOS の release に使った246ファイルの一致と上記の成功は有効だが、Linux の失敗を含めて両環境で成功したとは扱わない。

同日: Linux の失敗を実 CLI の独立した PNG 復号で調べたところ、装飾なしの字形でも、内側の線が native 塗りの外へ出ていた。31.5 px で9画素、32 px で11画素あり、装飾の有無では変わらなかった。固定版 Skia は字形パスを unhinted で生成する一方、Paragraph の既定字形には slight hinting が使われていた。両者を同じ形状にするため、文字の TextStyle に `FontHinting::None` を明示する。計測結果にも `hinting: "none"` を返す。ブラウザー内の文字や保存する `.pen` の属性は変更しない。

内側線の厳しい検査条件を維持し、装飾あり・なしの両方を回帰試験に含めた。修正後、macOS arm64・Rust 1.99.1 と Linux arm64・Rust 1.94 のそれぞれで、通常415試験・描画なし251試験、両構成の Clippy と既定ビルドが成功した。通常26件・描画なし3件の opt-in は除外する。Linux ではさらに重点試験1件、配布関連12試験、書式、最適化ビルド、mesh／1,000ノード／文字・罫線・画像・影の3描画と塗りなし装飾線の比較が成功した。課金付き生成・実 Chromium・本家との描画比較を再実行したという意味ではない。

この修正を含む264ファイルの2アーカイブはバイト一致し、SHA-256 は `18f252704f0fbe578cba8f64ac9aa98c6f1580c1c36048165f48a02a2c684023`。Linux コンテナー `lazypencil-text-hinting-18f252704f0f` は終了コード0で完了し、試験前後の264ファイルと共有 binding 60ファイルを照合した。この固定ソースには、次の Rust 通知収集ツール・通知データ・CI の追加を含めない。OS 間で PNG のバイト一致や本家との字形一致を保証するものではない。[文字と輪郭線](graphics.md#text-and-outlines)

修正後の Linux debug CLI でも、31.5／32 px と装飾あり・なしの4条件を描画して PNG の alpha を独立復号し、内側線が塗りの外へ出る対象画素がすべて0であることを確認した。macOS の最適化ビルドも修正後に再実行し、3描画と塗りなし装飾線の比較が正常終了した。前後の389入力のハッシュは一致し、文字・罫線・画像・影の1000×760 PNG は `dce8bb28907fe0537afd8d93bfa9cc720e393471f4c51b4d904b8b6a066a267c`。日本語・下線・取り消し線・輪郭線・画像・影を目視確認した。hinting の変更により修正前の PNG とバイト列は変わっており、古い画像の一致を今回の結果へ転用しない。

同日: Cargo.lock の全388パッケージについて、原本通知をオフライン収集・検証する `scripts/license_notices.py` と通知一覧を追加した。registry archive は lock の checksum で検証し、公開済みの通知・著作者宣言・ソースの表示を区別して保存する。archive に主な通知がない場合は、元の archive・公開 VCS commit・Git blob hash に束縛した upstream 資料を明示指定する。一般的な MIT 本文だけで個別の表示を置き換えず、宣言された作者から著作権者を推測しない。入れ子の QuickJS・ring 等の通知も保持するが、native と標準ライブラリの全監査を完了したとは扱わない。

通知束は267ファイル・3,251,335バイトで、原文263ファイル、一覧・来歴・読みやすい表示・manifest を含む。manifest の SHA-256 は `16aca753aa981d8db88cdc5a4d3586f8e62cf4e7db9ac1ebb78c455bc8009129`。同じ入力から2回生成した全ファイルが一致し、全388 archive から再収集した内容・メタデータとの照合も成功した。原文の6ファイルにある CRLF を含め、全263原文の Git filter 後の blob hash が元バイト列と一致することを確認した。可読版の空白正規化とは区別する。改変・不足・余分なファイル、archive checksum、不正 TAR、リンク、古い lock／ローカル資料、自己整合する偽作者情報、upstream の版・blob、CRLF を検査する新規12試験を含め、Python 全24試験と actionlint が成功した。CI へ保存済み束と原本再照合の2段階の検査を設定したが、GitHub 上での実行を意味しない。[通知と更新手順](../README.md#source-distributions-and-ci)

Rust 通知と CI の追加を含む533ファイルのソースも、2回のアーカイブがバイト一致した。SHA-256 は `f6391b3285ea5e3b9c7e7cd00e5ff0af34c5b7f8a1d808f698d11034044a49fb`。Linux arm64 の `lazypencil-rust-notices-f6391b3285ea` は終了コード0で完了し、Python 24試験、展開先での通知検証、全388 archive の原本再照合と通知再生成が成功した。生成した267ファイルは同梱版と全バイト一致し、試験後にもソース533ファイルを照合した。新しい Rust 動作は加えておらず、上記264ファイルの Rust 全体試験を再実行したという意味ではない。CI には展開したソース内の通知検証も追加した。この固定ソースには、次の native 棚卸しツールを含めない。

同日: Cargo の表示に含まれない native 依存を調べるため、既存の Skia ビルドから Ninja の対象グラフ・記録済みヘッダーを読み、ソース・生成物のハッシュ、原文通知、ソースのライセンスコメント候補を収集・再検証する `scripts/skia_notice_inventory.py` を追加した。通常の描画・起動時には実行しない。知らない native 部品、欠損・未ビルドの入力、native 入力のリンク、元ソースやライブラリを変更した古い束を拒否する。OS のヘッダーのリンクは除外対象として扱い、native 入力との混同を試験した。対応元ソースの変更・外部入力・ヘッダーだけの依存・未ビルド・CRLF 原文・不正メタデータ等の新規10試験を含む Python 全34試験が macOS で成功した。

macOS の実 release ビルドでは、Skia と8つの外部部品、ソース2,954入力、生成物1,452入力、原文・来歴33資料を記録した。2回の収集で34ファイルが全バイト一致し、実ソースとビルドからの再検証も成功した。zlib の SIMD・最適化コードには Chromium の BSD 本文への参照があり、Zlib の LICENSE だけで完了とは扱わない。これは最終配布用の通知一覧ではない。記録済みヘッダーや通知候補には未使用・試験用のものも含まれ、元 checkout の真正性、各ソースの参照先、最終リンク、Rust 標準ライブラリ・システム側・素材・ランタイム等の監査を別途続ける。[実行手順と範囲](../README.md#source-distributions-and-ci)

native 棚卸しツールを含む535ファイルの2アーカイブもバイト一致した。SHA-256 は `ba7e136550b26399476cd0900da10c4008d44aa15b0f3bdc0840e3411f7ba481`。Linux arm64 の `lazypencil-native-notices-ba7e136550b2` は終了コード0で完了し、展開したソースでの Python 全34試験、Rust 全388 archive の通知再照合、実 release Skia の棚卸し2回と再検証が成功した。Linux のソース入力は2,972件、Skia と8部品・33資料で、2回の生成結果34ファイルは全バイト一致した。試験後のソース535ファイルも照合した。root 外のシステムヘッダーは収集対象外であり、この検証を native 全体のライセンス監査完了へ読み替えない。

同日: zlib の Chromium 由来コードが参照する BSD 本文を、原本の repository・commit・Git blob・SHA-256 とともに `resources/licenses/native` へ追加した。Chromium `122.0.6261.0` の2024年1月23日の commit `b99eee51a56e42ecdaef53228da31fc3c12306b9` は、使用中の zlib commit の同月29日より前であり、本文の Git blob `2249a28657f86c645f8978b75babc1bd52aca318` は現在の upstream 原本とも一致した。原文は1,536バイト、SHA-256 `368cca1106be99d39ecd32a38d8305585d802a475effb66380b91ffc9bcf709b`。適用先は Skia DEPS のハッシュと参照元10ソースのハッシュへ束縛し、実際に観測した参照元が一致した場合だけ棚卸しへ追加する。ソース各年の著作権表示をこの本文の表示へ置き換えない。

`--upstream` を渡した macOS の実 release 棚卸しは、2回の35ファイルが全バイト一致し、実ソースからの再検証も成功した。既知の Chromium 本文欠落の finding は解消したが、空の finding 一覧を全ソースのライセンス分類・リンク監査完了という意味にはしない。省略時には従来の不足を報告する。元ソース・DEPS・Git blob・本文・repository URL・入力の構造を検査し、Skia の取得前にも使える `verify-upstream` を CI と展開先の検証へ追加した。GitHub 上で実行済みという意味ではない。

同日: Cargo.lock の外側にある Rust 標準ライブラリ向けに、`scripts/toolchain_notice_inventory.py` を追加した。実際の rustc の版・commit・target・sysroot を観測し、標準ライブラリの source/notice、COPYRIGHT-library とその参照本文、compiler-builtins 等の原文、LLVM libunwind の本文、vendor の publisher manifest、ソースのコメント候補と target library のハッシュを記録する。全 vendor ファイルを `.cargo-checksum.json` と照合し、package identity と宣言された archive checksum を標準ライブラリの lock と比較する。元 registry archive からの認証とは区別し、`registry_archives_verified: false` を明示する。見つからない表示は推測で補わず finding として残す。

macOS の Homebrew Rust 1.99.1 では、ソース7,238ファイル、vendor 30パッケージ・1,215ファイル、target library 55ファイル、原文等191資料を記録した。2回の80ファイル・2,935,259バイトの束が全バイト一致し、元入力からの再検証も成功した。fortanix-sgx-abi 0.6.1、vex-sdk 0.27.1、wasip1 1.0.0 の standalone notice 不足は残る。これらを含む全 lock・全 installed target library の棚卸しであり、本アプリにリンクされたという意味ではない。root 配布ライセンス・文書・完全な library/vendor を持つ入力が必要で、minimal／vendor のない rust-src から registry 原本を使う経路は未完成。別配置の明示指定と rust/rustc 両方の文書配置を扱う。Linux Rust 1.94 の minimal 環境に文書はあるが rust-src がないことも実コンテナーで確認した。これは macOS の source-to-binary 来歴や他 OS の標準ライブラリ全監査を完了したという記録ではない。[実行条件と手順](../README.md#source-distributions-and-ci)

native 参照の追加7試験と標準ライブラリの新規9試験を含む Python 全50試験が macOS で成功した。内容・パス・版の不一致、欠損・改変・余分な vendor ファイル、未導入 target、原文の CRLF、元ソースと library の変更、上書き、リンク、不正な metadata を検査する。これらは配布準備用のツールであり、本体の起動・描画時に Python の棚卸しを行う変更ではない。最終的な binary ごとの通知、他の native/system/asset 条件、他 installer の経路と残る製品機能は引き続き未完成である。

この変更を含む539ファイルの2アーカイブも全バイト一致し、SHA-256 は `43b296b2dc6a97179e1934c5446c3ffa391e1f62c98d658f6439269ff27884b2`。Linux arm64 の `lazypencil-reference-toolchain-43b296b2dc6a` は終了コード0で完了した。展開先での Python 全50試験、Rust 全388 archive の原本再照合、native 参照の単独検証と実 release 棚卸し2回・再検証が成功した。Skia と8部品・2,972ソース入力の35ファイルは全バイト一致し、元の著作権表示を保持したまま Chromium 参照を含む。試験後にも539ソースファイルを照合した。actionlint、Rust 書式と差分検査も macOS で成功した。これは Linux の実標準ライブラリ収集、Rust 全体試験の再実行、全ライセンス監査や公開準備全体の完了を示すものではない。

同日: 標準ライブラリの通知収集に `--cargo-home` を追加した。library/Cargo.lock の全 registry archive を原本 checksum で検証し、全 regular source member の SHA-256 を収集する。vendor のない公式 rust-src も扱う。vendor がある場合は従来の全 vendor checksum 検査も行い、原本との missing／extra／changed を別に記録する。局所的な欠損・改変を registry 内容で黙って置き換えない。`registry_archives_verified` と `vendor_file_checksums_verified` を分け、違いの確認が必要なことを finding に残す。リンクを含む source archive は原本ソースの収集で拒否する。プロジェクト自身の Rust 通知の出力形式は変更しない。

元 archive にない3パッケージの workspace-root 原文5資料を `resources/licenses/toolchain` へ追加した。archive checksum・公開された VCS commit・repository・Git blob に束縛し、版が異なる package へ流用しない。VEX SDK の公開 VCS の `dirty: true` は finding として保持し、root 本文の取得によって pristine な checkout と認定しない。MPL-2.0 の package には完全で未改変の registry source archive を `sources/` として同梱し、本文にパスと checksum を表示する。

Linux の実 Rust 1.94.0 では、公式 rust-src に vendor がないこと、COPYRIGHT-library が参照する LGPL-2.1-or-later／MPL-2.0／Zlib の本文が配布文書にないことを確認した。参照された識別子に一致する SPDX の固定 commit `31ba1a50e5397e00a304dbadc76531740e89ee48` の3本文を、Git blob・SHA-256 とともに保持した。元ソースの著作権表示と分け、参照用の本文という役割を明示する。書式の placeholder を作者名で補わない。Root の原文や知らない識別子の不足を、一般的な本文で隠す処理は加えない。Root 配布の COPYRIGHT・MIT・Apache は、実 rustc commit `4a4ef493e3a1488c6e321570238084b38948f6db` の原本を Git blob で検証して明示入力した。通常の収集にダウンロードやインストールは含めない。

macOS arm64・Homebrew Rust 1.99.1 では、30 archive・1,229 source member を検証し、83ファイル・3,640,619バイトの2束が全バイト一致し、再検証も成功した。14 vendor package では原本にある `.gitignore` が省かれており、差分へ記録した。Linux arm64・Rust 1.94.0 でも、公式ソース2,209ファイル・40 archive・1,506 source member・target library 55ファイルから、98ファイル・2,791,112バイトの2束が全バイト一致し、実入力からの再検証が成功した。Linux の実収集コンテナー `stdlib-registry-194-zeguwonq-registry-final` は終了コード0で完了し、使用した12個の collector/policy 入力も現在のファイルと照合した。3パッケージの standalone notice 不足は原本で補い、VEX の来歴と macOS の vendor 差分は残す。fortanix-sgx-abi 0.6.1 の13,691バイトの対応ソースは、lock の SHA-256 `5efc85edd5b83e8394f4371dd0da6859dff63dd387dab8568fece6af4cde6f84` と一致する。

元 registry、vendor なし、vendor 改変と部分欠損、workspace-root 本文の対応、dirty VCS、MPL 原本ソース、参照用本文の placeholder、識別子・repository・Git blob、archive 内のリンクを確認する追加10試験を含め、macOS の Python 全60試験が成功した。CI へ新しい upstream 資料の単独検証も追加したが、GitHub 上で実行済みという意味ではない。全 lock の棚卸しを最終 binary のリンク一覧とは扱わず、source-to-binary、残る native/system/asset 条件・最終の配布と製品機能は引き続き確認する。[収集モードと範囲](../README.md#source-distributions-and-ci)

この変更を含む548ファイルのソースも、2回のアーカイブが全バイト一致した。SHA-256 は `5b5dd2f6a9050dd70135c1a5e8e3d23ddedcee887193a2ed03d3b11b0bc70ad6`。実標準ライブラリ収集に使った12入力と展開先の一致も確認した。Linux arm64 の `lazypencil-stdlib-registry-5b5dd2f6a905` は終了コード0で完了し、Python 全60試験、プロジェクトの Rust 全388 archive の原本再照合、native と toolchain の upstream 単独検証が成功した。試験後にも548ソースファイルを照合し、同梱した原文8ファイルの Git filter 後の blob がすべて原本と一致した。actionlint、Rust 書式・差分検査も成功した。今回の Python／通知資料の変更で、Rust 全体試験・最適化描画を再実行したという意味ではない。

同日: GLSL の uniform 宣言を拡張した。修正前の実 Linux debug CLI は、同じ宣言の複数名と float 配列を使う独自の入力を `SHADER_COMPILE_FAILED`・終了コード4で拒否した。1 MiB の captured source から bounded token scanner で宣言を取得し、precision qualifier、コメント、複数名、1次元の数値配列を扱う。正確な `@resolution`・`@time` だけを検出し、`@time_step`・メールアドレスの誤認、隣接コメントでの注釈の脱落も修正した。grouped declaration は同じ注釈を共有し、組み込み入力の配列と保存済み上書きを拒否する。

`ivec2/3/4`・`bvec2/3/4`・column-major の `mat2/3/4` を通常値と数値配列へ対応した。配列は平坦な数値列、boolean の配列・ベクトルは数値0・1とする。整数リテラルの長さは Rust で検査し、定数・マクロ・式の寸法は元の GLSL のままコンパイラーへ渡す。WebGL が返す active な型と要素数に応じて値を設定し、最適化された未使用の末尾を除く。明示値の不足を拒否し、未指定の数値配列・行列等はゼロにする。これらの描画用既定値を `.pen` やソースへ保存しない。resource 上限は [動的コンテンツ](dynamic-content.md#宣言と数値入力) と英語の `shaders` ガイドへ定め、同梱ガイド集合の版を1.0.1へ更新した。Pencil 固有の未確認の注釈既定値、多次元・image sampler 配列、struct・マクロ生成 uniform と完全な vertex interface は残る。

macOS arm64・Rust 1.99.1 では通常423試験・描画なし251試験、両構成の Clippy、既定ビルドが成功した。通常28件・描画なし3件の opt-in はこの全体試験から除外する。新規8試験で宣言の分離、注釈の境界、型・長さ・範囲、未指定値、上限と不正構文を確認した。最初の実 Chromium 起動は開発環境の制限で `RUNTIME_UNAVAILABLE` だったため、Chromium の OS sandbox を保った host 実行へ切り替えた。新規2件の実ソフトウェア WebGL 試験では、group・scalar/vector 配列、整数・boolean ベクトル、非対称な行列の column-major、既定ゼロ、最適化された末尾、定数式で短い入力の失敗と後続要求の復帰、文書の保持を独立した期待画素で確認した。既存の動的描画12試験と、配列・行列の実 MCP の asset 追加・uniform 編集・image content・crop を含む1試験も成功した。実エージェント生成を再実行したという意味ではない。

独自の [uniform study](../examples/uniform-study.pen) と [GLSL](../examples/assets/uniform-study.glsl) を追加した。同じ shader を配色配列と座標行列で再利用する。実 CLI は960×640・11ノード・shader acquisition 2件・診断0件で完了し、元の2入力のハッシュを保持した。PNG は `e2f27d071b829bf8e772db4ef1da6011aa0f15d1ba0739359a6ec7f9f435b875`。横・縦の色と縞、日本語、角丸とラベルの位置を目視確認した。性能・全 OS・本家との描画／デザイン品質の一致を保証するものではない。Python 全60試験、書式・差分検査も成功した。macOS の最適化ビルドも正常終了し、mesh／1,000ノード／文字・罫線・画像・影と装飾線の3検査が成功した。前後の403入力のハッシュを照合した。その後の release CLI でも新しい shader 作例が診断0件で成功し、debug の PNG と全バイト一致した。

このシェーダー拡張を含む552ファイルの固定ソースも、Linux arm64・Rust 1.94 で終了コード0となった。アーカイブの SHA-256 は `70f1e85516202a952759a923ea75a1bb426a1d6666d799061de1e8018bb5b349`。通常423試験・描画なし251試験、両構成の Clippy、Python 全60試験、書式検査、release ビルドと3描画検査が成功した。試験後にも552ファイルを照合した。この Linux 検証では実 Chromium の opt-in は実行しておらず、上記 macOS の実 WebGL／MCP 検証と区別する。次のユーザー設定拡張を含めない。

同日: ユーザー `config.toml` に `[agents] default` と `[agents.codex]`／`[agents.claude]` の設定を追加した。アダプターと引数、認証方式・認証メソッド・報告の必須化、モデル・モード、文書ツールの承認方針とタイムアウトを保存できる。CLI の値の取得元を使い、組み込み既定値と同じ明示値も設定より優先する。`--require-auth-status=false` で true のユーザー既定値を上書きできるが、adapter が宣言した認証報告の要件や方式の不一致は引き続き拒否する。明示 `--adapter` は設定済みの引数もすべて置換する。

CLI と TUI の起動境界で同じ処理を使い、文書・会話を作る前に設定を検証する。単独コマンド名は PATH、相対の実行ファイルは設定ファイルの物理的な親を基準にし、引数をシェルで解釈しない。プロセスの作業先は文書ディレクトリのままとする。未選択の profile も型・長さ・容量を検査するが、実行ファイルの存在や内容は開かない。設定の show/schema/init、従来のフォント設定と独立した profile を扱う。プロジェクト側の `[agents]` と実行設定は引き続き拒否し、ガイド用の信頼を実行許可へ転用しない。

実 CLI と検証用 ACP の11試験で、選択・モデルとモードの適用、認証メソッド、subscription/API の報告照合、実行ファイルと引数の置換、厳格認証の解除、構造化された文書ツールの拒否・明示許可、実際の1秒 timeout と取消、保存会話の再開と変更された引数での拒否、エラーへ入力値を出さない検査、schema/template、シンボリックリンクの物理的な設定先を確認した。引数の取得元と TUI の共通優先順位の2試験も成功した。Unix の実 PTY 全11ケースも成功し、追加ケースでは保存した Claude profile で2要求を送り、モデル・モードを確認した。途中でユーザー既定値を存在しない別の adapter へ変更しても、進行中の会話は同じ ACP 接続を使い、文書を保持した。物理端末・実 API／課金付き生成の再実行を意味しない。ユーザーの runtime・key・asset/library 設定や、プロジェクトの実行設定と公開準備全体は残る。

macOS arm64・Rust 1.99.1 の全体試験では、描画なし263試験・通常436試験が成功した。その後の Clippy が新しいテスト内の不要な借用を指摘したため、その式だけを等価に修正した。修正後は両構成の Clippy、該当する11試験と既定ビルドが成功した。通常28件・描画なし3件の opt-in は全体試験から除外している。Python 全60試験、書式・差分検査と actionlint も成功した。

macOS の release ビルドと mesh／1,000ノード／装飾線を含む3描画検査も成功した。別の release CLI でユーザー Claude profile の選択・モデル・モード・認証メソッドを実際の検証用 ACP と確認し、release TUI の同じ保存設定で2要求と設定変更後の接続保持を確認した。前後の404入力が一致した。確認した binary の SHA-256 は `d601b43d78022b4cb1cf15b29fffd1cae8289b8f9fa677b475cc9ccef44d9dab`。この profile 試験はローカル検証用 peer を使い、実 API やサブスクリプションで推論を行ったものではない。

ユーザー設定拡張の最初の Linux 固定ソースは554ファイル、SHA-256 `43c67e3b52bf552dfd7111d290e0c14804c831d448043d45157445499fa03ea0`。Rust 1.94 で通常436試験・描画なし263試験、実 PTY 全11ケースを含めて成功した。続く Clippy は macOS ですでに修正したテスト内の不要な借用で停止した。この固定ソースを途中で書き換えず、全体試験の根拠と修正後の追加検証を分ける。

認証メソッドの検証用 peer には、設定された authenticate が session 作成より前に必要になる独立した確認モードも追加した。修正後の macOS の11試験と両構成の Clippy、同じ release binary の CLI・実 PTY の2要求で、この順序も成功した。実装の404入力は変えていない。

修正後の Linux ソースも554ファイルで、2回の archive が全バイト一致した。SHA-256 は `e9b619eb1a0c306663385400e4c365dd7a7c6c7bdae3d38765054b42aa1eea36`。先の全体試験のソースと比べ、実装・resource 等の404入力は一致し、Rust テストの借用と認証順序を確認する3テストファイル、文書だけが変わったことを照合した。Rust 1.94 の追加11試験、両構成の Clippy、既定ビルド、認証順序を必須にした実 PTY、release ビルドと3描画検査が終了コード0で成功した。Python 全60試験、通知資料の検査と書式検査も成功し、試験後にも554ファイルを照合した。最初の全体試験の554入力も、停止後にすべて照合した。

同じ Linux release binary で保存 Claude profile の CLI と実 PTY の2要求も成功し、model・mode・authenticate の順序、設定変更後の同じ接続と文書の保持を確認した。404入力のハッシュは一致し、binary の SHA-256 は `919739f7eb2e2b057cae7d13d081587953a5b25d8c44f2017c588081ba3fc226`。macOS と Linux の両方で最適化された経路を確認したが、他の CPU・Windows・物理端末や製品全体の完成を示すものではない。

同日: Phosphor と Material Symbols の初期パックを追加した。Phosphor Core 2.1.1 の原本 archive（SHA-256 `313332be6190b724da24107addd781799b48bf76b13963f24501112ffe1baadd`）から regular の1,512形状を正規化し、元カタログの PascalCase と廃止名1,548件を別名へ対応した。package JavaScript は実行しない。元の MIT 本文の CRLF も保持し、Git filter 後の blob が原本と一致することを確認した。原本 archive をオフライン fixture として同梱する。

Material Symbols Outlined／Rounded／Sharp は Google の commit `10ecc14790612ca8fd164e3f4ffe9c16fc50d5b7` の元 TTF を、専用コマンドで必要なものだけ取得する。各4,304名の元 `.codepoints`、Apache-2.0 本文、来歴とハッシュを持ち、font の copyright/name table を含む元バイト列を保存する。font はバイナリーへ同梱しない。`weight` は原本の100〜700を連続的に使い、省略時400、小数とテーマ変数を扱う。ほかの軸は `FILL=0`・`GRAD=0`・`opsz=24` とし、単位・軸・適用位置を検査する。合成太字や別のアイコンへの置換はしない。英語の `icons` ガイドを更新し、ガイド集合を1.0.2とした。

導入時に source／正規化データ／license を固定値と照合し、Material は元 font も毎回検査する。読み直したファイルを検証した後だけ、複数の live snapshot で不変データと native typeface を共有する。8位置の LRU と weak pack 参照により、以前開いた全 font を無期限に保持しない。追加インストールや変更・欠損の失効、manifest／license／形状／font の依存捕捉を共通化する。欠損時の導入コマンドは空白を含む library ID を引用する。`--input` は旧 `--archive` と同じオフライン引数へ対応する。

macOS arm64 の既存9試験と追加6試験では、オフラインの元 Phosphor の3,060名の形状、case を区別しない探索、元の別名・タグ、穴と半透明、共有と改変の拒否を確認した。Python 全65試験は、元 archive からの再生成と、カタログのデータ部分以外の式を評価せず拒否する検査を含めて成功した。原本 font の opt-in では3書体・全12,912名を解決し、100〜700・350.5、実画素の太さの違い、キャッシュの追い出し後の一致、テーマ・部品と改変を確認した。実 stdio MCP でも Phosphor の変数・部品と2色の画素、Material の numeric weight の保存と実 PNG、文書・font 変更後の古い計算の拒否が成功した。実 LLM 推論の再実行を意味しない。

実 Chromium との独立した比較は、元 Phosphor SVG の7例と、元 Material font 3書体×3重みの9例で成功した。最初は macOS のテスト用 HTTP socket が非同期状態を引き継いで大きな font 転送を中断していたため、接続後に同期へ戻した。正しい font を使った比較で境界のヒンティング差も分け、native の輪郭抽出を hinting none、参照の SVG text を geometric precision とし、同じ4軸・font synthesis none にそろえた。同じ許容値2/255で、Phosphor の最大平均差は約0.0095/255、Material は約0.439/255だった。Chromium の OS sandbox は有効のまま host 実行した。全体試験、最適化した作例、他 OS・CPU と完全な Pencil 往復互換の確認は別に進める。

2026-10-24: この追加後の macOS arm64・Rust 1.99.1 は、描画なし267試験・通常442試験（合計709）、両構成の Clippy、既定ビルドが成功した。描画なし3件・通常31件の opt-in は全体試験から除外し、今回の元 font 全名・重み・テーマの1試験、実 MCP の1試験、実 Chromium の16例は上記の個別実行と区別する。Python 全65試験、書式・差分検査と actionlint も成功した。2回の optional source 再生成で13リソースが原本から再現され、同梱ファイルと全バイト一致した。

実 CLI で Phosphor と Material 3書体の HTTPS 導入が成功した。Feather は固定 archive からオフライン導入し、追加5パックをそろえた。独自の [icon families](../examples/icon-families.pen) は1120×704・52ノード・診断0件、動的 artifact なしで描画され、日本語・3パックの形状・Material の100／400／700の違いを実画像で確認した。重みラベルを中央配置へ整えた最終 PNG の SHA-256 は `6843803c9e92cada4d785396f61cde2ebcb33f2a5e359990f0954179e9e7f694`、作例ソースは `310b4a3661b246afbcb2a9da9ec2511e14685716456c6ced1d1fecdae0bd1ce0`。実 Chromium を起動しない通常の Skia 経路だけで出力した。最適化ビルドと Linux の検証は引き続き進める。

macOS の最適化ビルドと既存の3描画検査も終了コード0で成功した。前後の418入力が一致し、release CLI でも新作例は診断0件・52ノードで成功した。debug と release の PNG は全バイト一致し、実装・作例ソースを保持した。最適化 binary の SHA-256 は `e7c3c6562143a14e0bd463fd3026d085f99aa7f8c21b778620662ff5095167d0`。追加フォントを含む作例の単発測定は約0.874秒・peak RSS約227.95 MiBだった。これはメモリ目標の合格ではなく、native font と元データの保持、基準文書・全プロセス群の測定と軽量化を引き続き実施する。[観測条件と未達項目](performance.md#optional-icon-font-observation-2026-10-24)

この icon 実装を含む573ファイルの source も、2回の archive が全バイト一致した。SHA-256 は `f434f2b01cd84478fcef4db1fdee4a9e05b89740edce4e2619bbd3f94f86c8d1`。実 Linux arm64・Rust 1.94 は終了コード0で完了し、描画なし267試験・通常442試験、両構成の Clippy、Python 全65試験、Rust 通知資料の検証・書式、原本 Material 全12,912名・可変重み・テーマの opt-in、実 MCP の weight 編集・PNG・font 改変時の失効、release ビルドと3描画検査が成功した。試験後にも573ファイルを照合した。実 Chromium の画素比較は Linux では実行しておらず、上記 macOS の host 実行と区別する。

同じ Linux release binary で追加5パックのオフライン CLI 導入も確認し、中央配置を整えた最終作例を573ファイルの frozen source とは別の検証用ファイルとして描画した。実装・resource は固定 source と同じで、作例データと文書だけが後から変わっている。1120×704・52ノード・診断0件、動的 artifact なしで日本語・6ライブラリ・3種類の太さを目視確認した。PNG の SHA-256 は `b8b40182ab0e6b32c8efe5188c7688635ae49ae89b483fe7253c013217b5a166`、binary は `d8e8102279474f2198743374d105931a1e8ce410a7c8a1302dbdd86878b6316f`。単発観測は約0.575秒・peak RSS約139.38 MiBであり、macOS との違いを含む性能の正式な評価・最適化は残る。[観測条件](performance.md#optional-icon-font-observation-2026-10-24)

### 依存データの検証時メモリ

2026-10-24: スナップショットを保持した状態で依存を検証するとき、画像・フォント・コードの全バイトを一時コピーしてから SHA-256 を計算していた。これを64 KiB の固定バッファによるストリーム検証へ変更した。読み込み区間ごとに取消を確認し、regular file・容量上限・内容ハッシュによる競合判定を維持する。存続している icon pack の再取得も、実ファイルをストリーム検証してから不変の原本バイトを共有する。Skia の所有データや字形・ウェイトは変更せず、snapshot の寿命だけに依存する借用データは導入しない。

読み込み境界・末尾・空・上限ちょうど・上限超過・Interrupted・読取エラー・途中取消を検査する3単体試験と、実 Project の素材の同じ長さの改変・伸長・切り詰め・64 MiB超過・復帰を検査する試験を追加した。macOS arm64 は描画なし271試験・通常446試験（合計717）、両構成の Clippy、既定ビルドが成功した。元 Material 全12,912名・可変重み・テーマ・改変検出と実 MCP の weight 編集・PNG・失効の opt-in も成功した。最適化ビルドと mesh／1,000ノード／文字・罫線・画像・影の3描画検査も終了コード0で完了した。[性能の観測と条件](performance.md#streaming-captured-asset-validation-2026-10-24)

同じ52ノードの作例について macOS release を修正前後で各3回測定し、子プロセス peak RSS の中央値は約250.38 MiBから144.58 MiBへ下がった。全6 PNG のバイト列が一致し、診断0件だった。Linux も通常の271・446試験が成功したが、残る検査と release の途中で、利用者のローカル容量削減と GHA 優先の指示に従ってジョブを停止した。Linux の今回の検証全体・新 release・性能測定は未完了とする。

以降の全体ビルドは GHA を標準とし、ローカルでは編集・書式・workflow lint 等の軽い確認を優先する。使い捨ての build target、重複したソース展開と開発用コンテナーを削除し、必要な検証記録と選択した成果物を保持した。移行時の CI 設定は旧5環境の source build、Cargo download cache、runner temporary directory、ログ・描画成果物と source artifact の3日保持に対応した。main への push、PR、手動操作で起動できる。設定の lint と実際の GHA 実行は区別し、push 前には remote の実行を成功扱いしない。[開発時の方針](../README.md#development)

### GHA への移行と依存・参照先の修正

2026-10-24: 利用者の指示で実装と CI を main へ反映し、commit `2f09f6b9b0a5cf5b0a959f0661c48c7ca820051c` の GHA 実行 `39983927769` を開始した。5環境で source archive・通知・書式の検査まで進み、Windows の初回試験と依存監査に失敗を確認した。監査は clipboard-win 5.4.1 の yanked を検出したため、5.4.3 へ限定更新した。新しい原本 archive の SHA-256 は `052af7878466feccda12ba7a6a67d019d712f296809559b47c12c8cc196d3a9a`、公開 package の VCS commit は `ed27008ae5d1f10cfbb266de039f0fb35fd05a56`。その commit の BSL 原本を再取得し、既存の通知と同じ1,338バイト・Git blob・SHA-256であることを確認して来歴を更新した。

更新した lock の SHA-256 は `9429a44093cbe9b21e7f7f5dfab0a540ead7b59e441e175ad63efb034d851c80`。通知267ファイルを原本から2回再生成して全バイト一致し、全388 package の原本 archive と再照合した。通知 manifest は `f59b9f2de0463f46ef9057c20e8de798f9a04c5b7efd08833cea3d04a529c028`。Windows の失敗は上限超過の試験データ作成時に append 専用ハンドルで set_len を呼んでいたため、fixture の準備に write 権限を使うよう修正した。製品の追記・上限・不正な記録の検査は維持する。CI は試験バイナリーと構成をまたいで失敗を収集し、最後まで検査したうえで失敗を返す。成功・失敗を模擬した軽量な実行で、5コマンドの継続、失敗状態、隔離設定の削除を確認した。

存続する MCP の計算結果を使い、インストール済み Feather の icons.json のリンク先を同じ原本バイトの別ファイルへ差し替えても、古い計算から画像が返る問題を実行で再現した。icon snapshot ごとに要求パスと canonical target を保持し、共有された不変の pack データとは別に検証するよう修正した。ファイル3種と親 directory の差し替え、cached load、元のリンク先への復帰、新しい依存での再計算を検査する2試験と、実 MCP の古い画像拒否・再計算後の画像返却を検査する1試験を追加した。これら Rust の変更は GHA で検証し、ローカルの native build は再作成しない。初回 CI の失敗を含め、GHA の起動自体を製品完成や全 OS 検証の成功とは扱わない。

### GHA の Windows 試験と追加フォント検証

2026-10-24: commit `c6085c0c945885505d0e2dea733471233d0e0b70` の GHA `39985891391` は、依存・license・source 監査と全388 archive による通知再照合が成功した。Linux x64 の通常試験・両 Clippy 工程も成功し、release 検査へ進んだ。Windows は描画なし242成功・2失敗・3 opt-in除外、通常410成功・2失敗・29 opt-in除外で、両構成の失敗は同じ2試験だった。前回の上限 journal fixture は成功した。残る失敗は provenance のパスを POSIX の suffix で判定していたためであり、期待する canonical file 全体との比較へ修正した。同じ試験内の後続のパス確認も修正した。Clippy が検出した Unix 専用 json macro の import には、その使用側と同じ cfg を付けた。新しいソースの Windows 検証は次の GHA で確認する。

Material 原本の取得を明示する CI helper と4試験を追加した。Google の固定 commit と3書体の SHA-256 を既存 catalogue から読み、16 MiB 上限・64 KiB のストリームで検証し、完了した原本だけを配置する。既存 cache も再検証し、checksum 不一致・mutable pin・想定外 redirect・過大入力では失敗し、一時ファイルを残さない。Python 全69試験と workflow lint・書式が成功し、既にある3原本でも同じ helper を実行して全ての pin とサイズを確認した。追加ダウンロードやローカルの native build は行っていない。5環境の GHA に、全12,912名・可変重み・theme と実 MCP の weight／失効の opt-in を設定した。設定済みであることと、その環境での実行成功は区別する。

同じ実行の Linux x64 job `120525593101` は終了コード0で完了した。描画なし273試験・通常449試験（合計722）、両構成の Clippy、既定ビルドに加え、release の mesh・1,000ノードの日本語 dashboard・文字罫線と画像影の3描画検査が成功した。描画なし3件・通常31件の opt-in はこの通常試験から除外しており、新しく設定した Material の2検査もこの実行の成功数には含めない。ログとこの環境の描画成果物だけを取得し、ローカルでの再ビルドは行っていない。

### GHA の依存ビルド再利用

2026-10-24: GHA `39990356564` の Linux x64 job `120540739751`、macOS arm64 `120540739687`、Windows `120540741705`、Linux arm64 `120540739468` が成功した。通常試験・両構成の Clippy の後に、固定した3つの Material 原本による全名・可変重みの opt-in と実 MCP の opt-in、release の3描画検査が成功した。依存監査も成功している。Linux x64 と macOS arm64 は描画なし273・通常449試験、Windows は244・412試験が成功した。これら3環境では小さい成果物だけを取得し、PNG のハッシュ・寸法・診断なしを確認した。Linux x64 の3画像は前回の成功結果と一致し、macOS arm64 と Windows の日本語 dashboard も目視確認した。macOS Intel の完了は引き続き確認する。

この実行を待つ間、以降の hosted build で依存物を再利用する cache を設定した。platform・runner image・Rust/build 設定・lock・vendored binding・icon catalogue を key に含め、Skia の取得済み source と原本 font も保存対象とする。保存は main の検証成功時に限定し、cache の不在・保存失敗を製品の検査成功へ置き換えない。main の更新は実行中の検証を取り消さず、最新の1件を待機させる。PR の更新は従来どおり前の実行を取り消す。これにより、長い native build の途中で main の次の変更を送っても、その検証結果と初回 cache 保存を失わない。

検査前と保存前に、Cargo の package 指定で lazypencil 本体を dev／release の両 profile から除く。古い3描画の PNG／JSON も除き、source archive の固定 timestamp によって旧アプリや旧画像を使うことを防ぐ。模擬 Cargo による軽量な確認で、2 profile の指定・6成果物の除去・依存 marker の保持・root directory の拒否・失敗の伝播を確認し、workflow lint と shell 構文も成功した。実際の cache 再利用と新しい CI 設定は GHA で確認する。成果物名には run attempt を含め、再実行時の証拠を区別する。

### GHA で検出した再読み込みと試験 fixture の問題

2026-10-24: `39990356564` は4環境と監査が成功し、macOS Intel だけが通常構成の実 PTY の `font-config` で失敗した。描画なし273試験は成功、通常は448成功・1失敗で、両構成の Clippy と既定ビルドは成功した。設定変更後の15秒以内に、新しい完全な Kitty frame と異なる画素が届く条件を満たさなかった。Intel の Material opt-in と release は未実行で、全体の成功とは扱わない。

調査中に、レイアウトを cache へ入れた後の描画失敗・取消を、後続の定期確認が「変更なし」として省く経路を修正した。画像の公開成功を別に記録し、未公開ならレイアウトが現在のものでも再試行する。最初の描画失敗と成功後の失敗からの回復・実画素・文書不変・成功後の再エンコード省略を検査する試験と、同じ font worker で TrueType から CFF の既定値を切り替える試験を追加した。PTY 失敗時には frame 数・画素ハッシュ・fixture 設定も出す。これらの Rust 変更は GHA で検証し、Intel の失敗の解決とはまだ断定しない。

cache 設定の実行 `39995993914` では、Windows の Python fixture が、`Cargo.lock` の変更検査後に原本 gzip を再作成していたため、秒境界で archive checksum が変わり、別のライセンス変更の検査を隠していた。gzip timestamp を1000→2000へ進める注入で失敗を再現した。元の lock bytes を保存・復元する修正後は同じ注入で成功し、Python 全69試験も成功した。製品のライセンス検査と拒否条件は変えていない。書式・workflow lint・Python 構文も成功し、ローカルの Rust／Skia ビルドは行っていない。既存 macOS arm64 バイナリーで更新した PTY helper の project／user font の2ケースも成功したが、このバイナリーは今回の Rust 修正を含まない。

### 繰り返し可能な1,000ノードの CLI 性能測定

2026-10-24: `scripts/measure_native_performance.py` を追加し、固定した shadow と日本語 dashboard の2文書について、初回と既定20回の繰り返しの実子プロセスを別々に計測する。文書・実行ファイル・生成 PNG のハッシュ、1,000ノード／1920×1080、診断・動的取得・script 実行なし、素材数を検査する。実行時間・子の peak RSS・user/system CPU の全試料と median・nearest-rank p95・最大値を残す。Unix は個々の子の `wait4`、Windows は元の process handle の OS API を使い、親や以前の子の累積 high-water mark を使わない。期限・出力量・実行失敗・結果の変化を拒否し、完成 JSON だけを上書きなしで公開する。測定用の文書と画像は終了時に除去する。

追加7試験と Python 全76試験が成功した。既存 macOS arm64 release バイナリー `8f322822…` で各21回、合計42描画が成功し、文書ごとに全 PNG が一致した。繰り返しの中央値／p95 は shadow 約160.84／165.26ms、dashboard 約824.94／907.03ms、最大 peak RSS は各133,136,384／123,322,368 bytes。これは現行ソースをローカルで再ビルドした結果ではない。GHA の release smoke に組み込み、小さい JSON を既存の成果物へ含め、依存 cache 保存前に除去する。新しい CI 統合と Windows／Linux の計測は GHA で確認する。統制した cold、全プロセス群、待機 CPU、TUI 応答、全性能要件の完了とは扱わない。[測定条件](performance.md#repeatable-native-child-measurements)

### 実 PTY の画像解析負荷の削減

2026-10-24: Intel の font reload の調査で、実 PTY helper が待機条件を調べるたびに全出力を再解析し、同じ Kitty payload を何度も復号していることを確認した。既存 macOS バイナリー `8f322822…` と Python profiler を使った project-font 1ケースは約6.57秒、そのうち Kitty 解析の累積が約4.07秒、Base64 が879,974回だった。`tests/support/terminal_images.py` の差分 decoder へ置き換え、同じ条件では約2.63秒・658回になった。RGBA と alpha、完了 frame のみの公開、分割 packet、問い合わせとペイロードなしの削除、寸法・payload・packet 上限を検査する。受信済みの大きな wire data を再保持せず、未完 packet と完成画像だけを保持する。

追加4試験・Python 全80試験と、保持済みバイナリーでの通常11 PTY ケースが成功した。15秒の期限や既存の画素変更・日本語・履歴・入力・端末復元の条件は維持した。VT の文字モデルは graphics overlay を表さないため、古い `No complete preview` の文字だけでは画像の欠落とは判断しない。画像は完了した frame の復号で検査する。新しい Rust バイナリーをローカルで作ってはおらず、Intel の失敗の解決は次の GHA で確認する。

### JavaScript 実行上限の共有設定

2026-10-24: ユーザー／プロジェクト設定へ `[runtime.javascript]` を追加した。評価時間・JS heap・待機時間・レイアウト全体の時間・評価回数・反復回数を項目ごとに継承し、明示プロジェクト値→明示ユーザー値→既定値の順で解決する。既定値と同じ値や待機0も明示上書きとする。型・範囲・未知の項目を検査し、上書きされる下位 scope の不正値も拒否する。`config show` の `runtime_origins`・schema・コメントだけの初期 template も拡張した。設定の生バイトは従来の依存・ロック・競合検査へ含まれる。

計算の共通入口で捕捉した設定を適用し、script input と分離した版1の envelope でワーカーへ渡す。ワーカー側でも範囲を検査する。JS の割り込みに加えて、親の監視時間と計算全体の絶対期限を使い、実行中の評価にも残り時間を適用する。割り込みを捕まえても、期限を越えた評価を成功にしない。計算記録の `limits` と ID に設定を含める。同期 native layout の途中停止、ブラウザー／shader と全プロセス群の上限を実装済みとは扱わない。

設定の継承・schema・不正値・競合・CLI の評価／反復／全体期限、実ワーカーの heap・時間・待機・絶対期限・回復・不正 protocol、実 MCP の設定変更後の失効と回復、TUI の timeout 後の修復・画素・待機0での子回収・下書き保持の試験を追加した。通常 PTY は新しいケースを含む12種類となる。ローカルでは Rust をビルドせず、これらの Rust／新しい PTY 試験は GHA で検証する。英語の scripts ガイドを設定と診断へ合わせ、ガイド集の版を1.0.3へ更新した。全体の未完了項目は維持する。

先行 GHA `39995993914` は macOS arm64 と Linux arm64 で release の検査と依存 cache の保存まで成功した。macOS の保存 log と GitHub cache API で実体を確認した。Intel は旧 PTY helper の同じ font-config timeout で失敗し、最新の差分 decoder は次の実行で確認する。Windows の旧 fixture の失敗も最新ソースでは修正を含むが、実行成功とはまだ扱わない。

続く `40000148383` は Python 全80試験が5環境とも成功し、Windows でも終了済みの実子プロセスのメモリー／CPU 取得と high-water mark の分離を確認した。この実行には新しい JavaScript 設定の Rust 変更は含まれない。各環境の Rust／release の終了は引き続き確認する。

### 入力ワーカー終了後の取消

2026-10-24: 実行 `40000148383` は macOS arm64 と Linux arm64 で通常試験・両構成の Clippy・Material の明示試験・release 検査まで成功した。Linux x64 は通常の11 PTY ケースが成功した一方、入力ワーカーの取消試験で `CANCELLED` の代わりに `CLIPBOARD_UNAVAILABLE` を返した。元の fixture は標準エラーを保存しておらず、その起動・終了原因は未確定である。fixture は CI が指定した Python を使い、起動失敗の診断を残すようにした。

関連する実装調査で、直接の子の終了後に出力を待つ最大1秒の間、取消と要求全体の5秒期限を確認しない経路を確認した。短い間隔で両方を確認し、完成出力を返す直前にも確認するように修正した。親が先に終了して子孫が stdout を保持する回帰試験を追加した。fixture 単体では、空白を含む Python パス・PID の原子的公開・親終了後の stdout 保持・process group の終了をローカルで確認した。Rust の回帰試験は GHA で検証し、元の Linux 失敗の解決とはまだ断定しない。ローカルの Rust／Skia ビルドは行っていない。

同じ実行の成功した arm64 2環境から `release-benchmark.json` を取得し、source commit・固定文書 hash・1,000ノード・1920×1080・各21回の結果を検査した。繰り返しの中央値／p95 は macOS の shadow 約319.05／447.51ms、dashboard 約1764.17／2058.10ms、Linux は各206.04／207.07msと1271.65／1284.11ms。各環境・文書内では全 PNG が一致した。ホスト負荷と cache は統制しておらず、全性能要件や他環境の release 検査を完了とは扱わない。[計測結果と範囲](performance.md#hosted-arm64-release-builds-2026-10-24)

### ブラウザー・シェーダーと画像 cache の共有設定

2026-10-24: `[runtime.browser]`・`[runtime.shader]` へ処理期限・起動期限・待機時間、`[runtime.dynamic]` へ共通描画期限・encoded PNG cache の容量と件数を追加した。JS と同じ明示値の項目別継承・範囲検査・出所・schema・コメント template・依存失効を使い、共通描画入口で CLI・MCP・TUI へ適用する。起動中にも処理／共通期限の残りを使い、取得の完了後と PNG encoding 後にも期限と取消を確認する。

各用途の待機時間を個別に適用し、設定変更時に取得 cache を消去する。容量・件数0では保持せず、単体で容量を超す画像も cache へ入れない。取得メタデータに実効の `limits` を含める。固定された画像寸法・PNG の上限と Chromium の sandbox は維持する。ブラウザーの認証・通信・待機条件、同期処理の途中停止、decoded/native memory・プロセス群の上限は未完成である。

通常試験へ項目別継承・既定値と0の明示上書き・不正な下位 scope・schema、cache の容量／件数・無効化・不正設定前後、期限の取消優先、待機する代役のブラウザーに対する2用途×3期限と子の回収、実 CLI の project 起動期限と出力不成立の検査を追加した。実 Chromium の別試験では目的別の即時回収、設定再読込後の取得 ID と画素、cache 無効化、文書不変を検査する。ローカルで Rust はビルドせず、通常の Rust 試験は GHA、実 Chromium の試験は opt-in として実行結果を確認する。英語ガイドを更新し、ガイド集の版を1.0.4とした。製品全体の残る必須範囲は維持する。

実 Chromium の opt-in をローカルのビルド木なしで検証するため、`chromium.yml` を追加した。選択した1環境で source archive を検証し、通常 CI の既存依存 cache を復元するが保存し直さず、アプリ出力を除去して現在のソースをビルドする。固定版の明示導入・完全性検査、設定・実 page/shader・実 MCP、Unix の実待機／動的 PTY を実行し、診断なし1000×640の複合例 PNG と hash・log・runtime identity を3日間保持する。通常 CI の feature／Clippy／release 検査を代替しない。workflow lint はローカルで検査し、実行成功は GHA の終了後に確認する。

最初の専用実行 `40007385870`（Linux x64）と `40007389921`（macOS arm64）は、既存の依存 cache を復元して現行ソースをコンパイルし、PNG encoding 後の期限検査が `Renderer` 経由で canvas の借用を延長する Rust エラーを検出した。ランタイムを描画中だけ再借用し、encoding 後は元の参照で確認するように修正した。両実行はブラウザー導入前の失敗であり、動的描画の成功とは扱わない。修正後の実行は両環境でビルドと固定版の導入・完全性検査まで通過した。

`40007807712` の Linux は設定試験7件と代役のプロセスに対する2用途×3期限の試験が成功したが、実ブラウザーが CDP 前に SIGTRAP で終了し、動的描画の13試験が失敗した。元の起動経路は stderr を捨てるため、この信号だけでは原因を確定できない。次の専用 workflow に isolated about:blank の実起動、stderr・共有ライブラリ・namespace の kernel 設定を残す診断を追加した。OS policy は変更しない。macOS の `40007811724` は設定・実 page/shader・実 MCP 画像の段階を通過し、実待機と PTY の段階を引き続き確認する。

専用 workflow の範囲を点検し、通常の JS ワーカー試験、`shader_uniforms` の数値配列・行列の実 WebGL 試験、通常と動的の全 TUI 試験も対象に加えた。これには入力ワーカーの取消回帰と12ケースの通常 PTY も含む。拡張した検査の成功は次の実行で確認し、ローカルの native build は追加しない。

macOS arm64 の `40007811724` は全工程が成功した。設定7件・動的描画16件・実 MCP 画像1件・実60秒の MCP 待機1件・実60秒の動的 PTY 1件が通過した。成果物の browser 完全性、1000×640・診断なし・2取得の limits・PNG hash を再検査し、日本語・HTML ページ・GLSL の複合画像を目視確認した。PNG は `4d1ca7eb7e29ceec82c6ef5556c40466bdfe40c9ae84cc924ffb7b68332b51ae`。この26試験は拡張前の範囲であり、新しく加えた JS ワーカー・cache unit・数値配列／行列・通常 TUI の成功、物理端末や他 OS、全体の公開品質を証明するものではない。次の実行は Linux `40008545887` と macOS `40008549460` で進める。

Linux の診断実行 `40008545887` は stderr に `No usable sandbox!` を記録した。共有ライブラリは解決し、`unprivileged_userns_clone=1`・`apparmor_restrict_unprivileged_userns=1`・namespace 数上限63852を確認した。次の専用 workflow では、完全性を確認したジョブ専用 Chromium の canonical な1パスだけへ `userns` を与える AppArmor profile を読み込む。パスを管理 runtime 配下に限定し、wildcard を許さず、profile と hash・読み込み結果・global 制限が1のままである証拠を成果物へ残す。Chromium の sandbox と sysctl を変更しない。この許可による起動と描画の成功は次の実行で確認する。製品の installer が管理者設定を自動で変更する機能は追加しない。

続く Linux `40009150116` は profile 読み込み後の sandbox 起動が成功し、cache／期限 unit 2件・設定7件・JS worker 12件・動的描画15件・数値 uniform 10件が成功した。残る動的 CLI の SIGINT 試験は130ではなく4を返した。元の assertion は出力を残していないため、実際のエラー内容は未確定である。CDP の read/write/flush が `Interrupted` を再試行しない経路を修正し、通信エラー・接続終了・起動失敗でも取消と期限を優先して確認する。CLI の assertion には stdout/stderr を追加し、元の失敗の解決は新しい実行で検証する。

macOS の拡張実行 `40008549460` は unit 2件・設定7件・JS worker 12件・動的16件・数値 uniform 10件・実 MCP 画像／実待機の各1件が成功し、TUI は5件成功・1件失敗だった。失敗は `script-config` が同じ画素の設定変更にも新しい terminal packet を要求したことによる。PTY は status 行を使い、同じ JS ソースの実行期限だけを下げて timeout、戻して緑画像へ復帰する観測へ修正した。別の worker 試験は、同じ PNG のまま設定変更で snapshot／dependency ID が更新され、その後の変更なしの確認を省略することを検査する。これらと CDP 修正は GHA で確認する。専用 workflow の library 試験も全件を実行し、ローカルの Rust／Skia ビルドは増やさない。

### 起動せずに取得する runtime と host の診断

2026-10-24: Linux の実起動失敗の調査に必要だった host 情報を `runtime status` と `doctor` へ接続した。両者は導入ファイルの有無・完全性と実起動を区別し、`startup: not_checked` を返す。host の OS／CPU・sandbox 必須方針、未確認の OS profile／共有ライブラリを明示し、Linux では実効 UID と3種類の namespace 関連 kernel 値を読み取る。存在しない値・読めない値・不正または128バイト超は null とする。値だけで、その実行ファイルに対する AppArmor profile や起動可否を推定しない。

CDP 前の子終了にも `browser_startup` 段階と同じ host 情報、診断コマンドを返す。任意のブラウザー stderr やページ内容は診断へ取り込まず、OS policy の変更・ブラウザー起動・ダウンロードを診断の副作用にしない。起動すると marker を残す代役を指定した実 CLI で、doctor／status が起動せず data／settings／lock を作らないこと、Linux の模擬 proc 値で既知・未知・不正・上限を検査する試験を追加した。通常 CI と専用 GHA で確認し、ローカルの native build は行わない。実ライブラリの依存検査・profile の検証と、製品内の一般的な起動 probe は残る。

macOS `40010094357` と Linux x64 `40010090321` は、source `c9dd6cb` の拡張検査92件がそれぞれ成功した。全 library 39件・設定7件・JS worker 12件・動的16件・数値 uniform 10件・実 MCP 画像／待機の各1件・全 TUI 6件で、同じ PNG の新 ID、設定だけの timeout／復帰、12種類の通常 PTY、2種類の入力取消も含む。Linux の CLI 取消も130で終了した。これは新しい host 診断の追加前の証拠である。

先行する通常 CI `40000148383` の Windows は両構成・Clippy・Material 2件・release 検査・42回の native benchmark が成功し、依存 cache の保存まで完了した。取得した小さい report で source・1,000ノード／1920×1080・各20反復を確認した。shadow の中央値／p95 は328／359ms、dashboard は907／922ms、最大 repeated peak RSS は各93,446,144／82,513,920 bytes。各文書内で PNG は一致したが、両方の hash は他 OS と異なり、その原因や全性能保証を確認したものではない。[計測範囲](performance.md#hosted-windows-release-build-2026-10-24)

### Intel の実行上限と依存 cache の継承

2026-10-24: 通常 CI `40000148383` の Intel は724件の通常試験を通過したが、Clippy の途中で60分の job 上限に達した。check annotation の `The job has exceeded the maximum execution time of 1h0m0s` を確認した。Intel のみ120分とし、Linux の package 導入は20分で区切る。後者は直近の専用実行で実際に約11分かかったため、その成功例を越える余裕を取る。

workflow 全体の hash に依存する旧 key は時間上限の変更でも cache を失効させるため、`native_cache.py` で追跡済みの依存・build script・属性・設定の Git fingerprint、実際の rustc identity とコンパイル環境、OS・runner image を key に束縛する。アプリ本体は毎回 prune して現在の source から作る。既に保存された4環境の旧 key は、監査した input fingerprint・Rust 1.94.0 の commit・コンパイル設定が一致するときだけ候補にする。未確認の Intel 旧 cache は候補にしない。変更された入力や不明な image、dirty／未追跡の依存入力は旧 cache に結び付けない。識別不能なら cache を省略し、検証は継続する。

実 Git での変更・staging・未追跡入力の拒否、依存や compiler／flag の変更、無関係な環境の除外、出力 key の入力検査の5試験と、Python 全85件が成功した。4環境の旧 key が条件付きで生成されること、現在の依存 fingerprint が監査値と一致することも確認した。新 key の復元・旧 cache の継承と保存は次の GHA で検証する。ローカル native build は行わず、全体の未完了項目は維持する。

通常 CI `40010058295` の終了済み3環境から、共通して MCP の新しい試験が必須 `expected_revision` を渡していなかったことと、script worker の既に `&str` である error code に対する不要な `.into()` を Clippy が拒否したことを確認した。試験は各更新後に取得した文書／依存リビジョンを正しく渡すようにし、型変換を除去した。Windows は利用記録への4並行 writer の一部が2秒の lock 待ち上限にも達したため、通常の文書 lock と同じ5秒の有限待ちにし、期限時の詳細を追加した。待機中の取消と、2.2秒後に peer が解放してから安全に記録できる回帰試験を加え、32件を保持する既存試験は維持した。残る2環境も同じ共通失敗と旧 Intel の時間上限を含むため、その実行を取り消し、準備した修正版の CI へ切り替える。修正後の Rust 検証は GHA で行う。

source `36d1269` の専用 Chromium は macOS arm64 `40014476233`（97件）、Linux x64 `40014480311`（99件）、Linux arm64 `40014488317`（99件）、Windows `40014484065`（84件）がすべて成功した。host 診断・起動しない doctor/status・利用記録の lock 取消／peer 解放も library/runtime 試験で通過した。Unix は実60秒の MCP 待機と全6件の TUI、Windows は対応する実 MCP 画像までを含み、Windows の物理端末や TUI の成功へ読み替えない。4環境とも監査した旧 native key から実際に cache を復元した。新 key への保存と通常の全環境 CI `40014323380` は引き続き確認する。今回の新しい待機条件はこれらの検証 source に含まれない。

同 source の通常 CI は macOS arm64 で描画なし284件・通常470件・Material 2件、Linux arm64 で描画なし286件・通常472件・Material 2件、両構成の Clippy、release・描画3件・各42回の benchmark が成功した。両環境の v2 native cache の保存と、その後の macOS 専用実行での v2 cache 復元も確認した。4環境の複合 PNG は1000×640・診断0件・2取得の実効 limits・固定 browser identity と report hash を照合した。Windows と Linux arm64 の日本語・HTML・GLSL の出力を目視確認し、macOS は以前に確認した PNG とバイト一致した。全 OS の出力 hash が一致するという意味ではない。

通常 CI の Linux x64 は20分の package step 上限に達し、Rust のコンパイル前に失敗した。ログでは Azure の mirror から11.2 MBの cmake・28.8 MBの libclang を繰り返し取得し、後者の1回目に約11分かかった。取得先をジョブ内の一時的な deb822 sources で公式 Ubuntu HTTPS archive／security に変更し、noble の同じ suite・component と既存の archive signing key を使う。package 一覧と20分の上限は維持し、runner 全体の source file は書き換えない。arm64 は既に成功した既存 source を使う。両 workflow は同じ helper を呼び、local invocation／未知 architecture の拒否、update 失敗の伝播、共通 source と一時ファイルの解放を4件の軽量試験で検査する。実際の x64 導入は次の GHA で確認する。Intel の実行は停止せず追跡する。

### Script を含む copy の接続

2026-10-25: 実行時対応 `b4bc7ee` は GHA `40147491465` の初回対象3環境・監査・source を成功した。通常／core は Linux 各1028件・macOS1018件で、実 worker の21 PNG・8400画素と配布ソース975ファイルを独立照合した。これを copy 操作へ接続し、現在の生成物に限定せず参照できる部品・変数・テーマを取り込み、JS の原文と宣言済み・追加指定の素材を管理アセットへコピーする。画像を指す文字列変数は本文・画像の両方で編集の連動を保つ。未登録の生成パスは同名のコピー先ファイルへ接続せず拒否する。追加の画像・shader・入れ子の JS は元 script の修飾キーごとの `script_assets` に指定する。

新しい core 試験は preview・commit・元 source の削除後の namespace、素材指定の失敗・link の不正指定での不変、無限ループを含む JS を copy 中に実行しないことを検査する。実 worker では2.19／2.21の元画像と copy 後・元 source 削除後を比較し、未実行の部品／変数／画像を選ぶ入力、利用者の明示入力、入れ子・テーマ・編集後の画素を検査する。実 WebGL は画像と本文の文字列を共有する sampler とコピー先の画像への編集を検査する。先行 CI はこれらの CPU を先に実行し、Chromium 後は新しい shader の1件に絞ってから最終3環境へ進む。Rust／Skia は GHA のみで検証する。

対応表を持つ script の再コピー・切り離し、一部の継承した入力・コード・metadata と変数 ref 上書き、生成する明示スコープ・file browser URL の閉包は未完成であり、拒否を維持する。追加素材の指定と原文の byte hash・元 source 削除後の実挙動は GHA の結果で確認する。全機能の完成、物理端末・性能・最低 OS・ライセンス・配布などの元の完成条件は維持する。

### 対応表付き script の切り離し

2026-10-25: 再コピー `98573e3` は GHA `40155735180` の対象3環境・監査・source を成功した。Linux 各1045件・macOS1035件の通常/core、84 Chromium 画像、3枚の release 画像・各42書き出し、実パン64px・ズームを照合した。再コピーの81 PNG・48,600画素と、配布ソース980ファイルの Git 一致も独立確認した。元の JS の byte hash は不変である。

次に、構造上の切り離しで対応表の所有文書を宣言済み別名へ保持し、コード URI の再配置と、入力／コードの失効を別々に維持する。入れ子の生成ノードは解決後の文書を使い、別名を二重適用しない。前後の等価性は生の保存 URI／hash ではなく、解決先・実効値・失効状態と通常のメタデータで検査する。コードは前後の検査で snapshot に捕捉し、disabled と古い未知入力を扱い、編集中に JS を実行しない。英語ガイドは1.0.22とする。

core は preview・保存・import 削除の拒否・disabled controls と、再度の独立コピーを検査する。実 worker は2.19／2.21それぞれの通常／継承・保存済み／明示的な変数入力・null/default・元の変数編集、別文書からもう一度切り離した後の独立コピーと元削除を確認する。実 WebGL は転送された画像 sampler と本文の連動を確認する。新しい Rust は先行 Linux の core/default/affected shader から GHA で検証し、その成功後に最終3環境へ進む。新しい結果は未確認であり、継承した copy の部分上書き・明示的な生成スコープ・file browser の閉包と、製品全体の完成条件は維持する。

### 対応表を持つ script の再コピー

2026-10-25: CI 改善 `61d4297` は GHA `40154199264` の3環境・監査・source を成功した。先行の CI 検査は14秒、core/default の build 工程合計は x64 2秒・arm64 7秒・macOS 12秒で、3環境とも1211出力の完全性を検証して `087b856` から再利用した。native 依存の再コンパイルは0件で、通常試験と実 Chromium・MCP・TUI・Material・release は実行した。ローカルの重い build は行っていない。

次に、対応表を持つ script の再コピーで、JS に残る元の名前・旧 ID の fallback、別スコープの保存済み入力と既定参照、変数・テーマ・descendant 経路・登録済み素材を合成する。元の JS は同じバイト列でコピーし、元の対応表で拒否したファイル名への暗黙の接続を加えない。script 4 MiB・コピー合計64 MiBの対応表上限を構築中に検査し、新規の絶対元パスの自動別名を除く。長い本文を画像ファイル候補として probe しない。英語ガイドは1.0.21とする。

新しい core 試験は元ファイル削除後の対応と依存閉包を確認する。実 worker は3回コピーと各元ソースの削除、保存済みの利用者入力・null/default・未実行分岐・入れ子／テーマの変数編集を検査する。実 WebGL では元削除後の再コピーと sampler／本文の連動編集を確認する。先行1環境の CPU と affected shader の成功後に最終3環境へ進む。新しい Rust の実行結果は GHA 未確認である。切り離し・継承した上書き・明示的な別スコープ・file browser の閉包と、元の公開完成条件は維持する。

### Script の独立コピーに向けた実行時の参照対応

2026-10-25: コピー後の JS を変更せずに生成物の部品・変数・テーマ・上書き経路をコピー先へ結ぶ `script_bindings` を追加する。保存済み入力の canonical JSON hash とコード URI、メタデータを所有する文書を固定し、同じ値の明示入力やコード上書きによる失効を区別する。ref 入力は実行時 handle へ変換し、生成した literal の参照と混同しない。対応状態を計算条件・実行記録へ含める。core/default の形式・対象・失効 unit と、元 source を削除した実 worker の画素、入力・null・変数・テーマ・descendant・literal metadata、別文書からの利用での同名衝突を検査する試験を追加する。先行1環境で新しい実 worker 試験を他の試験より先に実行し、同じ先行実行の後半では重複させない。実 Rust は GHA で検証する。

これは独立コピーの実行時参照の前提機能であり、copy 操作での対応表の生成、全スコープ・素材・入れ子と再コピー／切り離しの閉包は引き続き未完成。この時点では script コピー拒否を維持していたが、後続の独立コピー実装で解除した。リンクや静的 snapshot を代替とはせず、本来の独立コピーと公開可能な完成品質の条件を残す。

### CI の重複ビルド削減

2026-10-25 の追加調査では、直近8実行 `40137073301`〜`40152097284` の job／step 時刻と終了済みログを照合した。最新の3環境の runner 待機は2〜4秒で、core／default の build 工程合計は Linux x64 124秒・Linux arm64 360秒・macOS arm64 90秒、同じ通常試験は336／257／246秒だった。release 工程全体は208／241／246秒で、最適化コンパイルと画像・反復測定の双方を含む。別に最新の手動実行では、workflow 作成から最初の job 開始まで56秒かかった。これは job の runner 待機2秒と区別する。アプリ入力の一致した `40137073301` は core／default の build 合計が各3／7／9秒で、キャッシュでコンパイルを省けることを既存ログでも確認した。

CI・Python の CI 試験・説明文だけの変更にも従来は先行 Rust agent 試験と native cache の復元を行っていた。これらだけの変更は application cache と共通の入力境界で `ci` を選び、対象1環境の Python 全試験と現在の配布ソースの生成・検証を先行させる。Rust toolchain・native cache・Chromium の準備はこの先行 job では行わない。Rust・素材・埋め込みガイド・fixture・未分類入力が一つでも混ざれば従来の対象別先行検証へ戻す。先行検証成功後の3環境での core/default・Clippy・実 Chromium／MCP／TUI・Material・release、監査と source gate は維持し、キャッシュの compiler dep-info・出力 hash 検証と画像の再生成も変更しない。実行は main の push 1回に揃え、同じ変更の手動実行は重ねない。新しい進捗表は作らず、結果はこの既存記録へ追記する。

2026-10-25: 成功済み `40094747115`・`40097555337`・`40103285273` の job／step 時刻を照合した。待機は1〜4秒で、固定 cargo-deny の導入は169〜204秒だった。`40103285273` の release コンパイル自体は Linux x64 194秒・Linux arm64 213秒・macOS arm64 260秒で、release 工程全体の216／250／296秒とは区別する。通常試験・実 Chromium・実60秒 idle・TUI・Material・release の検証範囲は維持する。調査時の hosted cache は15件・10,896,984,343 bytesで、うち4件・5,186,184,401 bytesは同条件の full cache がある依存 checkpoint だった。成功した全環境検証の後にこの重複を整理し、application cache は各環境の full 2世代と未統合 focus 1世代に制限する。異なる ref の cache や他用途の cache は対象にしない。

固定 audit tool は version・実 compiler・runner image／architecture に束縛し、binary hash と実行時 version を確認して再利用する。advisory database や監査結果は再利用しない。依存 cache は従来どおりアプリを prune し、その後に別の application cache を検証して重ねる。source distribution の全入力（workflow scheduling を除く）・実 compiler／flag を含む native key・絶対パスを key に含め、保存するのは package の fingerprint／library／CLI／test binary のみ。compiler dep-info の入力が追跡済み source と一致しない場合や workflow 自体を include する場合、独自 build script がある場合は再利用しない。実 source の hash、cache file の hash／mode、追加の compile-time 環境も照合する。画像・試験結果は毎回生成する。対象限定 Clippy は test binary の link 前へ移した。軽量試験と実 GHA の結果は既存の検証記録で確認し、cache miss／拒否は fresh build へ戻す。

`8789b97` の3環境検証と source artifact は成功した。同じ audit key の cold job `120921699548` と warm job `120923033109` では準備工程が191秒から9秒へ短縮し、後者のコンパイルは0件だった。Linux full job `120916088058` は同じ source の focus から39出力を検証して再利用した。`c7d06d8` の focus `120936394399` は216秒で成功し、そのうち対象の試験は約66秒だった。新しい入力設定の再現を最初に一度だけ実行し、後続の対象試験から除外した。変更のない Wasm は再ビルドしなかった。3環境の最終検証はこの focus の成功後に開始した。

追加のキャッシュ調整では、workflow に加えて `scripts/ci/` と直下の `scripts/test_*.py` を application key の入力から除く。CI の変更だけで native binary を再コンパイルしないためで、除外ファイルを Rust が読み込む場合は compiler dep-info の照合で再利用を拒否する。ドキュメント・fixture・resource・その他の script は引き続き束縛する。除外パスの変更での再利用、除外したファイルを埋め込む場合の拒否、似た名前の実入力の失効を軽量試験で検査した。この追加調整の hosted 検証は、進行中の3環境検証を完了してから行う。

追加調整 `25c2676` と履歴追加 `7f0b49f` の3環境・監査・source 検証は成功した。直近3実行の job／step 時刻では待機は1〜4秒だった。`40123845625` の先行ジョブは147秒で、依存 cache 復元29秒、Clippy30秒、対象 build38秒、履歴の再現試験0.76秒で期待値の不一致を検出した。期待値修正後の `40124133381` は同じ runner image の依存 cache が見つからず、Skia 再ビルド中に Clippy の300秒上限へ達し、子プロセスの pipe が残って工程全体は373秒になった。試験へは到達していない。直近成功 `40120701435` の Linux x64 でも通常 build394秒・release 工程630秒を要し、待機列よりビルドを優先して改善する。

cache の一覧では application 6件が5,982,586,140 bytes、総量が10,183,648,331 bytesで、以前復元できた native key が消えていた。full 2世代を保存し終えてからの整理では容量の余裕を確保できないため、main の先行検証前に現在の source 入力を確認し、再利用できない application 世代を削除する。新 key は source hash を独立に含み、同じ source の full 1件と未統合 focus 1件を環境ごとに残す。依存・download・tool と別 ref の cache は保持し、同じ compiler／image のない依存 checkpoint も残す。CI だけの変更では source hash を維持し、compiler 入力と環境の完全性確認も維持する。

依存 cache がない先行ジョブは、短い lint 期限の前に対象 binary を一度だけ準備し、後段の同じ build を省く。その準備後に試験や lint が失敗しても依存だけは保存可能とする。対象の試験と最終3環境の成功条件は変えない。期限超過時は Cargo と同じ process group の子も回収し、Ninja や tee の残留を防ぐ。軽量試験には source hash の一致・旧世代の選別・別 ref と依存の保持・TERM を無視する子と pipe の回収を含める。Rust／Skia の実ビルドは GHA で確認する。

2026-10-25 の再調査では、成功済み `40134426833`・`40137073301`・`40139304446` の先行検証への待機は2〜3秒、先行成功後の3環境への待機は2〜5秒だった。`40139304446` の core／default build 工程は Linux x64 が27／32秒、Linux arm64 が74／238秒、macOS arm64 が63／104秒で、通常試験はそれぞれ31／202秒・35／215秒・39／178秒だった。同じ compiled source を再利用した `40137073301` の両 build 合計は x64 が3秒、arm64 が7秒、macOS が9秒だった。実60秒 idle、描画、Material、release の時間を待機列やコンパイル時間と混同しない。

追加修正は dynamic の先行 CPU 試験と Chromium 導入後の試験の重複を除く。`dynamic_render`／`shader_uniforms` は通常試験を導入前に実行し、導入後は ignored の実 browser 試験だけを実行する。他の変更対象は通常・ignored の両方を保持し、既知の再現試験も一度ずつ実行する。最終3環境の通常／core・Clippy・Chromium・MCP・TUI・Material・release・監査と source gate は変更しない。

README・root の Markdown・`docs/` の Markdown は application cache key から除外する。実際に Rust が埋め込む場合は保存時と復元時の compiler dep-info 検証で再利用を拒否し、新規ビルドへ戻す。`resources/guidance/`、fixture、実行用 script、非 Markdown の docs 入力は引き続き束縛する。キーの移行時には application output の再構築が一度発生し得るが、native dependency cache の識別は変更しない。試験結果や画像は再利用せず毎回実行・生成する。軽量検査は文書変更の再利用と埋め込み時の拒否、CPU／browser の全ケースが一度ずつ実行されることを検査する。進行中の既存 CI を完了させてから main の新しい CI で確認し、重いローカルビルドは行わない。

### 保存済み会話の読み取り

2026-10-25: 保存済み会話・ターンの一覧と、transcript 全体を容量制限付きでページ取得する共通 API／`conversations list|turns|read` を追加した。アダプター・認証・会話再開を呼ばず、使用中の会話も読み取る。UUID 順の live listing を明示し、一覧の cursor を文書・会話に束縛する。破損した既知の項目は unavailable として残す。JSONL は64 KiB以下で取得し、継続時は全内容の SHA-256 を要求・再検査する。mtime と長さを保った変更も失効させ、日本語の境界と末尾の未完了データを保持する。不正 UTF-8 は表示用文字列と元バイトの Base64 を返す。元データの64 MiB上限・30秒の協調期限・取消を持ち、FIFO と symlink の拒否を含めて試験を追加した。Rust／実 CLI の検証は GHA で行う。これは生の保存記録の読み取り経路であり、TUI の会話ログ閲覧、会話の選択・削除、保持ポリシーを完了扱いにはしない。

続く `7f0b49f` は対象3環境の通常試験・両構成の Clippy・Chromium／MCP／TUI・Material・release・source artifact が成功した（`40120701435`）。履歴の4 unit と実 CLI 1件が各環境の両構成で一度ずつ成功したこと、全959 source file、既存の84画像と実端末 protocol の出力を独立に照合した。Linux 各954件・macOS944件の通常試験であり、製品全体の完成や実機端末の合格へ読み替えない。

TUI の `/logs`・`/conversations` と palette から、会話一覧→ターン→整形ログ／原文を閲覧する経路を追加した。JSON 文字列を小さい buffer で逐次復号し、1レコードや長文全体の `Value`／文字列を作らず、32 KiB のページへ出す。未知・未完了・曖昧なレコードは安全な原文表示を残し、正確なバイトは原文 API に残す。内容 hash と formatter 版へ cursor を束縛し、変更時は元のページへ戻して明示更新を求める。一覧は逆方向 cursor も使い、全ページの配列を保持しない。整形ログのページ cursor は最大8,192件で、既存64 MiBの入力と表示量の上限内に収める。read-only background task は承認に優先せず、読取と AI が重なった Ctrl+C は両方を止める。下書き・画像・文書を保持した閲覧、同サイズ変更、承認の優先、実 PTY の3ページ・原文・更新・終了復元を試験に加えた。新しい表示処理と TUI 拡張は GHA で検証する。会話の切り替え・削除・全体の保持方針など、元の残件は維持する。

保存会話の選択を共通 `Conversation::select` と TUI の `u`／`N`／palette へ追加した。先に選択先の文書・agent・adapter・要求認証と排他を検証し、元の接続・grant を閉じてから切り替える。元の下書きと画像、選択先の native ID を保持し、送信まで新しい agent を起動しない。現在の ID の選択は無変更、使用中・欠損・異なる設定は元の状態を残して拒否する。親の短い registry lease と symlink／regular-file 検査も接続した。3つの共通試験、registry の所有解除、実 ACP の new→new→load と画像2→0→2、実 PTY の2会話の下書き保持と終了 ID を追加した。重い検証は GHA で行う。会話削除・保持方針と元の完成条件は維持し、選択の追加自体も hosted 結果を確認するまで合格扱いにはしない。

### ページの network-idle 待機

2026-10-24: `[runtime.browser] wait_until` に `load`（従来の既定値）と `network_idle` を追加した。後者は現在の main frame／loader に対する Chromium の `networkIdle` lifecycle 通知と document load を待ち、その後に従来の font・image decode・2 animation frame を待つ。別 session・子 frame・古い loader の通知を成功に使わず、現在の loader 一つだけを保持する。URL・header・本文や無制限の event queue は保持しない。context の破棄時に状態を解除し、既存の取消・取得／描画期限・失敗時の process 回収へ接続する。

設定の項目別継承・明示的な既定値による上書き・schema/template・依存失効と、取得 cache の失効・metadata を共通化した。shader へ同名設定を渡す誤りは拒否する。通常試験に session／frame／loader の識別と切り替え、不正通知、設定の継承と不正な下位 scope の拒否を加えた。実 Chromium の試験は、load 後の fetch が終わって緑画素になるまで待つこと、設定だけの再取得と同条件の cache 再利用、未完了の fetch の timeout・回収・復帰・取消、文書不変を検査する。Rust は GHA で検証し、ローカルは書式・差分などの軽量検査に限定する。英語ガイド集合は1.0.5とした。

source `8f14b13` の専用実行は macOS arm64 `40015918054`（101件）、Linux arm64 `40015927331`（103件）、Windows `40015922773`（88件）が全工程で成功した。新しい lifecycle unit 2件・設定の継承／拒否と schema・実 fetch の待機／画素／timeout／復帰／取消を含む。Unix の MCP 待機・全 TUI も通過した。通常 CI の全構成・Clippy・release と、x64 での新条件は引き続き確認する。

source `f2b9b84` の Linux x64 `40017390797` も103件すべて成功した。公式 mirror による package 導入は18秒で終了し、network-idle の実 fetch 回帰、MCP の60秒待機、全 TUI を通過した。旧 mirror の20分上限失敗を、取得先の修正後の実導入・全専用検証と区別して記録する。これは次の selector 待機の追加前の結果であり、通常 CI と Intel の実行は引き続き確認する。

この条件は Chromium の lifecycle を利用した取得方針であり、アプリ固有の完了、後から始まる通信、animation・worker・すべての cross-origin frame の静止を保証しない。認証 profile、通信制限、selector 条件、animation 制御、device preset の互換性などは必須の残件として維持する。[CDP lifecycle](https://raw.githubusercontent.com/ChromeDevTools/devtools-protocol/master/pdl/domains/Page.pdl)

### ページ内の要素を条件とする取得待機

2026-10-24: `[runtime.browser] ready_selector` と `selector_state` を追加した。空文字列で無効化し、指定時は現在の main document の最初の一致を待つ。`attached` は存在、既定の `visible` は正の面積の client rect と `checkVisibility` の opacity／CSS visibility／content visibility の条件を使う。被覆や viewport との交差、iframe／shadow root 内部の条件とは扱わない。selector は512 Unicode文字・NULなしとし、構文は Chromium へデータとして渡して検査する。誤った構文は即時 `PAGE_LOAD_FAILED`、満たされない条件は既存の取得／描画期限と取消に従う。文書へ独自属性を追加しない。[CSSOM の visibility](https://drafts.csswg.org/cssom-view/#dom-element-checkvisibility)

項目別の継承・明示的な空文字列・schema/template・依存と取得 cache の失効・metadata を既存の共通描画経路へ接続した。zoom と要求する scroll を待機中にも適用し、ページ側の CSS smooth scroll が中間位置を出力しないように instant を使う。待機後の font・画像 decode・2 frame と、実際の scroll の記録は維持する。JS の設定だけを読む経路で selector の文字列を複製しない。

通常試験に user/project の継承と保持済み設定の不変、下位 scope の不正値・Unicode上限・NUL・shader への誤指定、schema/template を追加した。専用 GHA に実 browser 試験4件を加え、load 後の表示と緑画素、設定だけの再取得と cache 再利用、引用符・JS 風の selector 文字列、hidden／opacity0／content visibility／寸法0／欠損・構文エラー、実 query を page が観測してからの取消・回収・復帰、smooth scroll と offscreen の content visibility を検査する。Rust は GHA で確認し、ローカルは軽量検査だけとする。英語ガイド集合は1.0.6へ更新した。認証・通信制限・animation・device preset・物理端末などの全必須範囲は維持する。

最初の専用検証では製品本体のビルドは通過したが、Linux の試験コンパイルが `toml::to_string` の未提供で失敗した。TOML は意図して読み取り用の feature だけを使っているため、試験のために依存構成を変更せず、試験用文字列・数値を TOML の basic value として作るように修正した。引用符・NUL・Unicode の試験値を保ち、実行結果は修正版の GHA で確認する。

修正版 source `01d559d` の専用実行は macOS arm64 `40019388943`（107件）、Linux x64 `40019393874`（109件）、Linux arm64 `40019397974`（109件）が全工程で成功した。新しい selector の実 browser 4件と、設定10件、従来の MCP／TUI も含む。macOS の複合 PNG と report を取得し、1000×640・診断0件・2取得・既定の selector metadata と実効 limits を確認した。PNG は以前に目視確認した `4d1ca7eb7e29ceec82c6ef5556c40466bdfe40c9ae84cc924ffb7b68332b51ae` とバイト一致した。

Windows `40019402106` は selector の3件が通過したが、取消試験の復帰時だけ同じ HTTP ページへの navigation が失敗した。取消の観測・`CANCELLED`・直接の子の回収は通過している。元の navigation 診断は network code を捨てるため、この出力だけでは原因を確定しない。試験用 HTTP server に、accepted socket の blocking mode の明示と、部分読取／一時的な待機を2秒の有限な header 期限まで継続する処理を加えた。500ms遅れて header が来る接続と、未使用の接続を残した終了を通常試験へ追加した。browser の5秒の取得期限や1.5秒の失敗検査は維持する。

navigation の失敗には `page_navigation` 段階と、128バイト以内の `net::ERR_` 形式の code だけを返す。その他の文字列・型は null とし、任意の URL・本文を詳細へ取り込まない。既知の code と不正値の unit を追加した。また、ページの global lexical な `scrollX`／`scrollY` ではなく `window` の実際の値を記録し、同名の変数を持つページで画像と metadata の5000px移動を照合する。これらの修正後の実行は次の GHA で確認する。

先行する source `36d1269` の Intel Mac は、通常試験・両構成の Clippy・通常 build を含む工程を成功で通過した。追加フォントと release の工程を確認してから、job 全体と cache 保存の結果を確定する。

修正後の source `d617d5e` は macOS arm64 `40021082049`（109件）、Linux x64 `40021085896`（111件）、Linux arm64 `40021090172`（111件）、Windows `40021078150`（96件）の専用実行がすべて成功した。Windows の同じ HTTP ページへの取消後の復帰、500ms遅れて来る header と終了、navigation の code／秘匿 unit、global な同名変数を持つページの実 scroll と緑画素を含む。Unix は従来の実 MCP 待機・全 TUI も成功した。元の Windows 失敗の network code は取得していないため、その原因を新しい診断から遡って確定しない。

4環境の小さい成果物を取得し、1000×640・診断0件・2取得・固定版 browser identity・管理 runtime の完全性・実効 limits と既定の selector/state を再検査した。各 PNG と report の hash が一致し、同じ環境の以前の PNG ともバイト一致した。macOS／Windows／Linux arm64 の比較元は目視確認済みである。OS 間で同じ hash になるという主張ではない。macOS・Windows・Linux arm64 は通常 CI が保存した v2 cache を実際に復元し、Linux x64 は監査した v1 を復元した。最新 main の通常 CI・Clippy・release・source artifact は、先行する Intel job の終了後に確認する。

### 次に実装・確認すること

- 両実エージェントの画像付きサブスクリプション生成は、上記の条件で確認した。実 API/provider、モデル・モードの全組み合わせ、ランタイムの制限と診断、保持と会話管理、対象3環境の子孫終了と異常な親終了後の回収は残る。Windows の追加対応は後日に回す。認証方式の確認は adapter report によるもので、請求額の監査ではない。TUI、クリップボード、設定・素材カタログ、配布と CI は未完成。MCP は変数の解決値と計算済みレイアウトの検証を実装した。計算済み・生成ノードの詳細取得も実装した。アイコンの追加カタログ設定や残る設定などを拡張する。通常編集と部品取り込みの保存準備・依存のロック待ちへ取消を接続した。静的レイアウト・描画全体の取消と、対象3環境の遅延検証は残る。
- JS は実行・生成物の計測・描画・待機時の終了を実装した。期限・heap・待機・反復の共有設定は上記の実装を GHA で検証する。2.11ヘッダーと本家の乱数列は上記 corpus で修正・比較し、実 worker と描画の GHA 検証を行う。残る入力・生成ノード・寸法の互換性と、その他の資源方針は引き続き追加・確認する。元ソースの位置と抜粋、動的コードの呼び出し位置を区別する診断は追加した。
- Chromium の導入・browser・GLSL の初期描画、待機時の終了、TUI の基本プレビュー・明示更新は実装・試験した。上記の未完成な機能、本家との互換性、対象3環境、専用ページ状態・プロセス使用量・TUI の周期更新や時刻操作の検証を進める。3機能全体の必須スコープは維持する。
- 上書き・置換・スロット操作とローカル・外部・継承したインスタンスの切り離しを実装した。部品の link/static copy は実装した。外部 script の切り離しには版付きメタデータで元の実行・入力の文書を保持する。script の独立したコピーと本家への往復描画互換、shader 画像変数の独立コピーに残る絶対 path・衝突する投影と本家比較、含まれる定義の再利用、ライブラリ参照の正式な版別コーデックは未完成。部品一覧のスロット・必要な変数・素材の詳細も拡張する。
- 長方形以外の辺別罫線は上記の実装を GHA で検証する。bitmap 字形の輪郭線、上記シェーダーと既存図形の残る互換性は引き続き追加する。影の18モードと mesh gradient は実装した。現在の crop transform・文字・罫線・内側の影・背景ぼかし・色と影の合成・mesh の範囲は [描画の実装](graphics.md) に記録する。明示的に拒否している機能を、対応済みとは扱わない。
- MCP ではスナップショットの保持・再利用とページ付きのレイアウト取得、フォントのコレクションと基本の代替チェーン、宣言済みローカル画像・コードの欠損修復を実装した。CLI 間での再利用、キャッシュの容量管理と性能、フォントの遅延読み込み、ライブラリ・設定フォント・実行時生成素材などの欠損修復は残る。ユーザー共通フォント設定と項目ごとのプロジェクト上書きも実装した。プロジェクト独自のガイド・スタイル設定も実装した。入口ガイド・自動ガイド・指定スタイルに加え、要求中に追加取得したガイド・スタイル・一覧も版とハッシュを要求へ束縛して記録する。資料の長期保持と親の異常終了時の回収は会話管理全体で残る。
- 大量のノードで診断を返す場合の応答量、スキーマ全体の属性検証、CLI の機械向け出力とエラーの残る経路も公開前に確認する。通常の引数解析エラーの JSON 出力は上記の実プロセス試験で確認した。
- 背景処理の中間画像と余白の範囲を絞り、基準の1,000ノードと1920×1080で性能を検証する。固定した単純な文書と、文字・部品・画像を含む文書の Linux release 初期観測は [性能の観測](performance.md) に記録した。親の画像コピーを減らして単純な影の文書の中央値を約0.31秒にしたが、最終ビルドでの統制した cold/warm、各 OS、TUI の起動・パン・ズーム・画像輸送、待機 CPU と全プロセス群の測定は残る。
- 既定値・寸法の相互依存・負の座標・効果について、許諾を確認したフィクスチャで互換性と性能を調べる。保存・依存・キャッシュの異常系も拡張する。
- Linux arm64 コンテナーのソースからのビルド・通常試験・release 初期描画を確認し、GHA の OS・source 別の実行結果は上表に記載した。最新 main の全環境検証、実デスクトップ・物理端末・最低 OS、バイナリー配布・署名、native を含む完全なライセンス表示とソース提供条件は残る。現在は upstream の修正を当てた `skia-bindings` をソースに同梱している。Cargo の package が patch を除去するため完全なソースアーカイブを用意したが、crates.io 公開の経路は別途解決する。
