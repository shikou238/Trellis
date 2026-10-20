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
  ウィンドウ3の**インスペクタ**で、両方のシーンの `defparam` をスライダーで調整でき、離すとソースに書き戻される。
  シーンのウィンドウで `T` を押すと**計測モード**になり、方式A と方式B を交互に出題して操作を `logs/` に記録し、終わるとインスペクタに集計が出る。
- チュートリアル由来の 3D シーン（テクスチャ付き五角形 ×100）は残してあり、UI はその上に重ねて描く。「ゲーム画面の上にメニューが出る」状況の背景として使える。
- **古いファイル**: `src/text.rs` は古い glyphon の API で書かれた未接続のラッパー（`lib.rs` で `// mod text;`）。文字描画は `src/ui/text.rs` で作り直した。

## 2. 技術スタック

| 用途 | クレート | 備考 |
|---|---|---|
| GPU | `wgpu 28` | wasm時は `webgl` feature、`Backends::GL` |
| ウィンドウ/イベント | `winit 0.30` | `ApplicationHandler` 方式 |
| 線形代数 | `cgmath 0.18` | |
| GPUへのバイト変換 | `bytemuck` | `Pod`/`Zeroable` derive |
| 画像 | `image 0.24`（png/jpeg のみ） | |
| モデル保存形式 | `serde` + `serde-lexpr`/`lexpr`（バージョン固定 `=`） | S式でシリアライズ |
| 文字描画 | `glyphon 0.10`（内部で cosmic-text 0.15） | `ui/text.rs`。wgpu 28 を共有（Cargo.lock 上 wgpu は1バージョンのみ） |
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
  assets.rs          ファイル読み書きの窓口（読み込み・書き戻しはすべてここを通す）
  input.rs           入力の2層化: キー → 抽象的な操作（Action）の押下・解放（InputEvent）
  parser.rs          DSL のレキサー/パーサー（S式 + {中置式}）
  ui/
    mod.rs           ScenePlayer: シーンファイルの読込・自動リロード・固定タイムステップ・入力の受け渡し
    scene.rs         Scene: DSL → コンパイル済みシーン、ステートマシンの実行（step）、毎フレームの評価。Runtime: 実行時の状態
    renderer.rs      UiRenderer（共有パイプライン）/ UiLayer（ウィンドウ毎のバッファ）/ RectInstance / srgb8()
    text.rs          TextSystem（共有フォント）/ TextLayer（ウィンドウ毎の文字描画）/ Label
    inspector.rs     Inspector: パラメータのスライダーの配置・描画・ドラッグ、計測結果の表示
    session.rs       TestSession: 計測モード（出題・完了判定・ログの書き出し）、TestConfig（ui/tasks.ui）
    analysis.rs      計測ログの読み込みと集計（Report, TrialRecord, SceneStats）
  ui.wgsl            UI 用 2D 矩形シェーダ（実行時にファイルから読む）
  camera.rs          Camera, CameraUniform, OPENGL_TO_WGPU_MATRIX
  model_instance.rs  ModelInstance / ModelInstanceRaw（3D のインスタンシング用）
  vertex.rs          Vertex, Model, ModelBuffer、モデルの保存/読込
  texture.rs         TextureWrapper（画像テクスチャ, 深度テクスチャ）
  shader.rs          ShaderWrapper（3D 用 WGSL 読込・BindGroupLayout 定義・リロード）, capture_validation()
  shader.wgsl        3D 用シェーダ（実行時にファイルから読む）
  text.rs            [古い・未接続] 旧 glyphon API のラッパー。ui/text.rs に置き換え済み
  bin/parser.rs      パーサー単体の動作確認用バイナリ
ui/
  menu_a.ui          ウィンドウ1のシーン（方式A: 押している間だけ出る十字型メニュー）
  menu_b.ui          ウィンドウ2のシーン（方式B: 開いて選んで決定する縦並びメニュー）
  tasks.ui           計測の設定（交互に試すシーン、試行回数、制限時間、課題）
