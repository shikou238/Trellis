use std::sync::Arc;

use wgpu::wgc::device;
use winit::{
    application::ApplicationHandler, event::*, event_loop::{ActiveEventLoop, EventLoop}, keyboard::{KeyCode, PhysicalKey}, window::Window
};

use crate::hsv_to_rgb;
use crate::vertex;
use crate::camera;

use wgpu::util::DeviceExt;
use crate::texture::TextureWrapper;
use crate::input::InputMapper;
use crate::ui::ScenePlayer;
use crate::ui::renderer::{UiLayer, UiRenderer};
pub struct WindowWrapper{
    pub nature: Arc<Window>,
    // todo why lifetime is needed here?
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    pub is_surface_configured: bool,
    pub last_cursor_position: Option<winit::dpi::PhysicalPosition<f64>>,
    pub camera: camera::Camera,
    pub camera_buffer: wgpu::Buffer,
    pub camera_uniform: camera::CameraUniform,
    pub camera_bind_group: wgpu::BindGroup,
    pub camera_controller: CameraController,
    pub depth_texture: TextureWrapper,
    pub scene: ScenePlayer,
    pub ui_layer: UiLayer,
    input: InputMapper,
    /// シーンが指定した背景色（線形 RGBA）。None ならカーソル位置の色
    background: Option<[f32; 4]>,
}

impl WindowWrapper {
    pub fn new(window: Arc<Window>, instance: &wgpu::Instance,device: &wgpu::Device, adapter: &wgpu::Adapter,surface_format: wgpu::TextureFormat, ui_renderer: &UiRenderer, scene_path: &str) -> Self {
        let size = window.inner_size();

        let surface = instance.create_surface(window.clone()).unwrap();

        let surface_caps = surface.get_capabilities(&adapter);
        
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        let camera = camera::Camera {
            // position the camera 1 unit up and 2 units back
            // +z is out of the screen
            eye: (0.0, 1.0, 2.0).into(),
            // have it look at the origin
            target: (0.0, 0.0, 0.0).into(),
            // which way is "up"
            up: cgmath::Vector3::unit_y(),
            aspect: size.width as f32 / size.height as f32,
            fovy: 45.0,
            znear: 0.1,
            zfar: 100.0,
        };

        // in new() after creating `camera`

        let mut camera_uniform = camera::CameraUniform::new();
        camera_uniform.update_view_proj(&camera);

        let camera_buffer = device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("Camera Buffer"),
                contents: bytemuck::cast_slice(&[camera_uniform]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            }
        );
        //todo このコードってカメラに書かれるべき？それともシェーダーに書かれるべき？
        let camera_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }
            ],
            label: Some("camera_bind_group_layout"),
        });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &camera_bind_group_layout,  
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                }
            ],
            label: Some("camera_bind_group"),
        });

        let depth_texture = TextureWrapper::create_depth_texture(device, &config, "depth_texture");

        Self {
            nature: window,
            surface,
            config,
            is_surface_configured: false,
            last_cursor_position: None,
            camera,
            camera_buffer,
            camera_uniform,
            camera_bind_group,
            camera_controller: CameraController::new(0.2),
            depth_texture,
            scene: ScenePlayer::new(scene_path),
            ui_layer: UiLayer::new(device, ui_renderer),
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
    // impl State
    pub fn handle_key(&mut self, _event_loop: &ActiveEventLoop, code: KeyCode, is_pressed: bool, repeat: bool) {
        self.camera_controller.handle_key(code, is_pressed);

        let mut inputs = Vec::new();
        self.input.key(code, is_pressed, repeat, &mut inputs);
        self.scene.queue_inputs(&inputs);
    }
    /// フォーカスを失うとキーの解放が届かないので、押されているものをすべて離す
    pub fn focus_lost(&mut self) {
        let mut inputs = Vec::new();
        self.input.release_all(&mut inputs);
        self.scene.queue_inputs(&inputs);
    }
    pub fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.camera_controller.update_camera(&mut self.camera);
        self.camera_uniform.update_view_proj(&self.camera);
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::cast_slice(&[self.camera_uniform]));

        let scale = self.nature.scale_factor() as f32;
        let logical_size = [self.config.width as f32 / scale, self.config.height as f32 / scale];
        self.scene.poll_file();
        self.scene.advance(logical_size);
        self.ui_layer.instances.clear();
        self.background = self.scene.build(logical_size, &mut self.ui_layer.instances);
        self.ui_layer.upload(device, queue, logical_size);
    }
    pub fn cursor_moved(&mut self, position: winit::dpi::PhysicalPosition<f64>) {
        self.last_cursor_position = Some(position);        
    }
    
    pub fn render(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, render_pipeline: &wgpu::RenderPipeline, model_buffer: &vertex::ModelBuffer, diffuse_bind_group: &wgpu::BindGroup,model_instance_buffer: &wgpu::Buffer,many: u32, ui_renderer: &UiRenderer) -> Result<(), wgpu::SurfaceError>{
        self.nature.request_redraw();

        // We can't render unless the surface is configured
        if !self.is_surface_configured {
            return Ok(());
        }
            
        let output = self.surface.get_current_texture()?;
        // We'll do more stuff here in the next tutorial

        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
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
            render_pass.set_pipeline(render_pipeline); // 2.
            render_pass.set_bind_group(0, diffuse_bind_group, &[]);
            render_pass.set_bind_group(1, &self.camera_bind_group, &[]);
            render_pass.set_vertex_buffer(0, model_buffer.vertex_buffer.slice(..)); 
            render_pass.set_vertex_buffer(1, model_instance_buffer.slice(..));
            render_pass.set_index_buffer(model_buffer.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(0..model_buffer.num_indices,0, 0..many); // 3.

            // UI は 3D の上に重ねる
            self.ui_layer.draw(&mut render_pass, ui_renderer);
        }

        // submit will accept anything that implements IntoIter
        queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
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
        log::debug!("code: {:?}, is_pressed: {:?}", code, is_pressed);
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
        use cgmath::InnerSpace;
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
