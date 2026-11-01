# engine 設計書

> ウィンドウ・GPU・入力・矩形と文字の描画。製品（メニュー試作、ゲーム、作曲ソフト）は知らず、`Product` trait 越しに呼ぶ。
> 場所は「ファイル + 関数名」で示す（行番号はすぐずれる）。

## 1. ファイル構成

```
src/
  lib.rs        run()（ロガー初期化 → app::run）、公開する型の再輸出（wgpu・winit も）、hsv_to_rgb()
  product.rs    Product trait と、製品に渡す Gpu / Frame / Pointer / WindowSpec
  app.rs        App<P>: winit の ApplicationHandler 実装（イベント振り分け）。OS 依存の層
  appview.rs    AppView: 全ウィンドウ共有の GPU リソースとウィンドウの一覧
  window.rs     WindowWrapper: ウィンドウ毎のリソース（surface・深度・入力・UI と文字の層）
  assets.rs     ファイル読み書きの窓口（読み込み・書き戻しはすべてここを通す）
  input.rs      入力の2層化: キー → 抽象的な操作（Action）の押下・解放（InputEvent）
  renderer.rs   UiRenderer（共有パイプライン）/ UiLayer（ウィンドウ毎のバッファ）/ RectInstance / srgb8()
  text.rs       TextSystem（共有フォント）/ TextLayer（ウィンドウ毎の文字描画）/ Label
  texture.rs    TextureWrapper（画像テクスチャ, 深度テクスチャ）
  shader.rs     capture_validation()（wgpu の検証エラーを Err にする）
  ui.wgsl       UI 用 2D 矩形シェーダ（実行時に CARGO_MANIFEST_DIR/src/ui.wgsl から読む）
examples/demo3d/  learn-wgpu チュートリアル由来の 3D の見本（§6）
```

## 2. 技術スタック

| 用途 | クレート | 備考 |
|---|---|---|
| GPU | `wgpu 28` | wasm時は `webgl` feature、`Backends::GL` |
| ウィンドウ/イベント | `winit 0.30` | `ApplicationHandler` 方式 |
| GPUへのバイト変換 | `bytemuck` | `Pod`/`Zeroable` derive |
| 画像 | `image 0.24`（png/jpeg のみ） | |
| 文字描画 | `glyphon 0.10`（内部で cosmic-text 0.15） | wgpu 28 を共有（Cargo.lock 上 wgpu は1バージョンのみ） |
| エラー / ログ | `anyhow` / `env_logger`（native, 既定 `info`。`RUST_LOG` で変える）・`console_log`（wasm） | |
| 非同期の待ち | `pollster` | 起動時の adapter/device 取得と、wgpu のエラースコープの回収 |
| 見本だけ（dev-dependencies） | `cgmath 0.18`、`serde` + `serde-lexpr`/`lexpr`（`=` で固定） | `examples/demo3d` |

製品側は `engine::wgpu` / `engine::winit` を使えば同じ版になる。

## 3. 製品との境界（`product.rs`）

```rust
engine::run(windows: Vec<WindowSpec>, make_product: impl FnOnce(&Gpu, &[WindowId]) -> anyhow::Result<P>)
```
- `windows` の順にウィンドウを開き（wasm では先頭の 1 つだけ、canvas に割り当て）、GPU ができたら `make_product` を 1 回呼ぶ。`&[WindowId]` は `windows` と同じ順。
- `Product` のメソッド（`update` 以外は既定で何もしない）:

| メソッド | 呼ばれるとき |
|---|---|
| `key(window, code, pressed, repeat)` | 物理キー。Esc はエンジンが終了に使う（製品にも届く）。フォーカスを失ったときは、押されているキーを `pressed = false` で離す |
| `inputs(window, &[InputEvent])` | キーを `Action` にしたもの。フォーカスを失ったときの一斉解放もここ |
| `pointer(Pointer)` | カーソル移動と左ボタンの押下・解放（論理ピクセル、ウィンドウの論理サイズ付き） |
| `reload(gpu)` | `R` キー。エンジンが UI シェーダを読み直したあと |
| `update(gpu, &mut Frame)` | 毎フレーム描画の前。`frame.rects` / `frame.labels`（毎フレーム空から）を詰め、`frame.background` を決める。`frame.dt` はこのウィンドウの前のフレームからの経過時間（最初は 0）。製品は `Instant` を使わずこれで時間を進める |
| `render(window, &mut RenderPass)` | 製品自身の描画（3D など）。このあとエンジンが矩形と文字を上に重ねる |
| `window_closed(window) -> bool` | ウィンドウが閉じられた。true で終了（ウィンドウが 0 になっても終了） |

