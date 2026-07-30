mod app;
mod appview;
mod window;
mod vertex;
mod texture;
mod shader;
mod camera;
mod model_instance;


use std::sync::Arc;

use winit::{
    application::ApplicationHandler, event::*, event_loop::{ActiveEventLoop, EventLoop}, keyboard::{KeyCode, PhysicalKey}, window::Window
};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
use env_logger::Env;



pub fn run() -> anyhow::Result<()> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        env_logger::init_from_env(Env::new().default_filter_or("debug"));
    }
    #[cfg(target_arch = "wasm32")]
    {
        // console_log::init_with_level(log::Level::Info).unwrap_throw();
        console_log::init_with_level(log::Level::Info).unwrap();
    }

    let event_loop = EventLoop::with_user_event().build()?;
    let mut app = app::App::new(
        #[cfg(target_arch = "wasm32")]
        &event_loop,
    );
    event_loop.run_app(&mut app)?;

    Ok(())
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn run_web() -> Result<(), wasm_bindgen::JsValue> {
    console_error_panic_hook::set_once();
    //run().unwrap_throw();
    run().unwrap();

    Ok(())
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