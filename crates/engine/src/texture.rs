use anyhow::Context;
use image::AnimationDecoder;
use wgpu::util::DeviceExt;

use crate::assets;

/// 読み込んだ絵。`rgba` は 1 画素 4 バイトで、上の行から並ぶ
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    /// 左上 (x, y) から幅 w・高さ h を切り出す。はみ出したら `Err`
    pub fn crop(&self, x: u32, y: u32, w: u32, h: u32) -> anyhow::Result<Image> {
        if x.checked_add(w).is_none_or(|r| r > self.width) || y.checked_add(h).is_none_or(|b| b > self.height) {
            anyhow::bail!("crop {}x{} at ({}, {}) is outside the {}x{} image", w, h, x, y, self.width, self.height);
        }
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for row in y..y + h {
            let start = ((row * self.width + x) * 4) as usize;
            rgba.extend_from_slice(&self.rgba[start..start + (w * 4) as usize]);
        }
        Ok(Image { width: w, height: h, rgba })
    }

    /// 同じ大きさのコマ（cols 列 × rows 段）に切り分ける。左上から行ごとの順。割り切れなければ `Err`
    pub fn grid(&self, cols: u32, rows: u32) -> anyhow::Result<Vec<Image>> {
        if cols == 0 || rows == 0 || self.width % cols != 0 || self.height % rows != 0 {
            anyhow::bail!("the {}x{} image cannot be split into {} columns and {} rows", self.width, self.height, cols, rows);
        }
        let (w, h) = (self.width / cols, self.height / rows);
        (0..rows).flat_map(|r| (0..cols).map(move |c| (c, r))).map(|(c, r)| self.crop(c * w, r * h, w, h)).collect()
    }
}

/// GIF の 1 フレーム。前のフレームに重ね合わせた後の完成した絵
pub struct GifFrame {
    pub image: Image,
    /// 表示時間（1/100 秒単位。GIF の書き方と同じ）
    pub delay: u32,
}

/// PNG・JPEG・GIF（最初のフレーム）を読む
pub fn load_image(path: &str) -> anyhow::Result<Image> {
    let bytes = assets::read_bytes(path)?;
    let rgba = image::load_from_memory(&bytes).with_context(|| format!("failed to decode {}", path))?.to_rgba8();
    Ok(Image { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() })
}

/// GIF のすべてのフレームを読む。フレームが前との差分でも、重ね合わせた後の絵を返す
pub fn load_gif(path: &str) -> anyhow::Result<Vec<GifFrame>> {
    let bytes = assets::read_bytes(path)?;
    let decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).with_context(|| format!("failed to decode {}", path))?;
    decoder
        .into_frames()
        .map(|frame| {
            let frame = frame.with_context(|| format!("failed to decode a frame of {}", path))?;
            let (numer, denom) = frame.delay().numer_denom_ms();
            let delay = (numer as f64 / denom as f64 / 10.0).round() as u32;
            let rgba = frame.into_buffer();
            Ok(GifFrame { image: Image { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() }, delay })
        })
        .collect()
}

pub struct TextureWrapper {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
}

impl TextureWrapper {
    pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float; // 1.

    /// 絵のファイルを読んでテクスチャにする。`filter` は拡大・縮小のしかた（ドット絵なら `Nearest`）
    pub fn new(path: &str, filter: wgpu::FilterMode, device: &wgpu::Device, queue: &wgpu::Queue) -> anyhow::Result<Self> {
        let image = load_image(path)?;
        Ok(Self::from_image(&image, filter, path, device, queue))
    }

    pub fn from_image(image: &Image, filter: wgpu::FilterMode, label: &str, device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self::create(image.width, image.height, 1, wgpu::TextureViewDimension::D2, &image.rgba, filter, label, device, queue)
    }

    /// 同じ大きさの絵を層にしたテクスチャ配列（ビューは 1 層でも `D2Array`）。アニメーションのフレームを層にすると、
    /// インスタンスごとに何層目かを渡すだけで、全員を別々のフレームで 1 回のインスタンシングで描ける
    pub fn from_layers(layers: &[Image], filter: wgpu::FilterMode, label: &str, device: &wgpu::Device, queue: &wgpu::Queue) -> anyhow::Result<Self> {
        let first = layers.first().with_context(|| format!("{}: no layers", label))?;
        let (width, height) = (first.width, first.height);
        if let Some((i, layer)) = layers.iter().enumerate().find(|(_, l)| (l.width, l.height) != (width, height)) {
            anyhow::bail!("{}: layer {} is {}x{}, expected {}x{}", label, i, layer.width, layer.height, width, height);
        }
        let rgba: Vec<u8> = layers.iter().flat_map(|l| l.rgba.iter().copied()).collect();
        Ok(Self::create(width, height, layers.len() as u32, wgpu::TextureViewDimension::D2Array, &rgba, filter, label, device, queue))
    }

