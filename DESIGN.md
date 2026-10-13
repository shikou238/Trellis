# learn-wgpu2 設計書

> 現在の実装の説明。「何がどこにあるか」「なぜそうなっているか」「どこが未完成か」を優先して書いている。
> 方針とその理由は [PLAN.md](PLAN.md) を参照。
> 行番号はすぐずれるので、場所は「ファイル + 関数名」で示す。

---

## 1. プロジェクト概要

- **出発点**: [Learn Wgpu](https://sotrh.github.io/learn-wgpu/) チュートリアルの写経。git履歴は章名に対応している（`Dependencies and the window` → … → `depth buffer`）。
- **現在の目標**: UI の操作感を試作・比較するツール（PLAN.md §4）。
  現在は、**DSL で書いた UI（見た目 + ステートマシン）をウィンドウごとに動かし、キーボードで操作できる**ところまでできている。
  ウィンドウ1が方式A（押している間だけ出るメニュー）、ウィンドウ2が方式B（開いて選んで決定するメニュー）。ファイルを保存すると自動でリロードされる。
- チュートリアル由来の 3D シーン（テクスチャ付き五角形 ×100）は残してあり、UI はその上に重ねて描く。「ゲーム画面の上にメニューが出る」状況の背景として使える。
- **未接続**: `src/text.rs`（glyphon による文字描画）。`lib.rs` で `// mod text;` とコメントアウトされている。

## 2. 技術スタック

| 用途 | クレート | 備考 |
|---|---|---|
| GPU | `wgpu 28` | wasm時は `webgl` feature、`Backends::GL` |
| ウィンドウ/イベント | `winit 0.30` | `ApplicationHandler` 方式 |
| 線形代数 | `cgmath 0.18` | |
| GPUへのバイト変換 | `bytemuck` | `Pod`/`Zeroable` derive |
| 画像 | `image 0.24`（png/jpeg のみ） | |
| モデル保存形式 | `serde` + `serde-lexpr`/`lexpr`（バージョン固定 `=`） | S式でシリアライズ |
| テキスト | `glyphon 0.10` | 未接続。wgpu 28 を共有（Cargo.lock 上 wgpu は1バージョンのみ） |
| エラー | `anyhow` | |
| ログ | `env_logger`（native, 既定 `debug`）/ `console_log`（wasm） | |
| 非同期の待ち | `pollster` | 起動時の adapter/device 取得と、wgpu のエラースコープの回収に使う |

Rust edition 2024。`crate-type = ["cdylib", "rlib"]`（wasm出力とネイティブbinの両対応）。

## 3. ディレクトリ / ファイル構成

```
Cargo.toml
src/
  main.rs            ネイティブのエントリ。lib の run() を呼ぶだけ
  lib.rs             run() / run_web()、モジュール宣言、hsv_to_rgb()
  app.rs             App: winit の ApplicationHandler 実装（イベント振り分け）。OS 依存の層
  appview.rs         AppView: 全ウィンドウ共有の GPU リソース
  window.rs          WindowWrapper: ウィンドウ毎のリソース + CameraController
  assets.rs          ファイル読み込みの窓口（読み込みはすべてここを通す）
  input.rs           入力の2層化: キー → 抽象的な操作（Action）の押下・解放（InputEvent）
  parser.rs          DSL のレキサー/パーサー（S式 + {中置式}）
  ui/
    mod.rs           ScenePlayer: シーンファイルの読込・自動リロード・固定タイムステップ・入力の受け渡し
    scene.rs         Scene: DSL → コンパイル済みシーン、ステートマシンの実行（step）、毎フレームの評価。Runtime: 実行時の状態
    renderer.rs      UiRenderer（共有パイプライン）/ UiLayer（ウィンドウ毎のバッファ）/ RectInstance
  ui.wgsl            UI 用 2D 矩形シェーダ（実行時にファイルから読む）
  camera.rs          Camera, CameraUniform, OPENGL_TO_WGPU_MATRIX
  model_instance.rs  ModelInstance / ModelInstanceRaw（3D のインスタンシング用）
  vertex.rs          Vertex, Model, ModelBuffer、モデルの保存/読込
  texture.rs         TextureWrapper（画像テクスチャ, 深度テクスチャ）
  shader.rs          ShaderWrapper（3D 用 WGSL 読込・BindGroupLayout 定義・リロード）, capture_validation()
  shader.wgsl        3D 用シェーダ（実行時にファイルから読む）
  text.rs            [未接続] glyphon テキスト描画
  bin/parser.rs      パーサー単体の動作確認用バイナリ
ui/
  menu_a.ui          ウィンドウ1のシーン（方式A: 押している間だけ出る十字型メニュー）
  menu_b.ui          ウィンドウ2のシーン（方式B: 開いて選んで決定する縦並びメニュー）
pentagon.json        3D モデルデータ（※中身は JSON ではなく S式）
happy-tree.png       3D 用ディフューズテクスチャ
run.html, pkg/       wasm 実行用（wasm-pack 出力。現在は動かない → §9）
```

## 4. アーキテクチャ

### 4.1 レイヤ構造（所有関係）

```
App  (ApplicationHandler<AppView>)              ← OS 依存の層
 └─ state: Option<AppView>                      ← 全ウィンドウで共有する GPU リソース
      ├─ device, queue
      ├─ shader: ShaderWrapper                   (3D: ShaderModule + BindGroupLayout[texture, camera])
      ├─ render_pipeline                         (3D)
      ├─ model_buffer / diffuse_bind_group       (pentagon.json / happy-tree.png)
      ├─ model_instances + model_instance_buffer (10x10 = 100個)
      ├─ ui_renderer: UiRenderer                 (UI: パイプライン + BindGroupLayout)
      └─ windows: Vec<WindowWrapper>             ← ウィンドウ毎のリソース
           ├─ nature: Arc<winit::Window>
           ├─ surface + config + is_surface_configured + depth_texture
           ├─ camera 一式 + camera_controller
           ├─ last_cursor_position
           ├─ input: InputMapper                 (押されているキー → 操作の押下・解放)
           ├─ scene: ScenePlayer                 (シーン定義 Scene + 実行時状態 tick / Runtime + 入力の待ち行列)
           ├─ ui_layer: UiLayer                  (画面サイズ uniform + 矩形インスタンスバッファ)
           └─ background                         (シーンが指定した背景色)
```

**設計意図**: チュートリアルの `State` を2つに分けている。
- **共有すべきもの**（device・パイプライン・メッシュ・テクスチャ）は `AppView` に1つ。
- **ウィンドウ固有のもの**（surface・深度バッファ・カメラ・入力状態・UI シーン）は `WindowWrapper` に。
  ネイティブでは起動時に2ウィンドウを作り（`App::resumed`）、それぞれ別のシーンファイルを読む（`appview.rs` の `SCENE_PATHS`）。

命名規約: wgpu/winit の生オブジェクトをラップした構造体は `XxxWrapper`、ラップ対象のフィールド名は **`nature`**。

### 4.2 起動シーケンス

```
main() → lib::run()
  ├─ ロガー初期化
  ├─ EventLoop::with_user_event()     (ユーザーイベント型 = AppView)
  └─ run_app(App)
       resumed()
         ├─ (wasm) <canvas id="canvas"> を取得して window に割当
         ├─ window1, window2 を作成
         ├─ native: pollster::block_on(AppView::new(vec![w1, w2]))
         └─ wasm  : spawn_local で AppView::new(vec![w1]) → proxy.send_event(AppView)
                     → user_event() で受け取り state にセット + 初回 resize
```

wasm では async を block できないので、生成した `AppView` 自体を **ユーザーイベントとして** イベントループに送り返す（チュートリアル準拠）。

`AppView::new` の流れ:
1. `Instance` 作成 → windows[0] で一時 surface を作り、それと互換な adapter を取得
2. sRGB の surface format を選択（なければ先頭）
3. device/queue 取得（wasm は `downlevel_webgl2_defaults`）
4. `ShaderWrapper::new` → `constuct_render_pipeline`（3D）
5. `pentagon.json` → `ModelBuffer`、`happy-tree.png` → `diffuse_bind_group`
6. `UiRenderer::new`（UI パイプライン）
7. 各 window から `WindowWrapper::new`（surface/カメラ/深度/`ScenePlayer`/`UiLayer` を作成。surface はこの時点では未configure）
8. 10×10 グリッドのインスタンス生成 → インスタンスバッファ

### 4.3 イベント処理（`App::window_event`）

`window_id` から `windows` 内のインデックスを引き、そのウィンドウにだけ処理を流す。

| イベント | 処理 |
|---|---|
| `CloseRequested` | そのウィンドウを `windows` から除去。空になったらアプリ終了 |
| `Resized` | surface 再configure + 深度テクスチャ再生成。**ここで初めて `is_surface_configured = true`** |
| `KeyboardInput` | `Esc`=終了、`R`=ホットリロード（全体）。その後そのウィンドウの `handle_key` へ（CameraController と InputMapper の両方に渡す。OS のキーリピートかどうかも渡す） |
| `Focused(false)` | `focus_lost`: 押されたままのキーをすべて離したことにする（解放が届かず押しっぱなしになるのを防ぐ） |
| `RedrawRequested` | `WindowWrapper::update()` → `render()`。Lost/Outdated なら resize で復旧 |
| `CursorMoved` | カーソル位置を保持（背景色に使用） |

キー入力はフォーカスのあるウィンドウにだけ届くので、2つの方式は**ウィンドウごとに独立して**動く。

`render()` の先頭で `request_redraw()` を呼ぶため、各ウィンドウは**常時連続描画**ループになる。

### 4.4 1フレームの流れ（`WindowWrapper::update` → `render`）

```
(イベント) handle_key → input.key() → scene.queue_inputs()   … 操作の押下・解放を待ち行列に積む

update(device, queue)
  ├─ カメラ更新 → camera_buffer に書き込み
  ├─ scene.poll_file()      … 200ms ごとにファイルの更新日時を確認し、変わっていればリロード
  ├─ scene.advance(size)    … 経過した実時間ぶん tick を進める（1/60 秒の固定ステップ）。
  │                            tick ごとに Scene::step（待ち行列の入力は最初の tick でまとめて処理）
  ├─ scene.build(...)       … シーンを評価して RectInstance の列と背景色を作る
  └─ ui_layer.upload(...)   … 画面サイズ（論理ピクセル）とインスタンスを GPU に送る
render(...)
  ├─ クリア（シーンの background があればその色、なければカーソル位置の HSV）
  ├─ 3D: 五角形 ×100 を描画
  └─ UI: ui_layer.draw()     … 同じレンダーパスで 3D の上に重ねる
```

### 4.5 3D 描画パイプライン（チュートリアル由来）

- `TriangleList`, `Ccw` が表, 背面カリング有効, `BlendState::REPLACE`。
- 深度: `Depth32Float`, `CompareFunction::Less`, 毎フレーム 1.0 でクリア。

| group | binding | 内容 | 所有者 | 可視性 |
|---|---|---|---|---|
| 0 | 0 | `t_diffuse: texture_2d<f32>` | AppView | FRAGMENT |
| 0 | 1 | `s_diffuse: sampler` | AppView | FRAGMENT |
| 1 | 0 | `camera: CameraUniform { view_proj: mat4x4 }` | WindowWrapper | VERTEX |

| slot | step | 構造体 | location: 型 |
|---|---|---|---|
| 0 | Vertex | `Vertex` (32B) | 0: position `vec3`, 1: color `vec3`, 2: texture_coords `vec2` |
| 1 | Instance | `ModelInstanceRaw` (72B) | 5: model_id `u32`, 6: texture_id `u32`, 7–10: model行列 `vec4×4` |

- location 3,4 は将来の頂点属性（法線等）用に空けてある。
- `model_id` / `texture_id` はシェーダに渡しているが**未使用**（将来のテクスチャ配列などのため）。
- カメラ: eye=(0,1,2) から原点を見る透視投影。`CameraController` は WASD/矢印で前後移動と周回。

### 4.6 UI 描画パイプライン（`ui/renderer.rs`, `ui.wgsl`）

- **頂点バッファは使わない**。`vertex_index`（0..6）から四角形の角を作り、矩形ごとのデータはインスタンスとして渡す。`draw(0..6, 0..矩形数)` を1回。
- 座標は**論理ピクセル**（左上原点・y 下向き）。シェーダの uniform に論理サイズ（物理サイズ ÷ scale_factor）を渡して NDC に変換する。
- アルファブレンド有効、カリングなし。
- 3D と同じレンダーパスで描くため、深度アタッチメントの形式は合わせるが `depth_compare: Always` / 書き込みなしで常に手前に出る。
- 色はシーンでは 0–255 の sRGB で書き、`RectInstance` に入れる時点で線形に変換する（サーフェスが sRGB のため）。

| group | binding | 内容 | 所有者 |
|---|---|---|---|
| 0 | 0 | `screen: Screen { size: vec2, _padding: vec2 }` | UiLayer（ウィンドウ毎） |

| slot | step | 構造体 | location: 型 |
|---|---|---|---|
| 0 | Instance | `RectInstance` (36B) | 0: position（左上）`vec2`, 1: size `vec2`, 2: color（線形 RGBA）`vec4`, 3: rotation（ラジアン、中心まわり）`f32` |

インスタンスバッファは足りなくなったら2のべき乗の容量で作り直す（`UiLayer::upload`）。

## 5. 外部データとホットリロード

### 5.1 実行時に読むファイル（すべて**カレントディレクトリ相対**、読み込みは `assets.rs` 経由）

| パス | 読む場所 | 用途 | リロード |
|---|---|---|---|
| `ui/menu_a.ui`, `ui/menu_b.ui` | `ScenePlayer::reload` | UI シーン | **保存で自動** + `R` |
| `src/ui.wgsl` | `UiRenderer::build_pipeline` | UI シェーダ | `R` |
| `src/shader.wgsl` | `ShaderWrapper::load` | 3D シェーダ | `R` |
| `pentagon.json` | `AppView::load_model_and_create_buffer` | 3D モデル | `R` |
| `happy-tree.png` | `AppView::new` | 3D テクスチャ | なし |

→ **リポジトリのルートで `cargo run` する前提**。

### 5.2 失敗しても落ちない仕組み

- **シーンファイル**: パースやコンパイルに失敗したら `log::error!` に `パス:行:列: メッセージ` を出し、**直前の正常なシーンのまま動き続ける**。ファイルが読めない場合も同じ。
- **シェーダとパイプライン**: `shader::capture_validation()` が wgpu のエラースコープで検証エラーを捕まえ、パニックではなく `Err` にする。失敗したら差し替えない。
  - 3D: `ShaderWrapper::load` と `constuct_render_pipeline` の両方を包んでいる。
  - UI: `UiRenderer::build_pipeline` 全体を包んでいる。
- `AppView::reload`（`R` キー）は各ステップの失敗を `report_err` でログに出すだけで、残りのステップは続行する。

### 5.3 状態の保持

`ScenePlayer` は**定義（`Scene`）と実行時状態（`tick` と `Runtime`）を分けて持つ**（PLAN.md §3.4）。
- `Runtime` = 今の状態・その状態に入った tick・変数の値。
- リロードでは `Scene::adopt` が、**同じ名前の状態と変数を引き継いだ** `Runtime` を作る。今の状態がファイルから消えていたら初期状態から始める。新しい変数は初期値になる。
- `time` は続きから進む。`time` は `tick / 60` で求める。tick は `ScenePlayer::advance` が実時間を積算して進める（最大 250ms 分まで。処理落ちで固まらないため）。

### 5.4 3D モデルファイル形式

`Model { vertices: Vec<Vertex>, indices: Vec<u16> }` を serde でシリアライズ。
- **S式（採用中）**: `save_sexpr_file` / `load_sexpr_file`。`pentagon.json` は拡張子が json だが中身は S式。
- **バイナリ（未使用）**: `save_to_binaryfile` / `load_from_binary_file`。
- `vertex::pentagon()` はハードコードの元データ。`load_sexpr_file` 内の `reset` フラグを true にするとファイルを再生成する（開発用スイッチ）。

## 6. DSL

### 6.1 構文の考え方

構造は S式（Lisp 風、`;;` 行コメント）、数式だけ `{ ... }` で囲んだ中置記法。前置記法で数式を書く煩わしさを避けるハイブリッド。

### 6.2 パーサー（`parser.rs`）

```
&str ──Lexer──▶ Vec<Token{kind, span}> ──Parser──▶ Vec<AstNode{kind, span}>
```

- 公開 API: `parser::parse(source) -> ParseResult<Vec<AstNode>>`、`parser::line_col(source, index) -> (行, 列)`（1始まり）。
- **Lexer**: `( ) { } + - * / ! , == != < <= > >=`, 整数, 浮動小数（`1.5`）, 文字列（`\n \t \" \\`）, シンボル（先頭 `[A-Za-z_?]`, 以降 `[A-Za-z0-9_\-?!]`）。単独の `=` はエラー。
- **AstNode**: `{ kind: AstKind, span: Span }`。`AstKind` は `Form{head, args}` / `Expr(Expr)` / `Integer` / `Float` / `String` / `Symbol`。Form の head は必ずシンボル。
- **負のリテラル**: 数値に密着した `-`（`-12`）はフォーム中でも負の数として読む。離れた `- 12` は不可。
- **Expr**: Pratt パーサー。左結合。`f(a, b)` 形式の関数呼び出しと `( )` によるグルーピングに対応。

  | 優先度（弱い → 強い） | 演算子 | binding power |
  |---|---|---|
  | 比較（結果は 1 か 0） | `== != < <= > >=` | (1, 2) |
  | 加減 | `+ -` | (3, 4) |
  | 乗除 | `* /` | (5, 6) |
  | 単項 | `- !` | 100 |

- **Span**: char インデックス。エラー表示と、将来のソースへの書き戻しに使う。
- エラーは Lexer・Parser とも `ParseError { message, span }`。

### 6.3 UI シーン言語（`ui/scene.rs`）

```lisp
(defparam item_w 260 (Float 100 400 _))   ;; 調整用の値: 名前 既定値 (型 最小 最大 刻み)。`_` または省略は指定なし
(var cursor 0)                            ;; 実行時の変数: 名前 初期値。リロードしても値を引き継ぐ
(state closed                             ;; 状態。最初に書いた state が初期状態
  (on (press menu) (set cursor 0) (goto open)))
(state open
  (on (press down) (set cursor {wrap(cursor + 1, 4)}))    ;; goto なし = 状態はそのまま
  (on (press confirm) (when {cursor == 2}) (goto closed)) ;; when = 条件
  (on (release menu) (goto closed))
  (on (after 2) (goto closed)))           ;; この状態に入ってから 2 秒後（goto 必須）
(def top {height / 2 - 120})              ;; 毎フレーム計算される名前付きの値
(background (rgb 24 24 32))               ;; 省略するとカーソル位置の色
(rect x y w h (rgb 255 0 0) (rotate 15) (visible open))  ;; 左上 x y、幅 高さ、色、[回転（度）]、[表示条件]
```

| 要素 | 内容 |
|---|---|
| 数値が書ける場所 | リテラル / 名前 / `{中置式}` |
| 色 | `(rgb r g b)` / `(rgba r g b a)`。すべて 0–255 |
| パラメータ型 | `Int` / `Float`（現状どちらも f32 として扱う） |
| 組み込みの値 | `time`（秒）, `state_time`（今の状態に入ってからの秒）, `width`, `height`（論理ピクセル）, `pi` |
| 状態名 | 式の中では、その状態にいれば 1、いなければ 0（`(visible open)`, `{page * (screen == 0)}`） |
| 関数 | `sin` `cos`（ラジアン）, `abs` `floor` `min` `max` `clamp(x, lo, hi)` `lerp(a, b, t)` `wrap(x, n)`（0..n に折り返す） `select(条件, 真, 偽)` `smoothstep(端0, 端1, x)` |
| 操作（`press` / `release` の引数） | `menu` `up` `down` `left` `right` `confirm` `cancel`（キー配置は §8） |

**ステートマシンの規則**（`Scene::step`）:
- 1 tick ごとに、たまっている入力を**届いた順に**1つずつ処理し、最後に `after` を調べる。
- 1つの入力（または `after` の確認）につき、今の状態の `on` を上から見て、**最初に当てはまった1つだけ**を実行する。
- `set` はすべての値を同じ環境で評価してから代入する（`set` を書く順番に結果が左右されない）。
- `goto` すると `state_time` が 0 に戻る。自分自身への `goto` でも戻る。`goto` のない遷移では戻らない。
- `after` に `goto` がないとコンパイルエラーになる（条件を満たしている間、毎 tick 発火してしまうため）。
- どの遷移にも当てはまらなかった**押下**は「無視された入力」として報告する（解放は報告しない）。

**名前の解決規則**:
- `defparam` / `var` / `state` は1周目で集めるので、**ファイル内のどこからでも参照できる**。
- `def` は上から順に処理するので、**def から別の def を使うときは、使うより前の行で定義する**。状態の中身や `rect` からは、どの def も参照できる。
- 同じ名前の二重定義、組み込みの値や `pi` と同じ名前はエラー。`set` できるのは `var` だけ。

**コンパイルと評価**（CLAUDE.md の「毎フレーム辞書を引かない」原則）:
- リロード時に AST を `Scene` にコンパイルし、名前は**スロット番号**に解決しておく（`CExpr::Slot`）。
- スロットの並び: `[time, width, height, state_time, パラメータ..., 変数..., 状態フラグ..., def...]`。
- 評価のたびに `Scene::fill_env` で f32 の配列を作り（def は順に評価して追加）、`CExpr::eval` は配列を引くだけ。

**StepEvent**: `step` は起きたこと（遷移・goto のない遷移・無視された入力）を `StepEvent` としてコールバックに渡す。いまは `ScenePlayer` が tick 番号つきで `log::info!` に出しているだけ。操作ログ（PLAN.md §4.5）はここに記録処理を足して作る。

**Param は書き戻しの準備ができている**: `Param` は型・最小・最大・刻みと、既定値リテラルの位置（`value_span`）を持つ。スライダー化とソースへの書き戻しにはまだ使っていない。

### 6.4 パーサー単体の確認

`cargo run --bin parser [ファイル]` で AST を表示する。引数なしでは、当初の構想を書いたサンプル（`paint`, `defvar`, 行列リテラルなど）をパースする。
このサンプルは `()` と `((1 2 0) ...)`（head がシンボルでないリスト）の行で `expected symbol` エラーになる（未対応の構文）。

## 7. 未接続: テキスト描画（`src/text.rs`）

`GlyphonText`: glyphon の FontSystem/SwashCache/TextAtlas/TextRenderer/Buffer をまとめたラッパー。
- 32px / 行間42px、左上 (24,24) マージン、白文字、SansSerif、日本語サンプル文。
- API: `new(device, queue, format, w, h)` / `set_text` / `resize` / `prepare(device, queue, w, h)` / `render(&mut RenderPass)`。
- 統合するなら WindowWrapper ごとに持ち、`render()` 内で描画前に `prepare`、同じレンダーパスの最後に `render` を呼ぶ形が自然。
  ただしレンダーパスは深度アタッチメント付きなので、`TextRenderer::new` の depth_stencil 引数（現在 `None`）を合わせる必要がある。
- 最初の用途の候補: シーンのエラーを画面に表示する（PLAN.md §3.4）。

## 8. ビルド / 実行 / テスト

```bash
cargo run                  # ネイティブ。リポジトリ直下で実行すること（相対パス読込のため）
cargo run --bin parser     # DSL パーサー単体（AST を Debug 出力）
cargo test --lib           # input と ui::scene のテスト（同梱の ui/*.ui を読み込んで操作の流れを試すテストを含む）
```

**キー配置**（`input.rs` の `action_for_key`）:

| キー | 操作 / 機能 |
|---|---|
| Space | `menu` |
| ↑ ↓ ← → | `up` `down` `left` `right` |
| Enter / Z | `confirm` |
| Backspace / X | `cancel` |
| WASD | 3D カメラ |
| R | 全体のホットリロード |
| Esc | 終了 |

**2つのメニュー方式**（操作したいウィンドウをクリックしてフォーカスしてから）:

| | 方式A（`ui/menu_a.ui`, ウィンドウ1） | 方式B（`ui/menu_b.ui`, ウィンドウ2） |
|---|---|---|
| 開く | Space を押している間だけ表示（中心の上下左右に項目が広がる） | Space で開く / 閉じる |
| 選ぶ | Space を押したまま矢印キー → その方向の画面へ | ↑↓ で選び、Enter / Z で決定 |
| 戻る | 画面で Backspace / X → 閉じる | Backspace / X → 1つ戻る（画面 → メニュー → 閉じる） |

画面は4色（赤・青・緑・黄）で区別し、両方式で同じ遷移アニメーション（`page_time`）にしてある。
`ui/*.ui` は保存するだけで反映される。マウス移動で背景色が変わるのは、シーンが `background` を指定していないときだけ。

## 9. 既知の問題・設計上の注意点

**意図的なのか未着手なのか**も併記。

| # | 内容 | 場所 | 状態 |
|---|---|---|---|
| 1 | camera の BindGroupLayout が `ShaderWrapper` と `WindowWrapper` で**二重定義**。パイプラインは前者、bind group は後者のレイアウトで作られている（内容が同一なので動く） | `ShaderWrapper::new`, `WindowWrapper::new` | 未解決の設計課題 |
| 2 | windows[0] に surface を2回作成（adapter 選択用に1つ、WindowWrapper で1つ）。前者は捨てられる | `AppView::new`, `WindowWrapper::new` | 冗長だが動作する |
| 3 | リサイズ時にカメラの `aspect` を更新していない | `WindowWrapper::resize` | 未対応 |
| 4 | surface の configure は `Resized` イベント待ち。来なければ描画されない | `WindowWrapper::resize`, `render` | チュートリアル準拠 |
| 5 | パイプライン（3D・UI とも）は windows[0] の surface format で作られ全ウィンドウ共用 | `AppView::new`, `reconstruct_render_pipeline` | 前提として許容 |
| 6 | **wasm は動かない**: `std::fs` でファイルを読む（`assets.rs`）、`std::time::Instant` が wasm32 ではパニックする（`ui/mod.rs`）、`capture_validation` の `pollster::block_on` は WebGPU バックエンドでは待てない可能性がある、`run.html` の import 先 `./pkg/tutorial1_window.js` が実際の出力 `learn_wgpu2.js` と違う、wasm 側でも window2 を作って使っていない。さらに WebGL では canvas をまたいで device を共有できない（PLAN.md §6） | 各所 | 移植時にまとめて対応 |
| 7 | シンボルに `-` を含められるため、`{a-b}` は名前 `a-b` として読まれる。引き算は `{a - b}` と空白を入れる。サンプルの名前は snake_case にしている | `parser.rs` `is_ident_continue` | DSL 設計上の注意 |
| 8 | `Expr`（`{}` の中）は位置を持たないので、式の中のエラーは囲んでいるノード全体の位置で報告される | `ui/scene.rs` `Compiler::expr` | 必要になったら Expr に Span を足す |
| 9 | シーンファイルの変更検知は 200ms ごとの更新日時のポーリング（`notify` 等は未使用） | `ScenePlayer::poll_file` | 意図的（依存を増やさない、移植しやすい） |
| 10 | `time` と入力の処理はウィンドウごとの tick で、そのウィンドウが再描画されたときにだけ進む | `ScenePlayer::advance` | 現状は問題なし |
| 11 | 1フレームの間に複数の tick が進んだ場合、たまっていた入力はすべて最初の tick で処理される（入力の時刻の分解能は「フレーム」） | `ScenePlayer::advance` | 操作ログで精密な時刻が必要になったら、入力に時刻を持たせる |
| 12 | 方式A と方式B で画面の色や遷移のパラメータを共有する仕組み（include など）がないので、両方のファイルに同じ記述がある | `ui/menu_a.ui`, `ui/menu_b.ui` | 未対応 |
| 13 | `R` キーで壊れた WGSL を読んだときに落ちないことは、コード上は対処済みだが手動では未確認 | `shader::capture_validation` | 要確認 |
| 14 | `TextureWrapper::new` は失敗時に `expect`/`unwrap` でパニック（他は anyhow） | `texture.rs` | エラー処理が不統一 |
| 15 | `constuct_render_pipeline` は綴り誤り（construct）。呼び出し側も同名 | `appview.rs` | 命名 |
| 16 | 未使用 import が多い（`wgpu::wgc::device` など）。`Param` の `kind`/`min`/`max`/`step`/`value_span` は次の段階（スライダー）まで未使用 | 各所 | 警告のみ |

## 10. 次の段階（PLAN.md §4.6）

1. **パラメータのスライダー化とソースへの書き戻し**: `Param` の範囲と `value_span` を使う。ツール自体の UI（スライダー）を自作するか egui などを借りるかは未決定（PLAN.md §7）。
2. **操作ログ**: `StepEvent` を tick つきで記録し、方式ごとに集計する（操作完了までの時間、手数、無視された入力、待たされた時間など。PLAN.md §4.5）。操作の目的（「青の画面を開く」など）を与える仕組みも要る。
3. 必要になったら: 記録した入力の再生、ログのタイムライン表示、文字の描画（`text.rs`）、シーン間で共通部分を共有する仕組み。

## 11. Claude 向け作業メモ

- コード変更前に「共有リソースか（→`AppView`）／ウィンドウ固有か（→`WindowWrapper`）」を判断して置き場所を決める。
- ファイルを読むときは `assets.rs` を通す。新しい外部ファイルはホットリロードに乗せるかを検討し、乗せるなら失敗しても落ちない形にする（シーンは `ScenePlayer` 方式、GPU 資源は `capture_validation` で包んで成功時だけ差し替え）。
- 新しい GPU 用構造体は `#[repr(C)]` + `bytemuck::Pod/Zeroable`。シェーダの `@location` / `@group` と必ず同期させる（3D は `Vertex::LAYOUT` / `ModelInstanceRaw::desc()`、UI は `RectInstance::LAYOUT`）。
- 3D の bind group を追加したら `ShaderWrapper::bind_group_layouts`（index = group 番号）に追加する。UI 側は `UiRenderer` が自前でレイアウトを持つ。
- DSL に新しいフォームや関数を足すときは、`ui/scene.rs` の `Scene::compile` / `Compiler` / `Func::lookup` を更新し、テストを足す。名前はコンパイル時にスロットへ解決し、毎フレーム名前で検索しない。スロットの並びを変えたら `Scene` のコメントと §6.3 も直す。
- シーンはキーを直接見ない。新しい入力（キー、パッド、タッチ）は `input.rs` で `Action` に変換して渡す。操作を増やすときは `Action` と `action_for_key` を更新する。
- 実行時の状態は `Runtime` に置き、`Scene` には置かない（リロードで消えないように）。状態を足したら `Scene::adopt` で引き継ぎ方を決める。
- `ui/*.ui` を変えたら `cargo test --lib` を回す（同梱のシーンを読み込むテストがある）。
- コメントは日本語と英語が混在（チュートリアル由来の英語コメントはそのまま残っている）。
