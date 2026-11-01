//! エンジンと製品（メニュー試作、ゲーム、作曲ソフト）の境界。
//! エンジンはウィンドウ・GPU・入力・矩形と文字の描画を持ち、製品の中身は `Product` 越しにだけ呼ぶ。

use std::time::Duration;

use winit::{keyboard::KeyCode, window::WindowId};

use crate::input::InputEvent;
use crate::renderer::RectInstance;
use crate::text::Label;

/// 全ウィンドウで共有する GPU の入口。製品が自分の描画パイプラインを作るときに使う
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface_format: wgpu::TextureFormat,
}

/// 起動時に開くウィンドウ。wasm では先頭の 1 つだけ（canvas）
pub struct WindowSpec {
    pub title: String,
    /// 論理ピクセル。None なら OS に任せる
    pub size: Option<[f64; 2]>,
}

/// 1 ウィンドウの 1 フレーム分。製品は `rects` と `labels` を詰め、背景色を決める（毎フレーム空から）
pub struct Frame<'a> {
    pub window: WindowId,
    pub logical_size: [f32; 2],
    /// このウィンドウの前のフレームからの経過時間。最初のフレームは 0。ウィンドウが複数あると、それぞれのウィンドウで呼ばれる
    pub dt: Duration,
    pub rects: &'a mut Vec<RectInstance>,
    pub labels: &'a mut Vec<Label>,
    /// 線形 RGBA。None ならカーソル位置の色
    pub background: Option<[f32; 4]>,
}

/// マウス（将来はタッチも）。座標は論理ピクセル
pub struct Pointer {
    pub window: WindowId,
    pub position: [f32; 2],
    pub window_size: [f32; 2],
    pub kind: PointerKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerKind {
    Moved,
    /// 左ボタンを押した
    Down,
    /// 左ボタンを離した
    Up,
}

/// 製品側が実装する。エンジンはイベントとフレームのたびにこれを呼ぶ
pub trait Product: 'static {
    /// 物理キー（Esc はエンジンが終了に使う。R はエンジンが自分のシェーダを読み直したあと `reload` を呼ぶ）。
    /// フォーカスを失ったときは、押されているキーをすべて `pressed = false` で離す
    fn key(&mut self, _window: WindowId, _code: KeyCode, _pressed: bool, _repeat: bool) {}
    /// キーを抽象的な操作にしたもの。フォーカスを失ったときの一斉解放もここに来る
    fn inputs(&mut self, _window: WindowId, _inputs: &[InputEvent]) {}
    fn pointer(&mut self, _pointer: Pointer) {}
    /// R キー: 定義を読み直す（実行時の状態は保持する）
    fn reload(&mut self, _gpu: &Gpu) {}
    /// 描画の前に毎フレーム呼ぶ
    fn update(&mut self, gpu: &Gpu, frame: &mut Frame);
    /// 製品自身の描画（3D など）。矩形と文字はこのあとエンジンが上に重ねる
    fn render(&self, _window: WindowId, _render_pass: &mut wgpu::RenderPass) {}
    /// ウィンドウが閉じられた。true を返すとアプリを終了する
    fn window_closed(&mut self, _window: WindowId) -> bool {
        true
    }
}
