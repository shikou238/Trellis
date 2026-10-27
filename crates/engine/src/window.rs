use std::sync::Arc;

use winit::{keyboard::KeyCode, window::Window};

use crate::hsv_to_rgb;
use crate::input::{InputEvent, InputMapper};
use crate::product::{Frame, Gpu, Product};
use crate::texture::TextureWrapper;
use crate::renderer::{UiLayer, UiRenderer};
use crate::text::{TextLayer, TextSystem};

pub struct WindowWrapper{
    pub nature: Arc<Window>,
    // todo why lifetime is needed here?
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    pub is_surface_configured: bool,
    pub last_cursor_position: Option<winit::dpi::PhysicalPosition<f64>>,
    pub depth_texture: TextureWrapper,
    pub ui_layer: UiLayer,
    pub text_layer: TextLayer,
    input: InputMapper,
    /// 背景色（線形 RGBA）。None ならカーソル位置の色
    pub background: Option<[f32; 4]>,
}

impl WindowWrapper {
    pub fn new(window: Arc<Window>, instance: &wgpu::Instance, gpu: &Gpu, adapter: &wgpu::Adapter, ui_renderer: &UiRenderer, text_system: &TextSystem) -> Self {
        let size = window.inner_size();

        let surface = instance.create_surface(window.clone()).unwrap();

        let surface_caps = surface.get_capabilities(&adapter);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: gpu.surface_format,
            width: size.width,
            height: size.height,
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        let depth_texture = TextureWrapper::create_depth_texture(&gpu.device, &config, "depth_texture");

        Self {
            nature: window,
            surface,
            config,
            is_surface_configured: false,
            last_cursor_position: None,
            depth_texture,
            ui_layer: UiLayer::new(&gpu.device, ui_renderer),
            text_layer: TextLayer::new(&gpu.device, &gpu.queue, text_system),
            input: InputMapper::default(),
            background: None,
        }
    }
    pub fn resize(&mut self, _width: u32, _height: u32,device: &wgpu::Device) {
        let width = _width.max(1);
        let height = _height.max(1);
        if width > 0 && height > 0 {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(device, &self.config);
            self.is_surface_configured = true;
        }
        self.depth_texture = crate::texture::TextureWrapper::create_depth_texture(device, &self.config, "depth_texture");

    }
    /// キーを製品に渡し、抽象的な操作にしたものも渡す
    pub fn handle_key(&mut self, product: &mut dyn Product, code: KeyCode, is_pressed: bool, repeat: bool) {
        let id = self.nature.id();
        product.key(id, code, is_pressed, repeat);

        let mut inputs: Vec<InputEvent> = Vec::new();
        self.input.key(code, is_pressed, repeat, &mut inputs);
        if !inputs.is_empty() {
            product.inputs(id, &inputs);
        }
    }
    /// フォーカスを失うとキーの解放が届かないので、押されているものをすべて離す
    pub fn focus_lost(&mut self, product: &mut dyn Product) {
        let mut inputs = Vec::new();
        self.input.release_all(&mut inputs);
        if !inputs.is_empty() {
            product.inputs(self.nature.id(), &inputs);
        }
    }
    pub fn scale_factor(&self) -> f32 {
        self.nature.scale_factor() as f32
    }
    pub fn logical_size(&self) -> [f32; 2] {
        let scale = self.scale_factor();
        [self.config.width as f32 / scale, self.config.height as f32 / scale]
    }
    /// カーソル位置（論理ピクセル）
    pub fn logical_cursor(&self) -> Option<[f32; 2]> {
        let scale = self.scale_factor() as f64;
        self.last_cursor_position.map(|p| [(p.x / scale) as f32, (p.y / scale) as f32])
    }
    /// 製品にこのウィンドウの中身（矩形・文字・背景色）を作らせて GPU に送る
    pub fn update(&mut self, gpu: &Gpu, text_system: &mut TextSystem, product: &mut dyn Product) {
        let logical_size = self.logical_size();
        self.ui_layer.instances.clear();
        self.text_layer.labels.clear();
        let mut frame = Frame {
            window: self.nature.id(),
            logical_size,
            rects: &mut self.ui_layer.instances,
            labels: &mut self.text_layer.labels,
            background: None,
        };
        product.update(gpu, &mut frame);
        self.background = frame.background;
        self.ui_layer.upload(&gpu.device, &gpu.queue, logical_size);
        self.text_layer.prepare(&gpu.device, &gpu.queue, text_system, [self.config.width, self.config.height], self.scale_factor());
    }
    pub fn cursor_moved(&mut self, position: winit::dpi::PhysicalPosition<f64>) {
        self.last_cursor_position = Some(position);
    }

    pub fn render(&mut self, gpu: &Gpu, ui_renderer: &UiRenderer, product: &dyn Product) -> Result<(), wgpu::SurfaceError>{
        self.nature.request_redraw();

        // We can't render unless the surface is configured
        if !self.is_surface_configured {
            return Ok(());
        }

        let output = self.surface.get_current_texture()?;

        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(
                            match (self.background, self.last_cursor_position){
                                (Some([r, g, b, a]), _) => wgpu::Color{r: r as f64, g: g as f64, b: b as f64, a: a as f64},
                                (None, None) => wgpu::Color{r: 0.1, g: 0.2, b: 0.3, a: 1.0},
                                (None, Some(winit::dpi::PhysicalPosition{x, y})) => {
                                    let size = self.nature.inner_size();
                                    hsv_to_rgb(x / size.width as f64, 0.5, y / size.height as f64)
                                }}),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_texture.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            product.render(self.nature.id(), &mut render_pass);

            // UI と文字は製品の描画の上に重ねる
            self.ui_layer.draw(&mut render_pass, ui_renderer);
            self.text_layer.draw(&mut render_pass);
        }

        // submit will accept anything that implements IntoIter
        gpu.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }
}
