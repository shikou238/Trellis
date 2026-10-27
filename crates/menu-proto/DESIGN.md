# menu-proto 設計書

> UI の操作感を試作・比較するツールの、最初の題材（PLAN.md §4）。UI ツール（uikit）自体を試す場所として残している。
> ゲームと作曲ソフトは知らない。

## 1. 概要とファイル構成

- ウィンドウ1が方式A（押している間だけ出るメニュー）、ウィンドウ2が方式B（開いて選んで決定するメニュー）。ファイルを保存すると自動でリロードされる。
- ウィンドウ3の**インスペクタ**で、両方のシーンの `defparam` をスライダーで調整でき、離すとソースに書き戻される。
- シーンのウィンドウで `T` を押すと**計測モード**になり、方式A と方式B を交互に出題して操作を `logs/` に記録し、終わるとインスペクタに集計が出る。

```
src/
  main.rs       クレートのディレクトリに移って engine::run(window_specs(), MenuProto::new)
  menu.rs       MenuProto（engine::Product の実装）: シーンのウィンドウ・インスペクタ・計測の配線。同梱シーンのテスト
  session.rs    TestSession: 計測モード（出題・完了判定・ログの書き出し）、TestConfig（ui/tasks.ui）
  analysis.rs   計測ログの読み込みと集計（Report, TrialRecord, SceneStats）
ui/
  menu_a.ui     方式A のシーン
  menu_b.ui     方式B のシーン
  tasks.ui      計測の設定（交互に試すシーン、試行回数、制限時間、課題）
logs/           計測のログ（.gitignore 済み。1 回の計測につき 1 ファイル）
```

## 2. `MenuProto`（`menu.rs`）

```
MenuProto
 ├─ scenes: Vec<SceneWindow>      (開いた順 = インスペクタの表示順)
 │    ├─ id: WindowId
 │    ├─ scene: ScenePlayer       (uikit。定義 + 実行時状態 + 入力の待ち行列)
 │    └─ session: Option<TestSession>  (計測中なら Some。計測用に別の ScenePlayer を持ち、scene の代わりに動かす)
 ├─ inspector: Inspector + inspector_window
 └─ report: Vec<String>           (直近の計測の集計。インスペクタの一番下に出す)
```
- `window_specs()` の順に「方式A」「方式B」「インスペクタ（420×640）」を開く。`SCENE_PATHS` の数だけシーンのウィンドウにし、次の 1 つをインスペクタにする。
- `Product` の各メソッド:
  - `key`: `T`（リピートは除く）で `toggle_session`。
  - `inputs`: 計測中なら `session`、そうでなければ `scene` の待ち行列に積む。
  - `pointer`: インスペクタのウィンドウだけ。Down = `Inspector::press`、Moved = ドラッグ中の値を `set_param`、Up = `write_param`（uikit DESIGN §4）。
  - `update`: 毎回 `collect_finished_sessions`（終わった計測のログを読んで集計し `report` に入れる）。インスペクタなら `Inspector::build`、シーンのウィンドウなら計測中は `session.update` → `session.build`、そうでなければ `poll_file` → `advance` → `build`。
  - `window_closed`: シーンのウィンドウが残っていなければ終了。

## 3. 計測モード（`session.rs`, `analysis.rs`, `ui/tasks.ui`）

PLAN.md §4.5 の「操作ログ」。触った感覚の違いを、データで分析するためのもの。

**設定**（`ui/tasks.ui`、`T` を押したときに読む。書き方は [uikit/DSL.md](../uikit/DSL.md) §6）:
```lisp
(scenes "ui/menu_a.ui" "ui/menu_b.ui")   ;; 交互に試すシーン
(trials 20)                             ;; 試行の回数（全シーンの合計）
(timeout 10)                            ;; 1 試行の制限時間（秒）
(wait 0.5)                              ;; 試行の間の待ち時間（秒）
(task "青" (opened 1))                  ;; 画面に出す文字と、完了とみなす通知
```

