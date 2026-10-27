use crate::window::WindowWrapper;

use std::sync::Arc;

use winit::window::{Window, WindowId};

use crate::product::Gpu;
use crate::renderer::UiRenderer;
use crate::text::TextSystem;

/// 全ウィンドウで共有する GPU リソースとウィンドウの一覧。製品の中身は知らない
pub struct AppView {
    pub gpu: Gpu,
    pub windows: Vec<WindowWrapper>,
    pub ui_renderer: UiRenderer,
    pub text_system: TextSystem,
}

impl AppView {
    pub async fn new(windows: Vec<Arc<Window>>) -> anyhow::Result<Self> {
        let window = &windows[0];

        // The instance is a handle to our GPU
        // BackendBit::PRIMARY => Vulkan + Metal + DX12 + Browser WebGPU
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            #[cfg(not(target_arch = "wasm32"))]
            backends: wgpu::Backends::PRIMARY,
            #[cfg(target_arch = "wasm32")]
            backends: wgpu::Backends::GL,
            ..Default::default()
        });

        let surface = instance.create_surface(window.clone()).unwrap();

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await?;

        let surface_caps = surface.get_capabilities(&adapter);
        // Shader code in this tutorial assumes an sRGB surface texture. Using a different
        // one will result in all the colors coming out darker. If you want to support non
        // sRGB surfaces, you'll need to account for that when drawing to the frame.
        let surface_format = surface_caps.formats.iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

         let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::empty(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                // WebGL doesn't support all of wgpu's features, so if
                // we're building for the web we'll have to disable some.
                required_limits: if cfg!(target_arch = "wasm32") {
                    wgpu::Limits::downlevel_webgl2_defaults()
                } else {
                    wgpu::Limits::default()
                },
                memory_hints: Default::default(),
                trace: wgpu::Trace::Off,
            })
            .await?;
        let gpu = Gpu { device, queue, surface_format };

        let ui_renderer = UiRenderer::new(&gpu.device, surface_format)?;

        let text_system = TextSystem::new(&gpu.device, surface_format);

        let window_wrappers = windows
            .into_iter()
            .map(|window| WindowWrapper::new(window, &instance, &gpu, &adapter, &ui_renderer, &text_system))
            .collect();

        Ok(Self {
            gpu,
            windows: window_wrappers,
            ui_renderer,
            text_system,
        })
    }

    pub fn window_ids(&self) -> Vec<WindowId> {
        self.windows.iter().map(|w| w.nature.id()).collect()
    }

    /// エンジン自身の定義（UI シェーダ）を読み直す。製品の分は `Product::reload`
    pub fn reload(&mut self){
        if let Err(e) = self.ui_renderer.reload(&self.gpu.device) {
            log::error!("Failed to reload UI shader: {:?}", e);
        }
    }

    pub fn close(&mut self, window_id: WindowId) {
        self.windows.retain(|w| w.nature.id() != window_id);
    }
}
