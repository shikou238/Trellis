use wgpu::util::DeviceExt;
use std::fs::File;
use std::io::{self, Read, Write};
use bytemuck::NoUninit; 

use serde::{Serialize, Deserialize};
use serde_lexpr::{from_reader, to_writer,from_str,to_string};

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable, Serialize, Deserialize)]
pub struct Vertex {
    pub position: [f32; 3],
    pub color: [f32; 3],
    pub texture_coords: [f32; 2],
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Model{
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u16>,
}
pub struct ModelBuffer{
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub num_indices: u32,
}
pub fn pentagon() -> Model { 
    Model {
        vertices: vec![
            Vertex { position: [-0.0868241, 0.49240386, 0.0], color: [0.5, 0.0, 0.5]  ,texture_coords: [0.4131759, 0.00759614], }, // A
            Vertex { position: [-0.49513406, 0.06958647, 0.0], color: [0.5, 0.0, 0.5] , texture_coords: [0.0048659444, 0.43041354],}, // B
            Vertex { position: [-0.21918549, -0.44939706, 0.0], color: [0.5, 0.0, 0.5] , texture_coords: [0.28081453, 0.949397], }, // C
            Vertex { position: [0.35966998, -0.3473291, 0.0], color: [0.5, 0.0, 0.5] , texture_coords: [0.85967, 0.84732914], }, // D
            Vertex { position: [0.44147372, 0.2347359, 0.0], color: [0.5, 0.0, 0.5] , texture_coords: [0.9414737, 0.2652641],}, // E
        ],
        indices: vec![
            0, 1, 4,
            1, 2, 4,
            2, 3, 4,
        ],
    }
}


impl Vertex{
    pub const LAYOUT : wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress, // 1.
        step_mode: wgpu::VertexStepMode::Vertex, // 2.
        attributes: &[ // 3.
                wgpu::VertexAttribute {
                    offset: 0, // 4.
                    shader_location: 0, // 5.
                    format: wgpu::VertexFormat::Float32x3, // 6.
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 6]>() as wgpu::BufferAddress,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x2,
                }
            ]
        };
}
impl Model {
    pub fn create_buffer(&self, device: &wgpu::Device) -> ModelBuffer {
        let vertex_buffer = device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("Vertex Buffer"),
                contents: bytemuck::cast_slice(&self.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }
        );
        let index_buffer = device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("Index Buffer"),
                contents: bytemuck::cast_slice(&self.indices),
                usage: wgpu::BufferUsages::INDEX,
            }
        );
        ModelBuffer {
            vertex_buffer,
            index_buffer,
            num_indices: self.indices.len() as u32,
        }
    }
    pub fn save_to_binaryfile(&self, path: &str) -> io::Result<()> {
        let mut file = File::create(path)?;

        // 頂点数とインデックス数を書き込む（ロード時に使う）
        let vertex_count = self.vertices.len() as u64;
        let index_count = self.indices.len() as u64;
        file.write_all(&vertex_count.to_le_bytes())?;
        file.write_all(&index_count.to_le_bytes())?;

        // 頂点データ本体
        let vertex_bytes =
            bytemuck::cast_slice::<Vertex, u8>(&self.vertices); // &[Vertex] -> &[u8][web:44][web:47]
        file.write_all(vertex_bytes)?;

        // インデックスデータ本体
        let index_bytes =
            bytemuck::cast_slice::<u16, u8>(&self.indices); // &[u16] -> &[u8][web:44][web:47]
        file.write_all(index_bytes)?;

        Ok(())
    }

    pub fn load_from_binary_file(path: &str) -> io::Result<Self> {
        use std::io::Read;

        let mut file = File::open(path)?;

        // 頂点数とインデックス数を読む
        let mut buf8 = [0u8; 8];

        file.read_exact(&mut buf8)?;
        let vertex_count = u64::from_le_bytes(buf8) as usize;

        file.read_exact(&mut buf8)?;
        let index_count = u64::from_le_bytes(buf8) as usize;

        // 頂点データを読む
        let vertex_bytes_len = vertex_count * std::mem::size_of::<Vertex>();
        let mut vertex_bytes = vec![0u8; vertex_bytes_len];
        file.read_exact(&mut vertex_bytes)?;

        // &[u8] -> &[Vertex] に解釈し直してコピー
        let vertex_slice: &[Vertex] =
            bytemuck::try_cast_slice(&vertex_bytes).expect("invalid vertex data");
        let vertices = vertex_slice.to_vec();

        // インデックスデータを読む
        let index_bytes_len = index_count * std::mem::size_of::<u16>();
        let mut index_bytes = vec![0u8; index_bytes_len];
        file.read_exact(&mut index_bytes)?;

        let index_slice: &[u16] =
            bytemuck::try_cast_slice(&index_bytes).expect("invalid index data");
        let indices = index_slice.to_vec();

        Ok(Model { vertices, indices })
    }
    pub fn save_sexpr_file(&self, path: &str) -> io::Result<()> {
        let s = to_string(self).expect("serialize to s-expression failed");
        let mut file = File::create(path)?;
        file.write_all(s.as_bytes())?;
        Ok(())
    }

    pub fn load_sexpr_file(path: &str) -> io::Result<Self> {
        let reset = false;
        if (reset) {
            let model = pentagon();
            model.save_sexpr_file(path)?;
        }


        let s = crate::assets::read_to_string(path).map_err(io::Error::other)?;
        let model: Model = from_str(&s).expect("parse s-expression failed");
        Ok(model)
    }
}