- 製品固有の描画は製品側に置き、`render` でエンジンのレンダーパスに足す。レンダーパスは深度（`TextureWrapper::DEPTH_FORMAT`、毎フレーム 1.0 でクリア）付き。

## 4. アーキテクチャ

### 4.1 レイヤ構造（所有関係）

```
App<P>  (ApplicationHandler<AppView>)          ← OS 依存の層
 ├─ windows: Vec<WindowSpec>, make_product      (起動時に 1 回使う)
 └─ state: Option<(AppView, P)>
      ├─ AppView                                ← 全ウィンドウで共有する GPU リソース
      │   ├─ gpu: Gpu { device, queue, surface_format }   (製品にも渡す)
      │   ├─ ui_renderer: UiRenderer            (UI: パイプライン + BindGroupLayout)
      │   ├─ text_system: TextSystem            (フォント・グリフのキャッシュ。読み込みが重いので共有)
      │   └─ windows: Vec<WindowWrapper>        ← ウィンドウ毎のリソース
      │        ├─ nature: Arc<winit::Window>
      │        ├─ surface + config + is_surface_configured + depth_texture
      │        ├─ last_cursor_position
      │        ├─ input: InputMapper            (押されているキー → 操作の押下・解放)
      │        ├─ ui_layer: UiLayer             (画面サイズ uniform + 矩形インスタンスバッファ)
      │        ├─ text_layer: TextLayer         (文字のアトラス・描画パイプライン + 今フレームの Label 列)
      │        └─ background                    (製品が決めた背景色。None ならカーソル位置の HSV)
      └─ P: Product                             ← 製品の中身はすべてここ（ウィンドウは WindowId で区別）
```

- **共有すべきもの**（device・パイプライン・フォント）は `AppView` に、**ウィンドウ固有のもの**（surface・深度バッファ・入力状態・文字のアトラス）は `WindowWrapper` に置く。
- ウィンドウの指定は `windows` のインデックスではなく `WindowId` で行う（ウィンドウを閉じると並びがずれるため）。
- 命名規約: wgpu/winit の生オブジェクトをラップした構造体は `XxxWrapper`、ラップ対象のフィールド名は **`nature`**。

### 4.2 起動シーケンス

```
engine::run(windows, make_product)
  ├─ ロガー初期化
  ├─ EventLoop::with_user_event()     (ユーザーイベント型 = AppView)
  └─ run_app(App)
       resumed()
         ├─ native: windows を全部作成 → pollster::block_on(AppView::new(...)) → make_product
         └─ wasm  : <canvas id="canvas"> に windows[0] を割当、spawn_local で AppView::new → proxy.send_event(AppView)
                     → user_event() で初回 resize → make_product
```
wasm では async を block できないので、生成した `AppView` 自体を **ユーザーイベントとして** イベントループに送り返す（チュートリアル準拠）。

`AppView::new`: `Instance` → windows[0] で一時 surface を作り、互換な adapter を取得 → sRGB の surface format を選択（なければ先頭）→ device/queue（wasm は `downlevel_webgl2_defaults`）→ `UiRenderer::new`、`TextSystem::new` → 各ウィンドウの `WindowWrapper::new`（surface はこの時点では未configure）。

### 4.3 イベント処理（`App::window_event`）

| イベント | 処理 |
|---|---|
| `CloseRequested` | `windows` から除去 → `Product::window_closed`。true かウィンドウが 0 なら終了 |
| `Resized` | surface 再configure + 深度テクスチャ再生成。**ここで初めて `is_surface_configured = true`** |
| `KeyboardInput` | `Esc`=終了、`R`=`AppView::reload`（UI シェーダ）+ `Product::reload`。その後 `WindowWrapper::handle_key` → `Product::key` → `InputMapper` → `Product::inputs` |
| `Focused(false)` | `focus_lost`: 押されたままのキーをすべて離したことにして `Product::inputs` に渡す |
| `RedrawRequested` | `WindowWrapper::update`（`Product::update`）→ `render`。Lost/Outdated なら resize で復旧 |
| `CursorMoved` / `MouseInput`（左） | カーソル位置を保持し、`Product::pointer`（Moved / Down / Up） |

キー入力はフォーカスのあるウィンドウにだけ届く。`render()` の先頭で `request_redraw()` を呼ぶため、各ウィンドウは**常時連続描画**ループになる。

### 4.4 1フレームの流れ（`WindowWrapper::update` → `render`）