logs/                計測のログ（.gitignore 済み。1 回の計測につき 1 ファイル）
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
      ├─ text_system: TextSystem                 (フォント・グリフのキャッシュ。読み込みが重いので共有)
      ├─ report: Vec<String>                     (直近の計測の集計。インスペクタに出す)
      ├─ inspector: Inspector + inspector_window (ドラッグ中のパラメータ / インスペクタの WindowId)
      └─ windows: Vec<WindowWrapper>             ← ウィンドウ毎のリソース
           ├─ nature: Arc<winit::Window>
           ├─ surface + config + is_surface_configured + depth_texture
           ├─ camera 一式 + camera_controller
           ├─ last_cursor_position
           ├─ input: InputMapper                 (押されているキー → 操作の押下・解放)
           ├─ scene: Option<ScenePlayer>         (シーン定義 Scene + 実行時状態 tick / Runtime + 入力の待ち行列)
           │                                      None = シーンを動かさないウィンドウ（インスペクタ）
           ├─ session: Option<TestSession>       (計測中なら Some。計測用に別の ScenePlayer を持ち、scene の代わりに動かす)
           ├─ ui_layer: UiLayer                  (画面サイズ uniform + 矩形インスタンスバッファ)
           ├─ text_layer: TextLayer              (文字のアトラス・描画パイプライン + 今フレームの Label 列)
           └─ background                         (背景色)
```

**設計意図**: チュートリアルの `State` を2つに分けている。
- **共有すべきもの**（device・パイプライン・メッシュ・テクスチャ・フォント）は `AppView` に1つ。
- **ウィンドウ固有のもの**（surface・深度バッファ・カメラ・入力状態・UI シーン・文字のアトラス）は `WindowWrapper` に。
  ネイティブでは起動時に3ウィンドウを作り（`App::resumed`）、1・2 番目はそれぞれ別のシーンファイルを読む（`appview.rs` の `SCENE_PATHS`）。3 番目はインスペクタ。
- **ウィンドウをまたぐ処理**（インスペクタが他のウィンドウのシーンを読み書きする）は `AppView` のメソッドに置く。ウィンドウの指定は `windows` のインデックスではなく `WindowId` で行う（ウィンドウを閉じると `windows` の並びがずれるため）。

命名規約: wgpu/winit の生オブジェクトをラップした構造体は `XxxWrapper`、ラップ対象のフィールド名は **`nature`**。

### 4.2 起動シーケンス

```
main() → lib::run()
  ├─ ロガー初期化
  ├─ EventLoop::with_user_event()     (ユーザーイベント型 = AppView)
  └─ run_app(App)
       resumed()
         ├─ (wasm) <canvas id="canvas"> を取得して window に割当
         ├─ native: 「方式A」「方式B」「インスペクタ」の3ウィンドウを作成
         │          pollster::block_on(AppView::new(vec![w1, w2], Some(inspector)))
         └─ wasm  : spawn_local で AppView::new(vec![w1], None) → proxy.send_event(AppView)
                     → user_event() で受け取り state にセット + 初回 resize
