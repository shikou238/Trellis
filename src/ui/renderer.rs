//! UI 用の 2D 矩形描画。
//! パイプラインは全ウィンドウで共有（`UiRenderer` → AppView）、
//! 画面サイズとインスタンスのバッファはウィンドウ毎（`UiLayer` → WindowWrapper）。

use wgpu::util::DeviceExt;

use crate::{assets, shader, texture::TextureWrapper};

const SHADER_PATH: &str = "src/ui.wgsl";

/// 1 矩形分のインスタンスデータ。座標は論理ピクセル、左上原点・y 下向き
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RectInstance {
    /// 左上
    pub position: [f32; 2],
    pub size: [f32; 2],
    /// 線形 RGBA
    pub color: [f32; 4],
    /// ラジアン。矩形の中心まわり
    pub rotation: f32,
}

impl RectInstance {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] =
        wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32];

    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<RectInstance>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &Self::ATTRIBUTES,
    };
}

/// WGSL の `Screen` と一致させる（uniform は 16 バイト単位）
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct ScreenUniform {
    size: [f32; 2],
    _padding: [f32; 2],
}

pub struct UiRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    surface_format: wgpu::TextureFormat,
}

impl UiRenderer {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> anyhow::Result<Self> {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui_screen_bind_group_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline = Self::build_pipeline(device, &bind_group_layout, surface_format)?;
        Ok(Self {
            pipeline,
            bind_group_layout,
            surface_format,
        })
    }

    /// シェーダを読み直す。失敗したら今のパイプラインのまま
    pub fn reload(&mut self, device: &wgpu::Device) -> anyhow::Result<()> {
        self.pipeline = Self::build_pipeline(device, &self.bind_group_layout, self.surface_format)?;
        Ok(())
    }

    fn build_pipeline(
        device: &wgpu::Device,
        bind_group_layout: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
    ) -> anyhow::Result<wgpu::RenderPipeline> {
        let source = assets::read_to_string(SHADER_PATH)?;

        shader::capture_validation(device, || {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("UI Shader"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("UI Pipeline Layout"),
                bind_group_layouts: &[bind_group_layout],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("UI Pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    buffers: &[RectInstance::LAYOUT],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: surface_format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    // 回転や負のサイズで裏返っても描く
                    cull_mode: None,
                    ..Default::default()
                },
                // 3D と同じパスで描くので深度アタッチメントの形式は合わせるが、UI は常に手前
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: TextureWrapper::DEPTH_FORMAT,
                    depth_write_enabled: false,
                    depth_compare: wgpu::CompareFunction::Always,
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        })
    }
}

/// ウィンドウ毎の UI 描画リソース
pub struct UiLayer {
    pub instances: Vec<RectInstance>,
    screen_buffer: wgpu::Buffer,
    screen_bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    instance_count: u32,
}

impl UiLayer {
    pub fn new(device: &wgpu::Device, renderer: &UiRenderer) -> Self {
        let screen_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("UI Screen Buffer"),
            contents: bytemuck::cast_slice(&[ScreenUniform { size: [1.0, 1.0], _padding: [0.0; 2] }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let screen_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui_screen_bind_group"),
            layout: &renderer.bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: screen_buffer.as_entire_binding(),
            }],
        });
        let instance_capacity = 64;
        Self {
            instances: Vec::new(),
            screen_buffer,
            screen_bind_group,
            instance_buffer: Self::create_instance_buffer(device, instance_capacity),
            instance_capacity,
            instance_count: 0,
        }
    }

    fn create_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("UI Instance Buffer"),
            size: (capacity * std::mem::size_of::<RectInstance>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// `instances` と画面サイズ（論理ピクセル）を GPU に送る
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, logical_size: [f32; 2]) {
        queue.write_buffer(
            &self.screen_buffer,
            0,
            bytemuck::cast_slice(&[ScreenUniform { size: logical_size, _padding: [0.0; 2] }]),
        );

        if self.instances.len() > self.instance_capacity {
            self.instance_capacity = self.instances.len().next_power_of_two();
            self.instance_buffer = Self::create_instance_buffer(device, self.instance_capacity);
        }
        if !self.instances.is_empty() {
            queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&self.instances));
        }
        self.instance_count = self.instances.len() as u32;
    }

    pub fn draw(&self, render_pass: &mut wgpu::RenderPass, renderer: &UiRenderer) {
        if self.instance_count == 0 {
            return;
        }
        render_pass.set_pipeline(&renderer.pipeline);
        render_pass.set_bind_group(0, &self.screen_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        // 頂点バッファは使わず、vertex_index から四角形の角を作る
        render_pass.draw(0..6, 0..self.instance_count);
    }
}
