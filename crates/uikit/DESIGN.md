# uikit 設計書

> UI シーン DSL（見た目 + ステートマシン）の読み込み・ホットリロード・実行と、パラメータのインスペクタ。個々の製品は知らない。
> 書き方の一覧は [DSL.md](DSL.md)。構文は dsl クレート、描画（矩形・文字）とファイルの窓口は engine クレート。

## 1. ファイル構成

```
src/
  lib.rs        ScenePlayer: シーンファイルの読込・自動リロード・固定タイムステップ・入力の受け渡し。FixedClock, TICKS_PER_SECOND
  scene.rs      Scene: DSL → コンパイル済みシーン、ステートマシンの実行（step）、毎フレームの評価。Runtime: 実行時の状態
  inspector.rs  Inspector: パラメータのスライダーの配置・描画・ドラッグ、任意の行（計測結果など）の表示
DSL.md          シーン DSL のリファレンス
```

使い方（製品側）: ウィンドウごとに `ScenePlayer::new(path)` を持ち、`Product::inputs` で `queue_inputs`、`Product::update` で `poll_file` → `advance(size)` → `build(size, frame.rects)`（戻り値が背景色）。例は menu-proto の `menu.rs`。

## 2. 失敗しても落ちない・状態の保持

- パースやコンパイルに失敗したら `log::error!` に `パス:行:列: メッセージ` を出し、**直前の正常なシーンのまま動き続ける**。ファイルが読めない場合も同じ。エラーは `ScenePlayer::last_error` にも残る（次に読み込みが成功すると消える）。インスペクタに赤字で出る。
- 変更検知は 200ms ごとの更新日時のポーリング（`ScenePlayer::poll_file`）。`reload` は製品が `R` で呼ぶ。
- `ScenePlayer` は**定義（`Scene`）と実行時状態（`tick` と `Runtime`）を分けて持つ**（PLAN.md §3.4）。
  - `Runtime` = 今の状態・その状態に入った tick・変数の値。
  - リロードでは `Scene::adopt` が、**同じ名前の状態と変数を引き継いだ** `Runtime` を作る。今の状態がファイルから消えていたら初期状態から始める。新しい変数は初期値になる。
  - `time` は続きから進む。`time` は `tick / 60` で求める。tick は `ScenePlayer::advance` が実時間を積算して進める（`FixedClock`。最大 250ms 分まで。処理落ちで固まらないため）。
- `step_tick`（1 tick を自分で進める）と `reset`（初期状態に戻す）は、計測のように自分の時計でシーンを動かす製品のため。

## 3. UI シーン言語（`scene.rs`）

```lisp
(defparam item_w 260 (Float 100 400 _))   ;; 調整用の値: 名前 既定値 (型 最小 最大 刻み)。`_` または省略は指定なし
(var cursor 0)                            ;; 実行時の変数: 名前 初期値。リロードしても値を引き継ぐ
(state closed                             ;; 状態。最初に書いた state が初期状態
  (on (press menu) (set cursor 0) (goto open)))
(state open
  (on (press down) (set cursor (wrap {cursor + 1} 4)))    ;; goto なし = 状態はそのまま
  (on (press confirm) (when {cursor == 2}) (emit opened cursor) (goto closed)) ;; when = 条件、emit = 意味のある通知
  (on (release menu) (goto closed))
  (on (after 2) (goto closed)))           ;; この状態に入ってから 2 秒後（goto 必須）
(def top {height / 2 - 120})              ;; 毎フレーム計算される名前付きの値
(background (rgb 24 24 32))               ;; 省略するとカーソル位置の色
(rect x y w h (rgb 255 0 0) (rotate 15) (visible open))  ;; 左上 x y、幅 高さ、色、[回転（度）]、[表示条件]
```