```

wasm では async を block できないので、生成した `AppView` 自体を **ユーザーイベントとして** イベントループに送り返す（チュートリアル準拠）。

`AppView::new` の流れ:
1. `Instance` 作成 → windows[0] で一時 surface を作り、それと互換な adapter を取得
2. sRGB の surface format を選択（なければ先頭）
3. device/queue 取得（wasm は `downlevel_webgl2_defaults`）
4. `ShaderWrapper::new` → `constuct_render_pipeline`（3D）
5. `pentagon.json` → `ModelBuffer`、`happy-tree.png` → `diffuse_bind_group`
6. `UiRenderer::new`（UI パイプライン）、`TextSystem::new`（フォント読み込み）
7. 各 window から `WindowWrapper::new`（surface/カメラ/深度/`ScenePlayer`/`UiLayer`/`TextLayer` を作成。インスペクタは `ScenePlayer` なし。surface はこの時点では未configure）
8. 10×10 グリッドのインスタンス生成 → インスタンスバッファ

### 4.3 イベント処理（`App::window_event`）

`window_id` から `windows` 内のインデックスを引き、そのウィンドウにだけ処理を流す。

| イベント | 処理 |
|---|---|
| `CloseRequested` | そのウィンドウを `windows` から除去。シーンを動かすウィンドウが残っていなければアプリ終了 |
| `Resized` | surface 再configure + 深度テクスチャ再生成。**ここで初めて `is_surface_configured = true`** |
| `KeyboardInput` | `Esc`=終了、`R`=ホットリロード（全体）。その後そのウィンドウの `handle_key` へ（CameraController と InputMapper の両方に渡す。OS のキーリピートかどうかも渡す） |
| `Focused(false)` | `focus_lost`: 押されたままのキーをすべて離したことにする（解放が届かず押しっぱなしになるのを防ぐ） |
| `RedrawRequested` | （インスペクタなら先に `AppView::build_inspector`）→ `WindowWrapper::update()` → `render()`。Lost/Outdated なら resize で復旧 |
| `CursorMoved` | カーソル位置を保持（背景色に使用）。インスペクタなら `AppView::inspector_drag`（ドラッグ中の値を更新） |
| `MouseInput`（左ボタン, インスペクタのみ） | `AppView::inspector_mouse`: 押したらドラッグ開始、離したらソースに書き戻す |
| `KeyboardInput` の `T`（リピートは除く） | `AppView::toggle_session`: そのウィンドウで計測を始める / やめる |

`RedrawRequested` のたびに `AppView::collect_finished_sessions` が、終わった計測のログを読んで集計し `report` に入れる。
計測中のウィンドウでは、キー入力は `scene` ではなく `session` に渡る（`WindowWrapper::queue_inputs`）。

キー入力はフォーカスのあるウィンドウにだけ届くので、2つの方式は**ウィンドウごとに独立して**動く。

`render()` の先頭で `request_redraw()` を呼ぶため、各ウィンドウは**常時連続描画**ループになる。

### 4.4 1フレームの流れ（`WindowWrapper::update` → `render`）

```
(イベント) handle_key → input.key() → scene.queue_inputs()   … 操作の押下・解放を待ち行列に積む

(インスペクタのみ) AppView::build_inspector()
  └─ 全シーンの SceneSummary を集め、Inspector::build で矩形と Label を作って
     インスペクタの ui_layer.instances / text_layer.labels に入れる

update(device, queue, text_system)
  ├─ カメラ更新 → camera_buffer に書き込み
  ├─ (計測中なら) session.update(size) → session.build(...)   … シーンの代わりに計測を進めて描く（§5.5）
  ├─ (そうでなく、シーンがあれば)
  │   ├─ scene.poll_file()    … 200ms ごとにファイルの更新日時を確認し、変わっていればリロード
  │   ├─ scene.advance(size)  … 経過した実時間ぶん tick を進める（1/60 秒の固定ステップ）。
  │   │                          tick ごとに Scene::step（待ち行列の入力は最初の tick でまとめて処理）
  │   └─ scene.build(...)     … シーンを評価して RectInstance の列と背景色を作る
  ├─ ui_layer.upload(...)     … 画面サイズ（論理ピクセル）とインスタンスを GPU に送る
  └─ text_layer.prepare(...)  … Label をシェイピング（文字列が変わったものだけ）してアトラスに載せる
render(...)
  ├─ クリア（background があればその色、なければカーソル位置の HSV）
  ├─ 3D: 五角形 ×100 を描画（シーンのあるウィンドウだけ）
  ├─ UI: ui_layer.draw()      … 同じレンダーパスで 3D の上に重ねる
  └─ 文字: text_layer.draw()  … さらにその上
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
- カメラ: eye=(0,1,2) から原点を見る透視投影。`CameraController` は WASD で前後移動と周回（矢印キーは UI 用）。

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
Rust 側で色を作るときは `renderer::srgb8(r, g, b, a)`（0–255 の sRGB → 線形）を使う。

