//! learn-wgpu チュートリアル由来の 3D 描画（五角形 ×100、テクスチャ、カメラ）。
//! エンジンの外で、製品が自分の描画パスを足す見本。

use cgmath::prelude::*;
use wgpu::util::DeviceExt;
use engine::Gpu;
use engine::texture::{self, TextureWrapper};
use engine::wgpu;
use engine::winit::{keyboard::KeyCode, window::WindowId};

use crate::camera;
use crate::model_instance::{ModelInstance, ModelInstanceRaw};
use crate::shader::ShaderWrapper;
use crate::vertex::{self, Vertex};

const MODEL_PATH: &str = "pentagon.json";
const TEXTURE_PATH: &str = "happy-tree.png";

pub struct Demo3d {
    shader: ShaderWrapper,
    render_pipeline: wgpu::RenderPipeline,
    model_buffer: vertex::ModelBuffer,
    diffuse_bind_group: wgpu::BindGroup,
    model_instances: Vec<ModelInstance>,
    model_instance_buffer: wgpu::Buffer,
    /// ウィンドウ毎のカメラ（3D を描くウィンドウだけ）
    views: Vec<View>,
}

struct View {
    window: WindowId,
    camera: camera::Camera,
    camera_uniform: camera::CameraUniform,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    camera_controller: CameraController,
}

impl Demo3d {
    pub fn new(gpu: &Gpu) -> anyhow::Result<Self> {
        let device = &gpu.device;
        let shader = ShaderWrapper::new(device)?;
        let render_pipeline = match Self::construct_render_pipeline(device, &shader, gpu.surface_format) {
            Ok(pipeline) => pipeline,
            Err(e) => {
                log::error!("Failed to construct render pipeline: {:?}", e);
                return Err(e);
            }
        };
        let model_buffer = Self::load_model_and_create_buffer(device)?;

        let diffuse_texture = TextureWrapper::new(TEXTURE_PATH, wgpu::FilterMode::Linear, device, &gpu.queue)?;
        let diffuse_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: shader.get_texture_bind_group_layout(),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&diffuse_texture.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&diffuse_texture.sampler),
                },
            ],
            label: Some("diffuse_bind_group"),
        });

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
        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Instance Buffer"),
            contents: bytemuck::cast_slice(&instance_data),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Ok(Self {
            shader,
            render_pipeline,
            model_buffer,
            diffuse_bind_group,
            model_instances: instances,
            model_instance_buffer: instance_buffer,
            views: Vec::new(),
        })
    }

    /// `window` に 3D を描くようにする
    pub fn add_window(&mut self, gpu: &Gpu, window: WindowId) {
        let camera = camera::Camera {
            // position the camera 1 unit up and 2 units back
            // +z is out of the screen
            eye: (0.0, 1.0, 2.0).into(),
            // have it look at the origin
            target: (0.0, 0.0, 0.0).into(),
            // which way is "up"
            up: cgmath::Vector3::unit_y(),
            aspect: 1.0,
            fovy: 45.0,
            znear: 0.1,
            zfar: 100.0,
        };
        let mut camera_uniform = camera::CameraUniform::new();
        camera_uniform.update_view_proj(&camera);
        let camera_buffer = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Camera Buffer"),
            contents: bytemuck::cast_slice(&[camera_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let camera_bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &self.shader.bind_group_layouts[1],
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
            label: Some("camera_bind_group"),
        });
        self.views.push(View {
            window,
            camera,
            camera_uniform,
            camera_buffer,
            camera_bind_group,
            camera_controller: CameraController::new(0.2),
        });
    }

    pub fn remove_window(&mut self, window: WindowId) {
        self.views.retain(|v| v.window != window);
    }

    /// WASD でカメラを動かす
    pub fn key(&mut self, window: WindowId, code: KeyCode, pressed: bool) {
        if let Some(view) = self.views.iter_mut().find(|v| v.window == window) {
            view.camera_controller.handle_key(code, pressed);
        }
    }

    pub fn update(&mut self, gpu: &Gpu, window: WindowId, logical_size: [f32; 2]) {
        let Some(view) = self.views.iter_mut().find(|v| v.window == window) else {
            return;
        };
        view.camera.aspect = logical_size[0] / logical_size[1].max(1.0);
        view.camera_controller.update_camera(&mut view.camera);
        view.camera_uniform.update_view_proj(&view.camera);
        gpu.queue.write_buffer(&view.camera_buffer, 0, bytemuck::cast_slice(&[view.camera_uniform]));
    }

    pub fn render(&self, window: WindowId, render_pass: &mut wgpu::RenderPass) {
        let Some(view) = self.views.iter().find(|v| v.window == window) else {
            return;
        };
        render_pass.set_pipeline(&self.render_pipeline);
        render_pass.set_bind_group(0, &self.diffuse_bind_group, &[]);
        render_pass.set_bind_group(1, &view.camera_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.model_buffer.vertex_buffer.slice(..));
        render_pass.set_vertex_buffer(1, self.model_instance_buffer.slice(..));
        render_pass.set_index_buffer(self.model_buffer.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.draw_indexed(0..self.model_buffer.num_indices, 0, 0..self.model_instances.len() as u32);
    }

    /// シェーダとモデルを読み直す。失敗したら前のまま
    pub fn reload(&mut self, gpu: &Gpu) {
        if let Err(e) = self.shader.reload(&gpu.device) {
            log::error!("Failed to reload shader: {:?}", e);
        }
        match Self::construct_render_pipeline(&gpu.device, &self.shader, gpu.surface_format) {
            Ok(pipeline) => self.render_pipeline = pipeline,
            Err(e) => log::error!("Failed to reload render pipeline: {:?}", e),
        }
        match Self::load_model_and_create_buffer(&gpu.device) {
            Ok(model_buffer) => self.model_buffer = model_buffer,
            Err(e) => log::error!("Failed to reload model: {:?}", e),
        }
    }

    fn load_model_and_create_buffer(device: &wgpu::Device) -> anyhow::Result<vertex::ModelBuffer> {
        let model = vertex::Model::load_sexpr_file(MODEL_PATH)?;
        Ok(model.create_buffer(device))
    }

    fn construct_render_pipeline(device: &wgpu::Device, shader_wrapper: &ShaderWrapper, surface_format: wgpu::TextureFormat) -> anyhow::Result<wgpu::RenderPipeline> {
        let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Render Pipeline Layout"),
            bind_group_layouts: &shader_wrapper.bind_group_layouts.iter().collect::<Vec<_>>(),
            immediate_size: 0,
        });
        engine::shader::capture_validation(device, || device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader_wrapper.nature,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::LAYOUT, ModelInstanceRaw::desc()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader_wrapper.nature,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: texture::TextureWrapper::DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: None,
        }))
    }
}