| 要素 | 内容 |
|---|---|
| 数値が書ける場所 | リテラル / 名前 / `{中置式}` / `(関数 引数...)`（`{}` の外の関数呼び出しは `Compiler::number` が `Form` として受け取る） |
| 色 | `(rgb r g b)` / `(rgba r g b a)`。すべて 0–255 |
| パラメータ型 | `Int` / `Float`（現状どちらも f32 として扱う） |
| 組み込みの値 | `time`（秒）, `state_time`（今の状態に入ってからの秒）, `width`, `height`（論理ピクセル）, `pi` |
| 状態名 | 式の中では、その状態にいれば 1、いなければ 0（`(visible open)`, `{page * {screen == 0}}`） |
| 関数 | `sin` `cos` `abs` `floor` `min` `max` `clamp` `lerp` `wrap` `select` `smoothstep`。**引数と使い方の表は [DSL.md](DSL.md) §4** |
| 操作（`press` / `release` の引数） | engine の `input::Action`（`menu` `up` `down` `left` `right` `confirm` `cancel`） |

**ステートマシンの規則**（`Scene::step`）:
- 1 tick ごとに、たまっている入力を**届いた順に**1つずつ処理し、最後に `after` を調べる。
- 1つの入力（または `after` の確認）につき、今の状態の `on` を上から見て、**最初に当てはまった1つだけ**を実行する。
- `set` はすべての値を同じ環境で評価してから代入する（`set` を書く順番に結果が左右されない）。
- `goto` すると `state_time` が 0 に戻る。自分自身への `goto` でも戻る。`goto` のない遷移では戻らない。
- `after` に `goto` がないとコンパイルエラーになる（条件を満たしている間、毎 tick 発火してしまうため）。
- どの遷移にも当てはまらなかった**押下**は「無視された入力」として報告する（解放は報告しない）。
- `(emit 名前 値...)` の値は `set` と同じく遷移前の環境で評価し、遷移の報告の後に `StepEvent::Emit` として報告する（menu-proto の計測の完了判定に使う）。

**名前の解決規則**:
- `defparam` / `var` / `state` は1周目で集めるので、**ファイル内のどこからでも参照できる**。
- `def` は上から順に処理するので、**def から別の def を使うときは、使うより前の行で定義する**。状態の中身や `rect` からは、どの def も参照できる。
- 同じ名前の二重定義、組み込みの値や `pi` と同じ名前はエラー。`set` できるのは `var` だけ。

**コンパイルと評価**（「毎フレーム辞書を引かない」原則）:
- リロード時に AST を `Scene` にコンパイルし、名前は**スロット番号**に解決しておく（`CExpr::Slot`）。
- スロットの並び: `[time, width, height, state_time, パラメータ..., 変数..., 状態フラグ..., def...]`。
- 評価のたびに `Scene::fill_env` で f32 の配列を作り（def は順に評価して追加）、`CExpr::eval` は配列を引くだけ。

**StepEvent**: `step` は起きたこと（遷移・goto のない遷移・無視された入力・通知）を `StepEvent` としてコールバックに渡す。`ScenePlayer::advance` は `log::info!` に出すだけ。計測など記録したい製品は `ScenePlayer::step_tick` にコールバックを渡す。

**Param**: 型・最小・最大・刻みと、既定値リテラルの位置（`value_span`）を持つ。インスペクタのスライダーとソースへの書き戻しに使う。値は `Scene` の中にあり、ドラッグ中は `ScenePlayer::set_param` が直接書き換える（リロードするとファイルの値に戻る）。

## 4. インスペクタとソースへの書き戻し（`inspector.rs`, `Scene::rewrite_param`）

```
左ボタンを押す   Inspector::press      … カーソルの下の行のパラメータでドラッグ開始
                                         （範囲のあるスライダーは押した位置の値にすぐ合わせる）
カーソル移動     Inspector::value_at   → ScenePlayer::set_param   … メモリ上の値だけ変える（即座に画面に反映）
左ボタンを離す   Inspector::release    → ScenePlayer::write_param … ファイルに書き戻す
                                         → 保存を検知して自動リロード（状態は引き継ぐ）
```
呼び出しの配線（どのウィンドウがインスペクタか、どのシーンを並べるか）は製品側（`SceneSummary` の列を渡す）。

