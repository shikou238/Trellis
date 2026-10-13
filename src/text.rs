use glyphon::{
    Attrs, Buffer, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer,
};

pub struct GlyphonText {
    font_system: FontSystem,
    swash_cache: SwashCache,
    atlas: TextAtlas,
    renderer: TextRenderer,
    buffer: Buffer,
}

impl GlyphonText {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();

        let mut atlas = TextAtlas::new(device, queue, target_format);
        let renderer = TextRenderer::new(
            &mut atlas,
            device,
            wgpu::MultisampleState::default(),
            None,
        );

        // フォントサイズ 32px、行間 42px
        let mut buffer = Buffer::new(&mut font_system, Metrics::new(32.0, 42.0));

        buffer.set_size(
            &mut font_system,
            Some(width as f32 - 48.0),
            Some(height as f32 - 48.0),
        );

        buffer.set_text(
            &mut font_system,
            "こんにちは、glyphon!\nwgpu 上に日本語テキストを描画しています。",
            Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
        );

        // テキストをシェイピングし、描画可能なグリフ列を作る
        buffer.shape_until_scroll(&mut font_system, false);

        Self {
            font_system,
            swash_cache,
            atlas,
            renderer,
            buffer,
        }
    }

    pub fn set_text(&mut self, text: &str) {
        self.buffer.set_text(
            &mut self.font_system,
            text,
            Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
        );

        self.buffer.shape_until_scroll(&mut self.font_system, false);
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.buffer.set_size(
            &mut self.font_system,
            Some(width as f32 - 48.0),
            Some(height as f32 - 48.0),
        );

        self.buffer.shape_until_scroll(&mut self.font_system, false);
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) {
        self.renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                Resolution { width, height },
                [TextArea {
                    buffer: &self.buffer,
                    left: 24.0,
                    top: 24.0,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: 0,
                        top: 0,
                        right: width as i32,
                        bottom: height as i32,
                    },
                    default_color: Color::rgb(255, 255, 255),
                    custom_glyphs: &[],
                }],
                &mut self.swash_cache,
            )
            .expect("glyphon の prepare に失敗しました");
    }

    pub fn render<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) {
        self.renderer
            .render(&self.atlas, render_pass)
            .expect("glyphon の render に失敗しました");
    }
}