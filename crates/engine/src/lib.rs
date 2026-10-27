//! ウィンドウ・GPU・入力・矩形と文字の描画。製品（メニュー試作、ゲーム、作曲ソフト）は知らず、`Product` 越しに呼ぶ。

mod app;
mod appview;
mod window;
mod product;
pub mod assets;
pub mod input;
pub mod renderer;
pub mod shader;
pub mod text;
pub mod texture;

pub use product::{Frame, Gpu, Pointer, PointerKind, Product, WindowSpec};
// 製品側が同じ版を使えるように
pub use wgpu;
pub use winit;

use winit::window::WindowId;

#[cfg(not(target_arch = "wasm32"))]
use env_logger::Env;

/// ロガーを用意し、`windows` を開いて製品を動かす。`make_product` は GPU とウィンドウができたあとに 1 回呼ばれる
pub fn run<P: Product>(windows: Vec<WindowSpec>, make_product: impl FnOnce(&Gpu, &[WindowId]) -> anyhow::Result<P> + 'static) -> anyhow::Result<()> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        env_logger::init_from_env(Env::new().default_filter_or("debug"));
    }
    #[cfg(target_arch = "wasm32")]
    {
        console_error_panic_hook::set_once();
        console_log::init_with_level(log::Level::Info).unwrap();
    }
    app::run(windows, make_product)
}

/// HSV(0.0–1.0) -> RGB(0.0–1.0)
fn hsv_to_rgb(h: f64, s: f64, v: f64) -> wgpu::Color {
    let h = (h.rem_euclid(1.0)) * 6.0; // [0,6)
    let i = h.floor();
    let f = h - i;

    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));

    match i as i32 {
        0 => wgpu::Color{r: v, g: t, b: p, a: 1.0},
        1 => wgpu::Color{r: q, g: v, b: p, a: 1.0},
        2 => wgpu::Color{r: p, g: v, b: t, a: 1.0},
        3 => wgpu::Color{r: p, g: q, b: v, a: 1.0},
        4 => wgpu::Color{r: t, g: p, b: v, a: 1.0},
        _ => wgpu::Color{r: v, g: p, b: q, a: 1.0}, // i == 5 のケース
    }
}
