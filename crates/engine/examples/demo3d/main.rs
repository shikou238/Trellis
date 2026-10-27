//! learn-wgpu チュートリアル由来の 3D（五角形 ×100、テクスチャ、カメラ）を `Product` で動かす見本。
//! 製品が自分の描画パスをエンジンに足す方法を示す。`cargo run -p engine --example demo3d`
//! WASD=カメラ、R=shader.wgsl と pentagon.json の読み直し

mod camera;
mod demo3d;
mod model_instance;
mod shader;
mod vertex;

use engine::winit::{keyboard::KeyCode, window::WindowId};
use engine::{Frame, Gpu, Product, WindowSpec, wgpu};

struct Demo(demo3d::Demo3d);

impl Product for Demo {
    fn key(&mut self, window: WindowId, code: KeyCode, pressed: bool, _repeat: bool) {
        self.0.key(window, code, pressed);
    }
    fn reload(&mut self, gpu: &Gpu) {
        self.0.reload(gpu);
    }
    fn update(&mut self, gpu: &Gpu, frame: &mut Frame) {
        self.0.update(gpu, frame.window, frame.logical_size);
    }
    fn render(&self, window: WindowId, render_pass: &mut wgpu::RenderPass) {
        self.0.render(window, render_pass);
    }
}

fn main() {
    // shader.wgsl・pentagon.json・happy-tree.png はこのディレクトリからの相対パスで読む
    std::env::set_current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/demo3d")).expect("could not enter the example directory");
    engine::run(vec![WindowSpec { title: "demo3d".into(), size: None }], |gpu, windows| {
        let mut demo = demo3d::Demo3d::new(gpu)?;
        for &window in windows {
            demo.add_window(gpu, window);
        }
        Ok(Demo(demo))
    })
    .unwrap();
}