```
update(gpu, text_system, product)
  ├─ ui_layer.instances / text_layer.labels を空にする
  ├─ product.update(gpu, Frame { window, logical_size, rects, labels, background })
  ├─ ui_layer.upload(...)     … 画面サイズ（論理ピクセル）とインスタンスを GPU に送る
  └─ text_layer.prepare(...)  … Label をシェイピング（文字列が変わったものだけ）してアトラスに載せる
render(gpu, ui_renderer, product)
  ├─ クリア（background があればその色、なければカーソル位置の HSV）+ 深度 1.0
  ├─ product.render(window, render_pass)   … 製品自身の描画
  ├─ UI: ui_layer.draw()      … 同じレンダーパスで上に重ねる
  └─ 文字: text_layer.draw()  … さらにその上
```

### 4.5 UI 描画パイプライン（`renderer.rs`, `ui.wgsl`）

- **頂点バッファは使わない**。`vertex_index`（0..6）から四角形の角を作り、矩形ごとのデータはインスタンスとして渡す。`draw(0..6, 0..矩形数)` を1回。
- 座標は**論理ピクセル**（左上原点・y 下向き）。シェーダの uniform に論理サイズ（物理サイズ ÷ scale_factor）を渡して NDC に変換する。
- アルファブレンド有効、カリングなし。製品の描画と同じレンダーパスで描くため、深度アタッチメントの形式は合わせるが `depth_compare: Always` / 書き込みなしで常に手前に出る。
- 色は 0–255 の sRGB で書き、`RectInstance` に入れる時点で線形に変換する（サーフェスが sRGB のため）。Rust 側では `renderer::srgb8(r, g, b, a)`。

| group | binding | 内容 | 所有者 |
|---|---|---|---|
| 0 | 0 | `screen: Screen { size: vec2, _padding: vec2 }` | UiLayer（ウィンドウ毎） |

| slot | step | 構造体 | location: 型 |
|---|---|---|---|
| 0 | Instance | `RectInstance` (36B) | 0: position（左上）`vec2`, 1: size `vec2`, 2: color（線形 RGBA）`vec4`, 3: rotation（ラジアン、中心まわり）`f32` |

インスタンスバッファは足りなくなったら2のべき乗の容量で作り直す（`UiLayer::upload`）。

### 4.6 文字描画（`text.rs`）

- glyphon を使う。**フォント（`FontSystem`）・`SwashCache`・`Cache` は全ウィンドウで共有**（`TextSystem` → AppView）。`FontSystem::new()` はシステムフォントを読むので重い。
- **アトラス・`TextRenderer`・`Viewport` はウィンドウ毎**（`TextLayer` → WindowWrapper）。
- 使い方: 毎フレーム `Frame::labels` に `Label { text, position, size, color }` を詰める → `update` 内の `prepare` → `render` 内の `draw`。
  - 位置とサイズは**論理ピクセル**。glyphon には物理ピクセルで渡す（`TextArea` の `left/top` に scale_factor を掛け、`scale` に scale_factor を入れる）。色は sRGB 0–255。
  - 前フレームと同じ文字列・サイズの Label は、シェイピングし直さない（`TextLayer::shaped` にキャッシュ）。Label の並び順が変わるとキャッシュが効かない。
- UI と同じく `depth_compare: Always` / 書き込みなし。

## 5. ファイルとホットリロード

- 読み書きはすべて `assets.rs` を通す（将来は同梱素材・ネットワークに差し替え）。パスはカレントディレクトリ相対。製品は起動時に自分のクレートのディレクトリへ `set_current_dir` している。
- エンジン自身が読むのは `ui.wgsl` だけで、製品のリポジトリから動かしても見つかるように `CARGO_MANIFEST_DIR` から引く（`renderer.rs` の `SHADER_PATH`）。`R` で読み直す。
- 素材の変更検知: `assets::Watch::new(&[パス…])` を作り、毎フレーム `changed()` を呼ぶと、更新日時が変わったファイルが返る（見に行くのは 200ms ごと）。
- 絵（`texture.rs`）: `load_image` は PNG・JPEG・GIF の最初のフレーム、`load_gif` は GIF の全フレーム（重ね合わせた後の完成した絵と、1/100 秒単位の表示時間）。どちらも失敗したら `Err`。`TextureWrapper::new(path, filter, ..)` / `from_image` は 1 枚、`from_layers` は同じ大きさの絵を層にしたテクスチャ配列（ビューは `D2Array`）。`filter` はドット絵なら `Nearest`。
- **失敗しても落ちない**: `shader::capture_validation()` が wgpu のエラースコープで検証エラーを捕まえ、パニックではなく `Err` にする。失敗したら差し替えない（`UiRenderer::build_pipeline` 全体を包んでいる）。

## 6. 見本: `examples/demo3d`（learn-wgpu チュートリアル由来）

`cargo run -p engine --example demo3d`。テクスチャ付き五角形 ×100 を `Product::render` で描く。製品が自分の描画パスを足す方法の見本で、ゲームで 3D を始めるときの下敷き。WASD=カメラ、R=`shader.wgsl` と `pentagon.json` の読み直し。

