# learn-wgpu2 設計書

> このドキュメントはコードを読んで設計意図を逆算したもの。Claude（や人間）が作業前に読む前提で、
> 「何がどこにあるか」「なぜそうなっているか」「どこが未完成か」を優先して書いている。
> 行番号は執筆時点（commit `f6c524c` + 未コミット変更）のもの。

---

## 1. プロジェクト概要

- **目的**: [Learn Wgpu](https://sotrh.github.io/learn-wgpu/) チュートリアルを写経しつつ、独自の拡張（マルチウィンドウ、ホットリロード、外部ファイルからのモデル読込、独自DSL）を加えていく学習用プロジェクト。
- **進捗**: git履歴はチュートリアルの章名に対応している。
  `Dependencies and the window` → `The Surface` → `The Pipeline` → `Uniform buffers and a 3d camera` → `Instancing` → `depth buffer`（最新）。
  次章は *Model Loading*（OBJ読込）に相当するが、本プロジェクトでは独自路線として S式モデル/DSL に進みつつある。
- **作業中（未コミット）**:
  - `src/parser.rs` … 描画用ミニ言語（S式 + 中置式）のレキサー/パーサー。独立バイナリ `parser` として実行可能。
  - `src/text.rs` … glyphon によるテキスト描画ラッパー。`lib.rs` では `// mod text;` とコメントアウト中で未統合。
  - `Cargo.toml` … `glyphon = "0.10.0"` と `[[bin]] parser` を追加。

## 2. 技術スタック

| 用途 | クレート | 備考 |
|---|---|---|
| GPU | `wgpu 28` | wasm時は `webgl` feature、`Backends::GL` |
| ウィンドウ/イベント | `winit 0.30` | `ApplicationHandler` 方式 |
| 線形代数 | `cgmath 0.18` | |
| GPUへのバイト変換 | `bytemuck` | `Pod`/`Zeroable` derive |
| 画像 | `image 0.24`（png/jpeg のみ） | |
| モデル保存形式 | `serde` + `serde-lexpr`/`lexpr`（バージョン固定 `=`） | S式でシリアライズ |
| テキスト | `glyphon 0.10` | wgpu 28 を共有（Cargo.lock 上 wgpu は1バージョンのみ） |
| エラー | `anyhow` | |
| ログ | `env_logger`（native, 既定 `debug`）/ `console_log`（wasm） | |

Rust edition 2024。`crate-type = ["cdylib", "rlib"]`（wasm出力とネイティブbinの両対応）。

## 3. ディレクトリ / ファイル構成

```
Cargo.toml
src/
  main.rs            ネイティブのエントリ。lib の run() を呼ぶだけ
  lib.rs             run() / run_web()、モジュール宣言、hsv_to_rgb()
  app.rs             App: winit の ApplicationHandler 実装（イベント振り分け）
  appview.rs         AppView: GPU共有リソース（device/queue/pipeline/model/instances）
  window.rs          WindowWrapper: ウィンドウ毎のリソース（surface/camera/depth）+ CameraController
  camera.rs          Camera, CameraUniform, OPENGL_TO_WGPU_MATRIX
  model_instance.rs  ModelInstance / ModelInstanceRaw（インスタンシング用）
  vertex.rs          Vertex, Model, ModelBuffer、モデルの保存/読込
  texture.rs         TextureWrapper（画像テクスチャ, 深度テクスチャ）
  shader.rs          ShaderWrapper（WGSL読込・BindGroupLayout定義・リロード）
  shader.wgsl        頂点/フラグメントシェーダ（実行時にファイルから読む）
  text.rs            [WIP・未接続] glyphon テキスト描画
  parser.rs          [WIP] 描画DSLのパーサー（独立bin）
pentagon.json        モデルデータ（※中身は JSON ではなく S式）
happy-tree.png       ディフューズテクスチャ
run.html, pkg/       wasm 実行用（wasm-pack 出力）
```

## 4. アーキテクチャ

### 4.1 レイヤ構造（所有関係）

```
App  (ApplicationHandler<AppView>)
 └─ state: Option<AppView>            ← 全ウィンドウで共有する GPU リソース
      ├─ device, queue
      ├─ shader: ShaderWrapper         (ShaderModule + BindGroupLayout[texture, camera])
      ├─ render_pipeline               (1本のみ)
      ├─ model_buffer: ModelBuffer     (pentagon.json から)
      ├─ diffuse_bind_group            (happy-tree.png)
      ├─ model_instances + model_instance_buffer (10x10 = 100個)
      └─ windows: Vec<WindowWrapper>   ← ウィンドウ毎のリソース
           ├─ nature: Arc<winit::Window>
           ├─ surface + config + is_surface_configured
           ├─ camera / camera_uniform / camera_buffer / camera_bind_group
           ├─ camera_controller
           ├─ depth_texture
           └─ last_cursor_position
```

**設計意図**: チュートリアルの `State` 構造体を2つに分割している。
- **共有すべきもの**（device・パイプライン・メッシュ・テクスチャ・インスタンス）は `AppView` に1つ。
- **ウィンドウ固有のもの**（surface・深度バッファ・カメラ・入力状態）は `WindowWrapper` に。
  → 同じシーンを複数ウィンドウから別々のカメラで見る構成。ネイティブでは起動時に2ウィンドウ作る（`app.rs:52-61`）。

命名規約: wgpu/winit の生オブジェクトをラップした構造体は `XxxWrapper`、ラップ対象のフィールド名は **`nature`**（`WindowWrapper.nature: Arc<Window>`, `ShaderWrapper.nature: ShaderModule`）。

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

wasm では async を block できないので、生成した `AppView` 自体を **ユーザーイベントとして** イベントループに送り返す（チュートリアル準拠のパターン）。

`AppView::new` の流れ（`appview.rs:32-166`）:
1. `Instance` 作成 → windows[0] で一時 surface を作り、それと互換な adapter を取得
2. sRGB の surface format を選択（なければ先頭）
3. device/queue 取得（wasm は `downlevel_webgl2_defaults`）
4. `ShaderWrapper::new` → `constuct_render_pipeline`
5. `pentagon.json` → `ModelBuffer`
6. `happy-tree.png` → `diffuse_bind_group`
7. 各 window から `WindowWrapper::new`（surface/カメラ/深度を作成。surface はこの時点では未configure）
8. 10×10 グリッドのインスタンス生成 → インスタンスバッファ

### 4.3 イベント処理（`app.rs:100-166`）

`window_id` から `windows` 内のインデックスを引き、そのウィンドウにだけ処理を流す。

| イベント | 処理 |
|---|---|
| `CloseRequested` | そのウィンドウを `windows` から除去。空になったらアプリ終了 |
| `Resized` | surface 再configure + 深度テクスチャ再生成。**ここで初めて `is_surface_configured = true`** |
| `KeyboardInput` | `Esc`=終了、`R`=ホットリロード（全体）。その後そのウィンドウの CameraController に渡す |
| `RedrawRequested` | `update()`（カメラ→uniform書込）→ `render()`。Lost/Outdated なら resize で復旧 |
| `CursorMoved` | カーソル位置を保持（背景色に使用） |

`render()` の先頭で `request_redraw()` を呼ぶため、各ウィンドウは**常時連続描画**ループになる。

### 4.4 描画パイプライン

- 1パス・1パイプライン。`TriangleList`, `Ccw` が表, 背面カリング有効, `BlendState::REPLACE`。
- 深度: `Depth32Float`, `CompareFunction::Less`, 毎フレーム 1.0 でクリア。
- クリアカラー: カーソル未取得時は `(0.1, 0.2, 0.3)`。取得後は **カーソル位置を HSV に変換**（x→色相, y→明度, 彩度0.5）。チュートリアルの「マウスで背景色を変える」課題の名残。
- ドローコール: `draw_indexed(0..num_indices, 0, 0..インスタンス数)` を1回。

#### Bind Group

| group | binding | 内容 | 所有者 | 可視性 |
|---|---|---|---|---|
| 0 | 0 | `t_diffuse: texture_2d<f32>` | AppView（共有） | FRAGMENT |
| 0 | 1 | `s_diffuse: sampler` | AppView（共有） | FRAGMENT |
| 1 | 0 | `camera: CameraUniform { view_proj: mat4x4 }` | WindowWrapper（ウィンドウ毎） | VERTEX |

#### 頂点バッファ

| slot | step | 構造体 | location: 型 |
|---|---|---|---|
| 0 | Vertex | `Vertex` (32B) | 0: position `vec3`, 1: color `vec3`, 2: texture_coords `vec2` |
| 1 | Instance | `ModelInstanceRaw` (72B) | 5: model_id `u32`, 6: texture_id `u32`, 7–10: model行列 `vec4×4` |

- location 3,4 は将来の頂点属性（法線等）用に空けてある（チュートリアルの方針）。
- `model_id` / `texture_id` はシェーダに渡しているが **現状未使用**。将来のテクスチャ配列・インスタンス識別のための布石と思われる。

#### シェーダ（`shader.wgsl`）

```
clip = camera.view_proj * model_matrix * vec4(position, 1)
color = textureSample(t_diffuse, s_diffuse, uv) * vec4(vertex_color, 1)
```

### 4.5 カメラ

- `Camera`: eye/target/up + 透視投影（fovy 45°, near 0.1, far 100）。初期 eye=(0,1,2), target=原点。
- `OPENGL_TO_WGPU_MATRIX` で cgmath(OpenGL, z∈[-1,1]) → wgpu(z∈[0,1]) に変換。
- `CameraController`（`window.rs:208-281`、ウィンドウ毎）: WASD/矢印キー。
  W/S で target へ前後移動（近づきすぎ防止あり）、A/D で target 周りを**距離を保って周回**。speed=0.2/フレーム。

### 4.6 インスタンシング

`AppView::new` 内で 10×10 を xz 平面に配置（中心が原点付近になるよう `(5,0,5)` オフセット）。
各インスタンスは「原点からの方向ベクトルを軸に45°回転」。原点ちょうどのものはゼロ除算回避のため回転なし。
`ModelInstance`（CPU側, cgmath型）→ `to_raw()` → `ModelInstanceRaw`（GPU用 POD, 行列は `translation * rotation`）。
インスタンスは生成後に更新されない（バッファ usage に `COPY_DST` 無し）。

## 5. 外部データとホットリロード

### 5.1 実行時に読むファイル（すべて **カレントディレクトリ相対**）

| パス | 読む場所 | 用途 |
|---|---|---|
| `src/shader.wgsl` | `shader.rs:13` | シェーダ（`include_str!` ではなく実行時読込） |
| `pentagon.json` | `appview.rs:237` | モデル（頂点/インデックス） |
| `happy-tree.png` | `appview.rs:101` | テクスチャ |

→ **リポジトリのルートで `cargo run` する前提**。

### 5.2 ホットリロード（`R` キー, `AppView::reload` `appview.rs:258`）

1. `shader.wgsl` を再読込して ShaderModule を差し替え
2. レンダーパイプラインを再構築（surface format は windows[0] のものを使用）
3. `pentagon.json` を再読込して ModelBuffer を差し替え

各ステップは独立に失敗を `log::error!` するだけでアプリは継続する設計（`report_err`）。
**意図**: シェーダ/モデルをファイル編集→Rで即確認、というイテレーション高速化。シェーダを `include_str!` にしていないのはこのため。
テクスチャとインスタンスはリロード対象外。

### 5.3 モデルファイル形式

`Model { vertices: Vec<Vertex>, indices: Vec<u16> }` を serde でシリアライズ。

- **S式（現行・採用中）**: `save_sexpr_file` / `load_sexpr_file`。`pentagon.json` は拡張子が json だが中身は S式:
  ```lisp
  ((vertices ((position . #(x y z)) (color . #(r g b)) (texture_coords . #(u v))) ...)
   (indices 0 1 4 1 2 4 2 3 4))
  ```
  手書き編集しやすいテキスト形式として選ばれたと思われる。
- **バイナリ（未使用）**: `save_to_binaryfile` / `load_from_binary_file`。`[u64 頂点数][u64 インデックス数][Vertex...][u16...]`（LE, bytemuck直書き）。
- `vertex::pentagon()` はハードコードの元データ。`load_sexpr_file` 内の `reset` フラグを true にするとこれでファイルを再生成する（開発用スイッチ）。

## 6. WIP: 描画DSL パーサー（`src/parser.rs`）

### 6.1 意図

`main()` のサンプルから読み取れる狙い: **パラメータ付きで2D要素を配置する宣言的なシーン記述言語**。
いずれ `pentagon.json` のような静的データを置き換え/拡張し、ホットリロードで編集→即描画するためのものと推測される。

```lisp
(defparam s 100 (Int 0 100 _))          ;; 名前, 初期値, 型と範囲(min max step?)。_ はワイルドカード
(defparam t 0 (Float 0 _ 0.16))
(defparam i0 "test-ui.png" (Image))     ;; 画像リソースもパラメータ
(background (rgb 100 200 0))
(paint rect {s+t*100} {s + t * 100} 200 300)   ;; {} 内は中置記法の算術式
(paint i0 {s + t * 50 + 100} ... 200 300)
(defvar m1 (matrix 3 3 ((1 2 0) (4 5 0) (7 8 1))) (Matrix33))
(paint rect ... m1)                     ;; 変換行列を渡す
(log s)
```

**構文の設計**: 構造は S式（Lisp風、`;;` 行コメント）、数式だけ `{ ... }` で囲んだ中置記法。Lisp の前置記法で式を書く煩わしさを避けるハイブリッド。

### 6.2 構成

```
&str ──Lexer──▶ Vec<Token{kind, span}> ──Parser──▶ Vec<AstNode>
```

- **Lexer**: `( ) { } + - * / !`, 整数, 浮動小数(`1.5`), 文字列(`\n \t \" \\` エスケープ), シンボル（先頭 `[A-Za-z_?]`, 以降 `[A-Za-z0-9_\-?!]`）。
- **AstNode**: `Form{head: String, args}` / `Expr(Expr)` / リテラル / `Symbol`。Form の head は必ずシンボル。
- **Expr**: Pratt パーサー（binding power: `+ -` = (1,2), `* /` = (3,4), 単項 `- !` = 100）。`f(a, b)` 形式の関数呼び出し、`( )` によるグルーピング対応。左結合。
- エラー: `ParseError { message, span }`（Parser）と `String`（Lexer）が混在。

### 6.3 未完成箇所（コード上の `todo`/実装から判明するもの）

- `Comma` トークンが **Lexer で生成されない** → `{f(a, b)}` の複数引数呼び出しは現状パースできない（`todo` コメントあり）。
- `()`（空フォーム）や `((1 2 0) ...)`（head がシンボルでないリスト）は `expected symbol` エラーになる。→ サンプル `main()` は**現状パース失敗する**（行列リテラルの構文が未対応）。
- フォーム中の負数リテラル `-1` は不可（`-` は演算子トークン扱い）。`.5` 形式も不可。累乗未対応（コメントあり）。
- `Span` は char インデックスだが、エラーメッセージは "byte" と表示。
- 評価器（AST → 描画命令）は未実装。`lib.rs` で `mod parser;` 宣言されているが、ライブラリ側からは未使用（`[[bin]] parser` としてのみ使用）。

## 7. WIP: テキスト描画（`src/text.rs`）

`GlyphonText`: glyphon の FontSystem/SwashCache/TextAtlas/TextRenderer/Buffer を1つにまとめたラッパー。
- 32px / 行間42px、左上 (24,24) マージン、白文字、SansSerif、日本語サンプル文。
- API: `new(device, queue, format, w, h)` / `set_text` / `resize` / `prepare(device, queue, w, h)` / `render(&mut RenderPass)`。
- **未接続**。統合するなら: WindowWrapper ごとに持ち、`resize` で `resize`、`render()` 内で描画前に `prepare`、同じ render pass の最後に `render` を呼ぶ形が自然。
  ただし現パイプラインは深度アタッチメント付きなので、glyphon の `TextRenderer::new` の depth_stencil 引数（現在 `None`）との整合が必要。

## 8. ビルド / 実行

```bash
cargo run                  # ネイティブ。リポジトリ直下で実行すること（相対パス読込のため）
cargo run --bin parser     # DSL パーサー単体テスト（AST を Debug 出力）
```

操作: `WASD`/矢印=カメラ, マウス移動=背景色, `R`=ホットリロード, `Esc`=終了, ウィンドウ閉じる=そのウィンドウだけ破棄。

wasm: `pkg/` に wasm-pack 出力あり、`run.html` から読み込む想定。

## 9. 既知の問題・設計上の注意点

Claude が修正タスクを受けたときに踏みやすいポイント。**意図的なのか未着手なのか**も併記。

| # | 内容 | 場所 | 状態 |
|---|---|---|---|
| 1 | camera の BindGroupLayout が `ShaderWrapper` と `WindowWrapper` で**二重定義**。パイプラインは前者、bind group は後者のレイアウトで作られている（内容が同一なので動く）。コード内 todo で「カメラ側かシェーダ側か」と迷っている | `shader.rs:41`, `window.rs:73-88` | 未解決の設計課題 |
| 2 | windows[0] に surface を2回作成（adapter選択用に1つ、WindowWrapper で1つ）。前者は捨てられる | `appview.rs:48`, `window.rs:33` | 冗長だが動作する |
| 3 | リサイズ時にカメラの `aspect` を更新していない（初期サイズ固定） | `window.rs:117` | 未対応 |
| 4 | surface の configure は `Resized` イベント待ち。来なければ描画されない | `window.rs:107,147` | チュートリアル準拠 |
| 5 | パイプラインは windows[0] の surface format で作られ全ウィンドウ共用（全ウィンドウ同じ format 前提） | `appview.rs:169` | 前提として許容 |
| 6 | WGSL のコンパイルエラーは wgpu の uncaptured error 扱いで、`Result` では返らない → 壊れたシェーダで `R` を押すとパニックし得る（`push_error_scope` 未使用） | `shader.rs:64` | ホットリロードの穴 |
| 7 | wasm: `std::fs` でファイルを読むため wasm32 では読込失敗する。`run.html` は `./pkg/tutorial1_window.js` を import しているが実際の出力は `learn_wgpu2.js`。wasm 側でも window2 を作成している（使われない） | `shader.rs`, `vertex.rs`, `texture.rs`, `run.html:24`, `app.rs:54` | wasm は現状壊れている |
| 8 | `WindowWrapper::update` が毎フレーム `log::debug!` を出す（既定ログレベルが debug） | `window.rs:134`, `lib.rs:27` | ノイズ |
| 9 | `TextureWrapper::new` は失敗時に `expect`/`unwrap` でパニック（他は anyhow） | `texture.rs:14-15` | エラー処理が不統一 |
| 10 | `constuct_render_pipeline` は綴り誤り（construct）。呼び出し側も同名なので変更時は両方 | `appview.rs:180` | 命名 |
| 11 | 未使用 import が多い（`wgpu::wgc::device` など） | 各所 | 警告のみ |

## 10. 今後の拡張の方向性（コードから推測）

1. **DSL の完成** → 評価器を作り、`defparam` を UI（スライダ等）にバインド、`paint` を描画命令に変換。`R` リロードの対象にする。
2. **テキスト描画の統合**（`text.rs`）→ パラメータ値やログの画面表示に使う想定か。
3. **インスタンス毎テクスチャ**（`texture_index` / `texture_id`）→ テクスチャ配列化。
4. 次のチュートリアル章（Model Loading / Lighting）への合流。location 3,4 が空いているのはそのため。

## 11. Claude 向け作業メモ

- コード変更前に「共有リソースか（→`AppView`）／ウィンドウ固有か（→`WindowWrapper`）」を判断して置き場所を決めること。
- 新しい GPU 用構造体は `#[repr(C)]` + `bytemuck::Pod/Zeroable`、レイアウトは `Vertex::LAYOUT` / `ModelInstanceRaw::desc()` に倣う。シェーダの `@location` と必ず同期させる。
- 新しい bind group を追加したら `ShaderWrapper::bind_group_layouts` に追加（index = group 番号）し、WGSL の `@group` と合わせる。
- 外部ファイルを増やす場合はホットリロード（`AppView::reload`）に乗せるかを検討する。乗せるなら失敗してもアプリが落ちない形（`report_err`）で。
- コメントは日本語と英語が混在（チュートリアル由来の英語コメントはそのまま残っている）。