### 4.7 文字描画（`ui/text.rs`）

- glyphon を使う。**フォント（`FontSystem`）・`SwashCache`・`Cache` は全ウィンドウで共有**（`TextSystem` → AppView）。`FontSystem::new()` はシステムフォントを読むので重い。
- **アトラス・`TextRenderer`・`Viewport` はウィンドウ毎**（`TextLayer` → WindowWrapper）。
- 使い方: 毎フレーム `text_layer.labels` に `Label { text, position, size, color }` を詰め直す → `update` 内の `prepare` → `render` 内の `draw`。
  - 位置とサイズは**論理ピクセル**。glyphon には物理ピクセルで渡す（`TextArea` の `left/top` に scale_factor を掛け、`scale` に scale_factor を入れる）。
  - 色は sRGB 0–255。
  - 前フレームと同じ文字列・サイズの Label は、シェイピングし直さない（`TextLayer::shaped` にキャッシュ）。Label の並び順が変わるとキャッシュが効かない。
- UI と同じく、レンダーパスの深度アタッチメントに合わせて `depth_compare: Always` / 書き込みなしのパイプラインにしている。
- 現在の用途はインスペクタだけ（シーンの DSL から文字を出す機能はまだない）。

## 5. 外部データとホットリロード

### 5.1 実行時に読むファイル（すべて**カレントディレクトリ相対**、読み書きは `assets.rs` 経由）

| パス | 読む場所 | 用途 | リロード |
|---|---|---|---|
| `ui/menu_a.ui`, `ui/menu_b.ui` | `ScenePlayer::reload` | UI シーン。**インスペクタから書き戻される**（`ScenePlayer::write_param`） | **保存で自動** + `R` |
| `ui/tasks.ui` | `TestSession::start` | 計測の設定 | `T` で計測を始めるたびに読む |
| `logs/session-*.log` | `TestSession`（書く）/ `AppView::collect_finished_sessions`（読む） | 計測のログ | — |
| `src/ui.wgsl` | `UiRenderer::build_pipeline` | UI シェーダ | `R` |
| `src/shader.wgsl` | `ShaderWrapper::load` | 3D シェーダ | `R` |
| `pentagon.json` | `AppView::load_model_and_create_buffer` | 3D モデル | `R` |
| `happy-tree.png` | `AppView::new` | 3D テクスチャ | なし |

→ **リポジトリのルートで `cargo run` する前提**。

### 5.2 失敗しても落ちない仕組み

- **シーンファイル**: パースやコンパイルに失敗したら `log::error!` に `パス:行:列: メッセージ` を出し、**直前の正常なシーンのまま動き続ける**。ファイルが読めない場合も同じ。
  エラーは `ScenePlayer::last_error` にも残り、**インスペクタに赤字で表示**される（次に読み込みが成功すると消える）。
- **シェーダとパイプライン**: `shader::capture_validation()` が wgpu のエラースコープで検証エラーを捕まえ、パニックではなく `Err` にする。失敗したら差し替えない。
  - 3D: `ShaderWrapper::load` と `constuct_render_pipeline` の両方を包んでいる。
  - UI: `UiRenderer::build_pipeline` 全体を包んでいる。
- `AppView::reload`（`R` キー）は各ステップの失敗を `report_err` でログに出すだけで、残りのステップは続行する。

### 5.3 状態の保持

`ScenePlayer` は**定義（`Scene`）と実行時状態（`tick` と `Runtime`）を分けて持つ**（PLAN.md §3.4）。
- `Runtime` = 今の状態・その状態に入った tick・変数の値。
- リロードでは `Scene::adopt` が、**同じ名前の状態と変数を引き継いだ** `Runtime` を作る。今の状態がファイルから消えていたら初期状態から始める。新しい変数は初期値になる。
- `time` は続きから進む。`time` は `tick / 60` で求める。tick は `ScenePlayer::advance` が実時間を積算して進める（最大 250ms 分まで。処理落ちで固まらないため）。

