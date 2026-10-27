//! 文字の描画（glyphon）。
//! フォント（読み込みが重い）は全ウィンドウで共有（`TextSystem` → AppView）、
//! アトラスと描画パイプラインはウィンドウ毎（`TextLayer` → WindowWrapper）。

use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache, TextArea, TextAtlas,
    TextBounds, TextRenderer, Viewport,
};

use crate::texture::TextureWrapper;

pub struct TextSystem {
    font_system: FontSystem,
    swash_cache: SwashCache,
    cache: Cache,
    surface_format: wgpu::TextureFormat,
}

impl TextSystem {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        Self {
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
            cache: Cache::new(device),
            surface_format,
        }
    }
}

/// 1 つの文字列。位置とサイズは論理ピクセル
pub struct Label {
    pub text: String,
    /// 左上
    pub position: [f32; 2],
    pub size: f32,
    /// sRGB 0–255
    pub color: [u8; 4],
}

struct ShapedText {
    text: String,
    size: f32,
    buffer: Buffer,
}

/// ウィンドウ毎の文字描画。`labels` を毎フレーム詰め直して `prepare` → `draw` する
pub struct TextLayer {
    pub labels: Vec<Label>,
    atlas: TextAtlas,
    renderer: TextRenderer,
    viewport: Viewport,
    /// labels と同じ並び。文字列とサイズが変わらなければシェイピングし直さない
    shaped: Vec<ShapedText>,
}

impl TextLayer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, system: &TextSystem) -> Self {
        let mut atlas = TextAtlas::new(device, queue, &system.cache, system.surface_format);
        // 3D と同じレンダーパスで描くので深度アタッチメントの形式は合わせるが、文字は常に手前
        let depth_stencil = wgpu::DepthStencilState {
            format: TextureWrapper::DEPTH_FORMAT,
            depth_write_enabled: false,
            depth_compare: wgpu::CompareFunction::Always,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        };
        let renderer = TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), Some(depth_stencil));
        Self {
            labels: Vec::new(),
            atlas,
            renderer,
            viewport: Viewport::new(device, &system.cache),
            shaped: Vec::new(),
        }
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        system: &mut TextSystem,
        physical_size: [u32; 2],
        scale: f32,
    ) {
        self.viewport.update(queue, Resolution { width: physical_size[0], height: physical_size[1] });

        self.shaped.truncate(self.labels.len());
        for (i, label) in self.labels.iter().enumerate() {
            if self.shaped.get(i).is_some_and(|s| s.text == label.text && s.size == label.size) {
                continue;
            }
            let font_system = &mut system.font_system;
            let mut buffer = Buffer::new(font_system, Metrics::new(label.size, label.size * 1.3));
            buffer.set_size(font_system, None, None);
            buffer.set_text(font_system, &label.text, &Attrs::new().family(Family::SansSerif), Shaping::Advanced, None);
            buffer.shape_until_scroll(font_system, false);
            let shaped = ShapedText { text: label.text.clone(), size: label.size, buffer };
            if i < self.shaped.len() {
                self.shaped[i] = shaped;
            } else {
                self.shaped.push(shaped);
            }
        }

        let bounds = TextBounds { left: 0, top: 0, right: physical_size[0] as i32, bottom: physical_size[1] as i32 };
        let areas = self.labels.iter().zip(&self.shaped).map(|(label, shaped)| {
            let [r, g, b, a] = label.color;
            TextArea {
                buffer: &shaped.buffer,
                left: label.position[0] * scale,
                top: label.position[1] * scale,
                scale,
                bounds,
                default_color: Color::rgba(r, g, b, a),
                custom_glyphs: &[],
            }
        });

        // 前のフレームで使ったグリフの記録を消してから、今回使うものを詰め直す
        self.atlas.trim();
        if let Err(e) = self.renderer.prepare(
            device,
            queue,
            &mut system.font_system,
            &mut self.atlas,
            &self.viewport,
            areas,
            &mut system.swash_cache,
        ) {
            log::error!("text prepare failed: {}", e);
        }
    }

    pub fn draw(&self, render_pass: &mut wgpu::RenderPass) {
        if let Err(e) = self.renderer.render(&self.atlas, &self.viewport, render_pass) {
            log::error!("text render failed: {}", e);
        }
    }
}
