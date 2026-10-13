use wgpu::wgc::device;



pub struct ShaderWrapper {
    pub nature: wgpu::ShaderModule,
    path: String,
    pub bind_group_layouts: Vec<wgpu::BindGroupLayout>,
}

impl ShaderWrapper{
    pub fn new(device: &wgpu::Device) -> anyhow::Result<Self> {
        let path = "src/shader.wgsl";
        let shader = Self::load(device, path)?;

        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        // This should match the filterable field of the
                        // corresponding Texture entry above.
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
                label: Some("texture_bind_group_layout"),
            });
        
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

        
        Ok (Self {
            nature: shader,
            path: path.to_string(),
            bind_group_layouts: vec![texture_bind_group_layout,camera_bind_group_layout]
        })
    }
    fn load(device: &wgpu::Device,path: &str) -> anyhow::Result<wgpu::ShaderModule> {
        let s = crate::assets::read_to_string(path)?;
        capture_validation(device, || {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Shader"),
                source: wgpu::ShaderSource::Wgsl(s.into()),
            })
        })
    }
    pub fn reload(&mut self,device: &wgpu::Device) -> anyhow::Result<()> {
        match Self::load(device, &self.path) {
            Ok(new_shader) => {
                self.nature = new_shader;
                Ok(())
            },
            Err(e) => {
                Err(e)
            }
        }
    }
    pub fn get_texture_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.bind_group_layouts[0]
    }
}

/// `f` の中で起きた wgpu の検証エラー（WGSL のコンパイルエラーなど）を、パニックではなく Err で返す。
/// ホットリロードで壊れたシェーダを読んでも落ちないようにするため
pub fn capture_validation<T>(device: &wgpu::Device, f: impl FnOnce() -> T) -> anyhow::Result<T> {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let value = f();
    match pollster::block_on(scope.pop()) {
        None => Ok(value),
        Some(error) => Err(anyhow::anyhow!("{}", error)),
    }
}