### 5.4 インスペクタとソースへの書き戻し（`ui/inspector.rs`, `Scene::rewrite_param`）

```
左ボタンを押す   Inspector::press      … カーソルの下の行のパラメータでドラッグ開始
                                         （範囲のあるスライダーは押した位置の値にすぐ合わせる）
カーソル移動     Inspector::value_at   → ScenePlayer::set_param   … メモリ上の値だけ変える（即座に画面に反映）
左ボタンを離す   Inspector::release    → ScenePlayer::write_param … ファイルに書き戻す
                                         → 保存を検知して自動リロード（状態は引き継ぐ）
```

- **値の決め方**（`Inspector::value_at`）:
  - 範囲（min と max）があれば、トラック上のカーソル位置 → 値。
  - なければ、ドラッグ開始位置からの移動量 × 1 ピクセルあたりの変化量（`step` があればその値、`Int` は 0.25、`Float` は開始時の値の大きさ / 200）。
  - 最後に `Param::snap` で、`step`（`min` を基準）・整数・範囲に合わせる。
- **書き戻し**（`Scene::rewrite_param`）: 書き戻すたびに**ファイルを読み直してコンパイルし、名前でパラメータを探して**、既定値リテラルの `value_span` の部分だけを `Param::format_value` の文字列に置き換える。コメントや書式はそのまま残る。
  ファイルがコンパイルできない（編集途中など）・パラメータが見つからない場合は書き込まずにエラーをログに出す。
- **書式**（`Param::format_value`）: `Int` は整数、`Float` は `step` の桁数（なければ小数3桁まで）で、末尾の 0 は省く。
- **配置**（`inspector::layout`）: シーンごとに「見出し（パス + 今の状態名）→（エラーがあれば赤字）→ パラメータの行…」を上から並べる。描画と当たり判定で同じ配置を使う。
- インスペクタはシーンを持たないウィンドウ。中身は `AppView::build_inspector` が毎フレーム作り直す。

### 5.5 計測モード（`ui/session.rs`, `ui/analysis.rs`, `ui/tasks.ui`）

PLAN.md §4.5 の「操作ログ」。触った感覚の違いを、データで分析するためのもの。

**設定**（`ui/tasks.ui`、`T` を押したときに読む）:
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

### 5.6 3D モデルファイル形式

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
  (on (press confirm) (when {cursor == 2}) (emit opened cursor) (goto closed)) ;; when = 条件、emit = 意味のある通知
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
- `(emit 名前 値...)` の値は `set` と同じく遷移前の環境で評価し、遷移の報告の後に `StepEvent::Emit` として報告する。計測の完了判定に使う（§5.5）。

**名前の解決規則**:
- `defparam` / `var` / `state` は1周目で集めるので、**ファイル内のどこからでも参照できる**。
- `def` は上から順に処理するので、**def から別の def を使うときは、使うより前の行で定義する**。状態の中身や `rect` からは、どの def も参照できる。
- 同じ名前の二重定義、組み込みの値や `pi` と同じ名前はエラー。`set` できるのは `var` だけ。

**コンパイルと評価**（CLAUDE.md の「毎フレーム辞書を引かない」原則）:
- リロード時に AST を `Scene` にコンパイルし、名前は**スロット番号**に解決しておく（`CExpr::Slot`）。
- スロットの並び: `[time, width, height, state_time, パラメータ..., 変数..., 状態フラグ..., def...]`。
- 評価のたびに `Scene::fill_env` で f32 の配列を作り（def は順に評価して追加）、`CExpr::eval` は配列を引くだけ。

**StepEvent**: `step` は起きたこと（遷移・goto のない遷移・無視された入力・通知）を `StepEvent` としてコールバックに渡す。普段は `ScenePlayer::advance` が `log::info!` に出すだけで、計測中は `TestSession` が受け取ってログファイルに書く（`ScenePlayer::step_tick` に渡すコールバック）。