**課題と完了判定**: シーン側が遷移で `(emit opened 1)` のような**意味のある通知**を出し、計測側は課題の `(opened 1)` と比べる。
シーンの中身（変数名・状態名）に依存しないので、十字型と縦並びのように作りが違う方式でも同じ課題を使える。
- 名前と値が一致 → **完了**（`complete`）
- 名前が同じで値が違う → **誤操作**（`wrong`。違う画面を開いた）。試行はそこで終わる
- 制限時間を過ぎた → **時間切れ**（`timeout`）
- 計測を `T` で途中でやめた → **中断**（`aborted`。集計から除く）

**1 試行の流れ**（`TestSession::step_tick`。決定的で、テストは直接呼ぶ）:
```
Waiting: シーンを初期状態に戻して（ScenePlayer::reset）表示。wait 秒の間の入力は捨てる
   ↓ wait 秒経過 → 課題を乱数で選び、上部に表示（trial 行を記録）
Running: 入力をシーンに渡し、入力・遷移・無視された押下・通知をログに書く
   ↓ 完了 / 誤操作 / 時間切れ（trial_end 行を記録）
次のシーン（交互）で Waiting へ。規定回数に達したら終了（session_end）
```
- 計測用に**別の `ScenePlayer`** をシーンごとに持つ（ウィンドウ本来の `scene` は止まる）。1 つのウィンドウで方式を持ち替えるので、操作する場所が変わらない。
- 時間軸は計測全体で 1 本（`TestSession::tick`）。ログの tick はすべてこれ。
- 乱数は SplitMix64。シードと最初のシーンはログに残るので、出題順を再現できる。
- 計測中もファイルの変更は拾う。パラメータが変わったら `param` 行として記録して続ける。

**ログ**（`logs/session-YYYYMMDD-HHMMSS.log`、UTC。S 式で 1 行 1 イベント、書くたびに flush）:
```lisp
(session (started "2026-10-20 05:03:00 UTC") (seed 123) (ticks_per_second 60) (config "ui/tasks.ui"))
(scene 0 (path "ui/menu_a.ui") (source_hash "3f2a...") (params (item_w 190.636) (tilt -9.01) ...))
(trial 1 (scene 0) (task "青") (expect opened 1) (shown 1203))
(input 1250 press menu)
(transition 1250 closed open)
(ignored 1255 press confirm)
(input 1262 press right)
(transition 1262 open page)
(emit 1262 opened 1)
(trial_end 1 complete 1262)
(param 1400 1 item_w 300)
(session_end 2000)
```
- `source_hash` は FNV-1a（Rust のバージョンに依存しない）。同じソースで測ったかを見分ける。
- 入力を tick つきで全部残しているので、シーンのソース + パラメータ + 入力列から操作を再生できる（再生機能は未実装）。

**集計**（`analysis::analyze`）: ログを DSL と同じパーサーで読み、ログだけから計算する（指標を変えても過去のログで計算し直せる）。
- 試行ごと: 表示 → 最初の押下（**迷い**）、最初の押下 → 完了（**操作**）、押下数、無視された押下数、キャンセル数、結果。
- シーンごと: 最初の `WARMUP_TRIALS`（2）回を練習として除き、完了・誤操作・時間切れの数、時間の中央値（完了した試行のみ）、押下などの平均。
- 結果はインスペクタの一番下に出る（`Report::lines`）。

**最初の版で測っていないもの**: 「入力を受け付けない時間」と「入力から画面が反応するまでの時間」。今の DSL にはアニメーション中に入力を受け付けない仕組みがなく、見た目が落ち着いた時点もログからは分からないため（シーン側の協力が要る）。

## 4. 実行と操作

```bash
cargo run -p menu-proto    # どこから実行してもよい（起動時にこのクレートのディレクトリに移る）
cargo test -p menu-proto   # 計測・集計と、同梱の ui/*.ui を読むテスト
```

**キー配置**（engine の `input.rs` の `action_for_key`）:

| キー | 操作 / 機能 |
|---|---|
| Space | `menu` |
| ↑ ↓ ← → | `up` `down` `left` `right` |
| Enter / Z | `confirm` |
| Backspace / X | `cancel` |
| R | 全体のホットリロード |
| T | そのウィンドウで計測を始める / やめる |
| Esc | 終了 |

**2つのメニュー方式**（操作したいウィンドウをクリックしてフォーカスしてから）:

| | 方式A（`ui/menu_a.ui`, ウィンドウ1） | 方式B（`ui/menu_b.ui`, ウィンドウ2） |
|---|---|---|
| 開く | Space を押している間だけ表示（中心の上下左右に項目が広がる） | Space で開く / 閉じる |
| 選ぶ | Space を押したまま矢印キー → その方向の画面へ | ↑↓ で選び、Enter / Z で決定 |
| 戻る | 画面で Backspace / X → 閉じる | Backspace / X → 1つ戻る（画面 → メニュー → 閉じる） |

画面は4色（赤・青・緑・黄）で区別し、両方式で同じ遷移アニメーション（`page_time`）にしてある。
`ui/*.ui` は保存するだけで反映される。マウス移動で背景色が変わるのは、シーンが `background` を指定していないときだけ。
（分割前はチュートリアル由来の 3D の五角形が背景に出ていて WASD でカメラが動いたが、engine の見本 `examples/demo3d` に移した。PLAN.md §9.2）

**計測**: 方式A か方式B のウィンドウで `T` を押す。上部に課題の色が出るので、その画面を開く。方式は試行ごとに交互に切り替わる。
`ui/tasks.ui` の回数に達するか、もう一度 `T` を押すと終わり、ウィンドウは普段の表示に戻る。集計はインスペクタの一番下に出る。

**インスペクタ**（ウィンドウ3）: 両シーンの `defparam` がスライダーとして並ぶ。左ボタンでドラッグすると即座に反映され、離すと `.ui` ファイルに書き戻される。
範囲のあるパラメータはつまみの位置が値、範囲のないものは左右のドラッグ量で増減する。見出しには今の状態名、シーンにエラーがあれば赤字でエラーが出る。

## 5. 既知の問題・設計上の注意点

| # | 内容 | 場所 | 状態 |
|---|---|---|---|
| 1 | 方式A と方式B で画面の色や遷移のパラメータを共有する仕組みがないので、両方のファイルに同じ記述がある | `ui/menu_a.ui`, `ui/menu_b.ui` | 未対応 |
| 2 | 計測の入力の時刻はフレーム単位（1 フレームで溜まった入力は同じ tick になる）。迷い・操作の時間も 1/60 秒単位 | `TestSession::step_tick` | 精密な時刻が要るなら入力に時刻を持たせる |
| 3 | 計測の「入力を受け付けない時間」「反応までの時間」は未計測（§3） | — | シーン側に「操作中」を宣言させる仕組みが要る |
| 4 | `TestSession::start`（設定の読み込み・ログファイルの作成）と `T` キー・集計の表示、インスペクタのドラッグは、手動でしか確認していない。出題・判定・ログ・集計はテストあり | `session.rs`, `menu.rs` | — |
| 5 | 課題の表示位置・サイズは固定（上部の帯、40px）。右上の進み具合の表示は、ウィンドウが狭いとはみ出す | `TestSession::build` | 必要になったら調整 |

## 6. 次の段階（PLAN.md §4.6）

PLAN.md §4.6 の 1〜3（2 つの方式・スライダーとホットリロード・操作ログと集計）は実装済み。この先は必要になったものから:
- 実際に計測して、方式の違いを分析する（ツールを使う段階）
- 記録した入力の再生、ログのタイムライン表示
- 「入力を受け付けない時間」「反応までの時間」の計測（シーン側に「操作中」を宣言させる）
- 将来ほかの人に計測してもらうとき: 課題の説明、練習用の試行、参加者ごとのログ、集計の書き出し

## 7. 作業メモ

- 計測のログの形式を変えたら、`analysis.rs` の読み込みと §3 の例も直す。ログは集計の唯一の材料なので、記録できる情報は捨てずに書く。
- 計測の判定はシーンの中身を見ずに `emit` の通知だけで行う。新しい課題は `ui/tasks.ui` に、必要な通知はシーン側の遷移に足す。
- ウィンドウをまたぐ処理（インスペクタが他のウィンドウのシーンを読み書きする）は `MenuProto` に置き、相手は `WindowId` で指定する。
- Rust 側で UI を描くときは、矩形は `RectInstance`（色は `srgb8`）、文字は `Label` を `Frame` に詰める（インスペクタと計測モードが例）。