struct CameraController {
    speed: f32,
    is_forward_pressed: bool,
    is_backward_pressed: bool,
    is_left_pressed: bool,
    is_right_pressed: bool,
}

impl CameraController {
    fn new(speed: f32) -> Self {
        Self {
            speed,
            is_forward_pressed: false,
            is_backward_pressed: false,
            is_left_pressed: false,
            is_right_pressed: false,
        }
    }

    fn handle_key(&mut self, code: KeyCode, is_pressed: bool) -> bool {
        match code {
            KeyCode::KeyW => {
                self.is_forward_pressed = is_pressed;
                true
            }
            KeyCode::KeyA => {
                self.is_left_pressed = is_pressed;
                true
            }
            KeyCode::KeyS => {
                self.is_backward_pressed = is_pressed;
                true
            }
            KeyCode::KeyD => {
                self.is_right_pressed = is_pressed;
                true
            }
            _ => false,
        }
    }

    fn update_camera(&self, camera: &mut camera::Camera) {
        let forward = camera.target - camera.eye;
        let forward_norm = forward.normalize();
        let forward_mag = forward.magnitude();

        // Prevents glitching when the camera gets too close to the
        // center of the scene.
        if self.is_forward_pressed && forward_mag > self.speed {
            camera.eye += forward_norm * self.speed;
        }
        if self.is_backward_pressed {
            camera.eye -= forward_norm * self.speed;
        }

        let right = forward_norm.cross(camera.up);

        // Redo radius calc in case the forward/backward is pressed.
        let forward = camera.target - camera.eye;
        let forward_mag = forward.magnitude();

        if self.is_right_pressed {
            // Rescale the distance between the target and the eye so
            // that it doesn't change. The eye, therefore, still
            // lies on the circle made by the target and eye.
            camera.eye = camera.target - (forward + right * self.speed).normalize() * forward_mag;
        }
        if self.is_left_pressed {
            camera.eye = camera.target - (forward - right * self.speed).normalize() * forward_mag;
        }
    }
}
