use crate::{shader::ShaderWrapper, texture, vertex::Vertex, window::WindowWrapper};

use std::sync::Arc;
use crate::camera;

use winit::{
     event_loop::ActiveEventLoop, keyboard::KeyCode, window::{self, Window}
};
use wgpu::util::DeviceExt;


use crate::vertex;
use crate::model_instance::{ModelInstance, ModelInstanceRaw};

use cgmath::prelude::*;

pub struct AppView {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub render_pipeline: wgpu::RenderPipeline,
    pub model_buffer: vertex::ModelBuffer,
    pub shader: ShaderWrapper,
    pub diffuse_bind_group: wgpu::BindGroup,
    pub windows: Vec<WindowWrapper>,
    pub model_instances: Vec<ModelInstance>,
    pub model_instance_buffer: wgpu::Buffer,
}

impl AppView {
    // We don't need this to be async right now,
    // but we will in the next tutorial
    pub async fn new(windows: Vec<Arc<Window>>) -> anyhow::Result<Self> {
        //todo : support multiple windows
        let window = &windows[0];

        let size = window.inner_size();

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

        let shader = ShaderWrapper::new( &device)?;

        let render_pipeline = match Self::constuct_render_pipeline(&device, &shader, surface_format){
            Ok(pipeline) => pipeline,
            Err(e) => {
                log::error!("Failed to construct render pipeline: {:?}", e);
                return Err(e);
            }
        };
        // let model = vertex::pentagon();
        // let model_buffer = model.create_buffer(&device);
        
        //temporary code to save model data to json file
        //model.save_sexpr_file("pentagon.json").expect("Failed to save model to file");

        let model_buffer = Self::load_model_and_create_buffer(&device)?;

        let diffuse_texture = texture::TextureWrapper::new("happy-tree.png", &device, &queue);

        let diffuse_bind_group = device.create_bind_group(
            &wgpu::BindGroupDescriptor {
                layout: &shader.get_texture_bind_group_layout(),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&diffuse_texture.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&diffuse_texture.sampler),
                    }
                ],
                label: Some("diffuse_bind_group"),
            }
        );

        
        let window_wrappers = windows.into_iter().map(|window| WindowWrapper::new(window, &instance, &device,&adapter, surface_format)).collect();
        
        const NUM_INSTANCES_PER_ROW: u32 = 10;
        const INSTANCE_DISPLACEMENT: cgmath::Vector3<f32> = cgmath::Vector3::new(NUM_INSTANCES_PER_ROW as f32 * 0.5, 0.0, NUM_INSTANCES_PER_ROW as f32 * 0.5);

        let instances = (0..NUM_INSTANCES_PER_ROW).flat_map(|z| {
            (0..NUM_INSTANCES_PER_ROW).map(move |x| {
                let position = cgmath::Vector3 { x: x as f32, y: 0.0, z: z as f32 } - INSTANCE_DISPLACEMENT;

                let rotation = if position.is_zero() {
                    // this is needed so an object at (0, 0, 0) won't get scaled to zero
                    // as Quaternions can affect scale if they're not created correctly
                    cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_z(), cgmath::Deg(0.0))
                } else {
                    cgmath::Quaternion::from_axis_angle(position.normalize(), cgmath::Deg(45.0))
                };

                ModelInstance {
                    id: (z * NUM_INSTANCES_PER_ROW + x) as u32,
                    texture_index: 0,
                    position, rotation,
                }
            })
        }).collect::<Vec<_>>();

        let instance_data = instances.iter().map(ModelInstance::to_raw).collect::<Vec<_>>();
        let instance_buffer = device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("Instance Buffer"),
                contents: bytemuck::cast_slice(&instance_data),
                usage: wgpu::BufferUsages::VERTEX,
            }
        );

        Ok(Self {
            device,
            queue,
            render_pipeline,
            model_buffer,
            shader,
            diffuse_bind_group,
            windows: window_wrappers,
            model_instances: instances,
            model_instance_buffer: instance_buffer,
        })
    }
    fn reconstruct_render_pipeline(&mut self) -> anyhow::Result<()> {
        log::debug!("Reconstructing render pipeline...--------------------------------------------------------------");
        let surface_format = self.windows[0].config.format;
        self.render_pipeline = match Self::constuct_render_pipeline(&self.device, &self.shader, surface_format){
            Ok(pipeline) => pipeline,
            Err(e) => {
                log::error!("Failed to reconstruct render pipeline: {:?}", e);
                return Err(e);
            }
        };
        log::debug!("Render pipeline reconstructed successfully.");
        Ok(())
    }
    fn constuct_render_pipeline(device: &wgpu::Device,shader_wrapper: &ShaderWrapper,surface_format: wgpu::TextureFormat) -> anyhow::Result<wgpu::RenderPipeline> {
        
        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &shader_wrapper.bind_group_layouts.iter().collect::<Vec<_>>(),
                immediate_size: 0,
            });
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader_wrapper.nature,
                entry_point: Some("vs_main"), // 1.
                buffers: &[Vertex::LAYOUT, ModelInstanceRaw::desc()], // 2.
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState { // 3.
                module: &shader_wrapper.nature,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState { // 4.
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
             primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList, // 1.
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw, // 2.
                cull_mode: Some(wgpu::Face::Back),
                // Setting this to anything other than Fill requires Features::NON_FILL_POLYGON_MODE
                polygon_mode: wgpu::PolygonMode::Fill,
                // Requires Features::DEPTH_CLIP_CONTROL
                unclipped_depth: false,
                // Requires Features::CONSERVATIVE_RASTERIZATION
                conservative: false,
            },
             depth_stencil: Some(wgpu::DepthStencilState {
                format: texture::TextureWrapper::DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less, // 1.
                stencil: wgpu::StencilState::default(), // 2.
                bias: wgpu::DepthBiasState::default(),
            }), // 1.
            multisample: wgpu::MultisampleState {
                count: 1, // 2.
                mask: !0, // 3.
                alpha_to_coverage_enabled: false, // 4.
            },
            multiview_mask: None, // 5.
            cache: None, // 6.compile result
        });
        Ok(render_pipeline)
    }
    fn load_model_and_create_buffer(device: &wgpu::Device) -> anyhow::Result<vertex::ModelBuffer> {
        let model = vertex::Model::load_sexpr_file("pentagon.json")?;
        Ok(model.create_buffer(device))
    }
    fn reload_model(&mut self) -> anyhow::Result<()> {
        match Self::load_model_and_create_buffer(&self.device){
            Ok(model_buffer) => {
                self.model_buffer = model_buffer;
                Ok(())
            },
            Err(e) => Err(e)
        }
    }
    fn report_err(anyhow_err: anyhow::Result<()>, report : fn(&anyhow::Error)) {
        match anyhow_err {
            Ok(_) => {},
            Err(e) => {
                report(&e);
            }
        }
    }
    
    pub fn reload(&mut self){
        Self::report_err(self.shader.reload(&self.device), {|e| log::error!("Failed to reload shader: {:?}", e)});
        Self::report_err(self.reconstruct_render_pipeline(), {|e| log::error!("Failed to reload render pipeline: {:?}", e)});
        Self::report_err(self.reload_model(), {|e| log::error!("Failed to reload model: {:?}", e)});
    }
    pub fn close(&mut self, window_id: winit::window::WindowId) -> bool{
        self.windows.retain(|w| w.nature.id() != window_id);
        self.windows.is_empty()
    }
}