- **値の決め方**（`Inspector::value_at`）: 範囲（min と max）があれば、トラック上のカーソル位置 → 値。なければ、ドラッグ開始位置からの移動量 × 1 ピクセルあたりの変化量（`step` があればその値、`Int` は 0.25、`Float` は開始時の値の大きさ / 200）。最後に `Param::snap` で、`step`（`min` を基準）・整数・範囲に合わせる。
- **書き戻し**（`Scene::rewrite_param`）: 書き戻すたびに**ファイルを読み直してコンパイルし、名前でパラメータを探して**、既定値リテラルの `value_span` の部分だけを `Param::format_value` の文字列に置き換える。コメントや書式はそのまま残る。ファイルがコンパイルできない・パラメータが見つからない場合は書き込まずにエラーをログに出す。
- **書式**（`Param::format_value`）: `Int` は整数、`Float` は `step` の桁数（なければ小数3桁まで）で、末尾の 0 は省く。
- **配置**（`inspector::layout`）: シーンごとに「見出し（パス + 今の状態名）→（エラーがあれば赤字）→ パラメータの行…」を上から並べ、最後に製品が渡す行（計測結果など）。描画と当たり判定で同じ配置を使う。

## 5. 既知の問題・設計上の注意点

| # | 内容 | 場所 | 状態 |
|---|---|---|---|
| 1 | 変更検知はポーリング（`notify` 等は未使用） | `ScenePlayer::poll_file` | 意図的（依存を増やさない、移植しやすい） |
| 2 | `time` と入力の処理はウィンドウごとの tick で、そのウィンドウが再描画されたときにだけ進む | `ScenePlayer::advance` | 現状は問題なし |
| 3 | 1フレームの間に複数の tick が進んだ場合、たまっていた入力はすべて最初の tick で処理される（入力の時刻の分解能は「フレーム」） | `ScenePlayer::advance` | 精密な時刻が必要になったら、入力に時刻を持たせる |
| 4 | 式の中のエラーは囲んでいるノード全体の位置で報告される（dsl の `Expr` が Span を持たない） | `Compiler::expr` | 必要になったら Expr に Span を足す |
| 5 | `step` のない Float パラメータは、書き戻すと小数3桁の値になる（例: `310.136`）。きりのいい値にしたいときは `step` を指定する | `Param::format_value` | 意図的 |
| 6 | 書き戻しはファイルを外から書き換えるので、エディタ側で未保存の編集があると衝突する | `ScenePlayer::write_param` | 注意 |
| 7 | インスペクタはスクロールできない | `inspector.rs` | 必要になったら対応 |
| 8 | インスペクタの当たり判定・ドラッグにはテストがない（`WindowId` をテストで作りにくいため）。書き戻し・丸め・書式は `Scene` / `Param` のテストで確認 | `inspector.rs` | 未対応 |
| 9 | 範囲のないパラメータの、1 ピクセルあたりの変化量（`Int` 0.25 / `Float` 値の 1/200）は仮の値 | `Inspector::value_at` | 触って調整する |
| 10 | シーンの DSL から文字を出す機能（`(text ...)`）、シーン間で共通部分を共有する仕組み（include など）はない | `scene.rs` | 未対応 |
| 11 | `std::time::Instant` を使うので wasm32 ではパニックする | `lib.rs` | 移植時に対応 |

## 6. 作業メモ

- DSL に新しいフォームや関数を足すときは、`Scene::compile` / `Compiler` / `Func::lookup` を更新し、テストを足す。名前はコンパイル時にスロットへ解決し、毎フレーム名前で検索しない。スロットの並びを変えたら `Scene` のコメントと §3 も直す。**書き方を変えたら [DSL.md](DSL.md) も直す**（関数を足したら §4 の表に行を足す）。
- シーンはキーを直接見ない。入力は engine の `Action` で受け取る。
- 実行時の状態は `Runtime` に置き、`Scene` には置かない（リロードで消えないように）。状態を足したら `Scene::adopt` で引き継ぎ方を決める。