    #[allow(clippy::too_many_arguments)]
    fn create(width: u32, height: u32, layers: u32, dimension: wgpu::TextureViewDimension, rgba: &[u8], filter: wgpu::FilterMode, label: &str, device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let texture = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: layers },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                // Most images are stored using sRGB, so we need to reflect that here.
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            rgba,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(dimension), ..Default::default() });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some(label),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: filter,
            min_filter: filter,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        Self { texture, view, sampler }
    }
     pub fn create_depth_texture(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration, label: &str) -> Self {
        let size = wgpu::Extent3d { // 2.
            width: config.width.max(1),
            height: config.height.max(1),
            depth_or_array_layers: 1,
        };
        let desc = wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT // 3.
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };
        let texture = device.create_texture(&desc);

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(
            &wgpu::SamplerDescriptor { // 4.
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                compare: Some(wgpu::CompareFunction::LessEqual), // 5.
                lod_min_clamp: 0.0,
                lod_max_clamp: 100.0,
                ..Default::default()
            }
        );

        Self { texture, view, sampler }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use image::{Delay, Frame, RgbaImage, codecs::gif::GifEncoder};

    #[test]
    fn gif_frames_keep_pixels_and_delay() {
        let path = std::env::temp_dir().join(format!("engine-test-{}.gif", std::process::id()));
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut encoder = GifEncoder::new(file);
            let red = RgbaImage::from_pixel(2, 3, image::Rgba([255, 0, 0, 255]));
            let blue = RgbaImage::from_pixel(2, 3, image::Rgba([0, 0, 255, 255]));
            encoder.encode_frame(Frame::from_parts(red, 0, 0, Delay::from_numer_denom_ms(100, 1))).unwrap();
            encoder.encode_frame(Frame::from_parts(blue, 0, 0, Delay::from_numer_denom_ms(250, 1))).unwrap();
        }
        let frames = load_gif(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(frames.len(), 2);
        assert_eq!((frames[0].image.width, frames[0].image.height), (2, 3));
        assert_eq!(frames[0].image.rgba.len(), 2 * 3 * 4);
        assert_eq!(&frames[0].image.rgba[..4], &[255, 0, 0, 255]);
        assert_eq!(&frames[1].image.rgba[..4], &[0, 0, 255, 255]);
        assert_eq!((frames[0].delay, frames[1].delay), (10, 25));
    }

    /// 画素の値 = (x, y) の番号。どこから切り出したかが分かる
    fn numbered(width: u32, height: u32) -> Image {
        let rgba = (0..height).flat_map(|y| (0..width).flat_map(move |x| [x as u8, y as u8, 0, 255])).collect();
        Image { width, height, rgba }
    }

    #[test]
    fn crop_takes_the_given_rectangle() {
        let piece = numbered(4, 3).crop(1, 1, 2, 2).unwrap();
        assert_eq!((piece.width, piece.height), (2, 2));
        let pixels: Vec<_> = piece.rgba.chunks(4).map(|p| (p[0], p[1])).collect();
        assert_eq!(pixels, [(1, 1), (2, 1), (1, 2), (2, 2)]);
        assert!(numbered(4, 3).crop(3, 0, 2, 1).is_err());
        assert!(numbered(4, 3).crop(0, 2, 1, 2).is_err());
    }

    #[test]
    fn grid_goes_row_by_row_from_top_left() {
        let cells = numbered(6, 4).grid(3, 2).unwrap();
        assert_eq!(cells.len(), 6);
        let corners: Vec<_> = cells.iter().map(|c| (c.width, c.height, c.rgba[0], c.rgba[1])).collect();
        assert_eq!(corners, [(2, 2, 0, 0), (2, 2, 2, 0), (2, 2, 4, 0), (2, 2, 0, 2), (2, 2, 2, 2), (2, 2, 4, 2)]);
        assert!(numbered(6, 4).grid(4, 2).is_err());
        assert!(numbered(6, 4).grid(0, 2).is_err());
    }

    #[test]
    fn missing_image_is_an_error() {
        assert!(load_image("no/such/file.png").is_err());
        assert!(load_gif("no/such/file.gif").is_err());
    }
}