**Param**: 型・最小・最大・刻みと、既定値リテラルの位置（`value_span`）を持つ。インスペクタのスライダーとソースへの書き戻しに使う（§5.4）。値は `Scene` の中にあり、ドラッグ中は `ScenePlayer::set_param` が直接書き換える（リロードするとファイルの値に戻る）。

### 6.4 パーサー単体の確認

`cargo run --bin parser [ファイル]` で AST を表示する。引数なしでは、当初の構想を書いたサンプル（`paint`, `defvar`, 行列リテラルなど）をパースする。
このサンプルは `()` と `((1 2 0) ...)`（head がシンボルでないリスト）の行で `expected symbol` エラーになる（未対応の構文）。

## 7. 古いファイル: `src/text.rs`

`GlyphonText`: 古い glyphon の API（`TextAtlas::new(device, queue, format)`, `Resolution` を `prepare` に渡す形）で書かれたラッパー。glyphon 0.10 ではそのままではコンパイルできないため、`lib.rs` で `// mod text;` のまま未接続。
文字描画は `src/ui/text.rs`（§4.7）で作り直した。日本語サンプル文の表示など、元の意図は `ui/text.rs` の `Label` で実現できる。不要なら削除してよい。

## 8. ビルド / 実行 / テスト

```bash
cargo run                  # ネイティブ。リポジトリ直下で実行すること（相対パス読込のため）
cargo run --bin parser     # DSL パーサー単体（AST を Debug 出力）
cargo test --lib           # input / ui::scene / ui::session / ui::analysis のテスト（同梱の ui/*.ui で操作の流れや計測を試すテストを含む）
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

**計測**: 方式A か方式B のウィンドウで `T` を押す。上部に課題の色（赤・青・緑・黄）が出るので、その画面を開く。方式は試行ごとに交互に切り替わる。
`ui/tasks.ui` の回数に達するか、もう一度 `T` を押すと終わり、ウィンドウは普段の表示に戻る。集計はインスペクタの一番下に出る。

**インスペクタ**（ウィンドウ3）: 両シーンの `defparam` がスライダーとして並ぶ。左ボタンでドラッグすると即座に反映され、離すと `.ui` ファイルに書き戻される。
範囲のあるパラメータはつまみの位置が値、範囲のないものは左右のドラッグ量で増減する。見出しには今の状態名、シーンにエラーがあれば赤字でエラーが出る。

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
| 16 | 未使用 import が多い（`wgpu::wgc::device` など） | 各所 | 警告のみ |
| 17 | `step` のない Float パラメータは、スライダーで書き戻すと小数3桁の値になる（例: `310.136`）。きりのいい値にしたいときは `step` を指定する | `Param::format_value` | 意図的（必要な精度はパラメータごとに違う） |
| 18 | 書き戻しはファイルを外から書き換えるので、エディタで開いていると「外部で変更された」扱いになる。エディタ側で未保存の編集があると衝突する | `ScenePlayer::write_param` | 注意 |
| 19 | インスペクタはスクロールできない。パラメータが多いとウィンドウからはみ出す | `ui/inspector.rs` | 必要になったら対応 |
| 20 | インスペクタの当たり判定・ドラッグにはテストがない（`WindowId` をテストで作りにくいため）。書き戻し・丸め・書式は `Scene` / `Param` のテストで確認している | `ui/inspector.rs` | 未対応 |
| 21 | 範囲のないパラメータの、1 ピクセルあたりの変化量（`Int` 0.25 / `Float` 値の 1/200）は仮の値 | `Inspector::value_at` | 触って調整する |
| 22 | 計測の入力の時刻はフレーム単位（#11 と同じ。1 フレームで溜まった入力は同じ tick になる）。迷い・操作の時間も 1/60 秒単位 | `TestSession::step_tick` | 精密な時刻が要るなら入力に時刻を持たせる |
| 23 | 計測の「入力を受け付けない時間」「反応までの時間」は未計測（§5.5） | — | シーン側に「操作中」を宣言させる仕組みが要る |
| 24 | `TestSession::start`（設定の読み込み・ログファイルの作成）と `T` キー・集計の表示は、手動でしか確認していない。出題・判定・ログ・集計はテストあり | `ui/session.rs`, `appview.rs` | — |
| 25 | 課題の表示位置・サイズは固定（上部の帯、40px）。右上の進み具合の表示は、ウィンドウが狭いとはみ出す | `TestSession::build` | 必要になったら調整 |

## 10. 次の段階（PLAN.md §4.6）

PLAN.md §4.6 の 1〜3（2 つの方式・スライダーとホットリロード・操作ログと集計）は実装済み。この先は必要になったものから:
- 実際に計測して、方式の違いを分析する（ツールを使う段階）
- 記録した入力の再生、ログのタイムライン表示
- 「入力を受け付けない時間」「反応までの時間」の計測（シーン側に「操作中」を宣言させる）
- シーンの DSL から文字を出す機能（`(text ...)`）、シーン間で共通部分を共有する仕組み、インスペクタのスクロール
- 将来ほかの人に計測してもらうとき: 課題の説明、練習用の試行、参加者ごとのログ、集計の書き出し

## 11. Claude 向け作業メモ

- コード変更前に「共有リソースか（→`AppView`）／ウィンドウ固有か（→`WindowWrapper`）」を判断して置き場所を決める。
- ファイルを読み書きするときは `assets.rs` を通す。新しい外部ファイルはホットリロードに乗せるかを検討し、乗せるなら失敗しても落ちない形にする（シーンは `ScenePlayer` 方式、GPU 資源は `capture_validation` で包んで成功時だけ差し替え）。
- 新しい GPU 用構造体は `#[repr(C)]` + `bytemuck::Pod/Zeroable`。シェーダの `@location` / `@group` と必ず同期させる（3D は `Vertex::LAYOUT` / `ModelInstanceRaw::desc()`、UI は `RectInstance::LAYOUT`）。
- 3D の bind group を追加したら `ShaderWrapper::bind_group_layouts`（index = group 番号）に追加する。UI 側は `UiRenderer` が自前でレイアウトを持つ。
- DSL に新しいフォームや関数を足すときは、`ui/scene.rs` の `Scene::compile` / `Compiler` / `Func::lookup` を更新し、テストを足す。名前はコンパイル時にスロットへ解決し、毎フレーム名前で検索しない。スロットの並びを変えたら `Scene` のコメントと §6.3 も直す。
- シーンはキーを直接見ない。新しい入力（キー、パッド、タッチ）は `input.rs` で `Action` に変換して渡す。操作を増やすときは `Action` と `action_for_key` を更新する。
- 実行時の状態は `Runtime` に置き、`Scene` には置かない（リロードで消えないように）。状態を足したら `Scene::adopt` で引き継ぎ方を決める。
- `ui/*.ui` を変えたら `cargo test --lib` を回す（同梱のシーンを読み込むテストがある）。
- ウィンドウをまたぐ処理（あるウィンドウの操作で別のウィンドウのシーンを変える）は `AppView` に置き、相手は `WindowId` で指定する。
- Rust 側で UI を描くときは、矩形は `RectInstance`（色は `srgb8`）、文字は `Label` を `ui_layer.instances` / `text_layer.labels` に詰める（インスペクタと計測モードが例）。
- 計測のログの形式を変えたら、`ui/analysis.rs` の読み込みと §5.5 の例も直す。ログは集計の唯一の材料なので、記録できる情報は捨てずに書く。
- 計測の判定はシーンの中身を見ずに `emit` の通知だけで行う。新しい課題は `ui/tasks.ui` に、必要な通知はシーン側の遷移に足す。
- コメントは日本語と英語が混在（チュートリアル由来の英語コメントはそのまま残っている）。