- ファイル: `main.rs`（`Product` の実装）、`demo3d.rs`（パイプライン・モデル・ウィンドウ毎のカメラ）、`camera.rs`、`model_instance.rs`、`vertex.rs`、`shader.rs`（3D 用 `ShaderWrapper`）、`shader.wgsl`、`pentagon.json`、`happy-tree.png`。起動時にこのディレクトリへ `set_current_dir` する。
- `TriangleList`, `Ccw` が表, 背面カリング有効, `BlendState::REPLACE`。深度 `Depth32Float`, `Less`。

| group | binding | 内容 | 所有者 | 可視性 |
|---|---|---|---|---|
| 0 | 0 | `t_diffuse: texture_2d<f32>` | Demo3d | FRAGMENT |
| 0 | 1 | `s_diffuse: sampler` | Demo3d | FRAGMENT |
| 1 | 0 | `camera: CameraUniform { view_proj: mat4x4 }` | View（ウィンドウ毎） | VERTEX |

| slot | step | 構造体 | location: 型 |
|---|---|---|---|
| 0 | Vertex | `Vertex` (32B) | 0: position `vec3`, 1: color `vec3`, 2: texture_coords `vec2` |
| 1 | Instance | `ModelInstanceRaw` (72B) | 5: model_id `u32`, 6: texture_id `u32`, 7–10: model行列 `vec4×4` |

- location 3,4 は将来の頂点属性用に空けてある。`model_id` / `texture_id` は**未使用**。
- bind group を足したら `ShaderWrapper::bind_group_layouts`（index = group 番号）に足す。
- モデルファイル: `Model { vertices, indices: Vec<u16> }` を serde で S式に（`save_sexpr_file` / `load_sexpr_file`。`pentagon.json` は拡張子が json だが中身は S式）。バイナリ版（`save_to_binaryfile` / `load_from_binary_file`）は未使用。`vertex::pentagon()` はハードコードの元データ。

## 7. 既知の問題・設計上の注意点

| # | 内容 | 場所 | 状態 |
|---|---|---|---|
| 1 | windows[0] に surface を2回作成（adapter 選択用に1つ、WindowWrapper で1つ）。前者は捨てられる | `AppView::new`, `WindowWrapper::new` | 冗長だが動作する |
| 2 | surface の configure は `Resized` イベント待ち。来なければ描画されない | `WindowWrapper::resize`, `render` | チュートリアル準拠 |
| 3 | パイプラインは windows[0] の surface format で作られ全ウィンドウ共用（`Gpu::surface_format`） | `AppView::new` | 前提として許容 |
| 4 | **wasm は動かない**: `std::fs` でファイルを読む（`assets.rs`）、`std::time::Instant` が wasm32 ではパニックする（uikit、`window.rs` の `Frame::dt`、`assets::Watch`）、`capture_validation` の `pollster::block_on` は WebGPU バックエンドでは待てない可能性がある、`run.html` の import 先が実際の出力と違う、wasm のエントリ（`#[wasm_bindgen(start)]`）は分割のときに消えたので製品側に作り直す必要がある。さらに WebGL では canvas をまたいで device を共有できない（PLAN.md §6） | 各所 | 移植時にまとめて対応 |
| 5 | `R` キーで壊れた WGSL を読んだときに落ちないことは、コード上は対処済みだが手動では未確認 | `shader::capture_validation` | 要確認 |
| 7 | 押せるのは左ボタンだけ（`PointerKind`）。タッチは未対応 | `app.rs` | 必要になったら足す |
| 8 | `Action`（`menu` `up` … `cancel`）とキー配置はメニュー試作の都合で決めたもの | `input.rs` | ゲームで要るものが分かったら見直す |

## 8. 作業メモ

- 共有リソースか（→ `AppView`）／ウィンドウ固有か（→ `WindowWrapper`）／製品のものか（→ 製品側。`Product` 越し）を判断して置き場所を決める。エンジンに製品の名前を持ち込まない。
- 新しい外部ファイルは `assets.rs` を通す。ホットリロードに乗せるなら、GPU 資源は `capture_validation` で包んで成功時だけ差し替える。
- GPU 用構造体は `#[repr(C)]` + `bytemuck::Pod/Zeroable`。シェーダの `@location` / `@group` と同期させる（UI は `RectInstance::LAYOUT`）。
- 新しい入力（キー、パッド、タッチ）は `input.rs` で `Action` に変換する。操作を増やすときは `Action` と `action_for_key` を更新する。
- コメントは日本語と英語が混在（チュートリアル由来の英語コメントはそのまま）。
