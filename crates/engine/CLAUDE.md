# engine

ウィンドウ・GPU・入力・矩形と文字の描画。設計は [DESIGN.md](DESIGN.md)。
**製品（メニュー試作、ゲーム、作曲ソフト）を知らない**。製品の中身は `Product` trait（`src/product.rs`）越しにだけ呼ぶ。製品固有の描画（3D、ボクセルなど）は製品側に置き、`Product::render` で足す。エンジンを太らせない。

## 規約
- 共有GPUリソースは `AppView`、ウィンドウ固有のものは `WindowWrapper` に置く。ウィンドウは `WindowId` で指定する
- wgpu/winit のラッパーは `XxxWrapper`、ラップ対象のフィールド名は `nature`
- GPU用構造体は `#[repr(C)]` + `bytemuck::Pod/Zeroable`。WGSLの `@location` / `@group` と必ず同期させる
- OS 依存コードは `app.rs` に閉じ込める。ファイルの読み書きは `assets.rs` を通す
- シェーダの作成は `shader::capture_validation` で包み、失敗しても落ちないようにする
- `Product` や公開している型を変えたら、使っている製品（このリポジトリでは `menu-proto` と `examples/demo3d`、ほかのリポジトリのゲーム）が壊れる。`cargo build --workspace --examples` で確かめる

## 実行
```bash
cargo run -p engine --example demo3d   # 3D の見本（WASD=カメラ, R=読み直し）
cargo test -p engine
```
