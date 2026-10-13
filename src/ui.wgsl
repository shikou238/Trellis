// UI 用 2D 矩形シェーダ。座標は論理ピクセル（左上原点・y 下向き）

struct Screen {
    size: vec2<f32>,
    _padding: vec2<f32>,
};
@group(0) @binding(0)
var<uniform> screen: Screen;

struct RectInput {
    @location(0) position: vec2<f32>, // 左上
    @location(1) size: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) rotation: f32,       // ラジアン、中心まわり
};

struct FragmentInput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32, rect: RectInput) -> FragmentInput {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 1.0),
    );
    let local = (corners[vertex_index] - vec2<f32>(0.5, 0.5)) * rect.size;
    let s = sin(rect.rotation);
    let c = cos(rect.rotation);
    let rotated = vec2<f32>(local.x * c - local.y * s, local.x * s + local.y * c);
    let pixel = rect.position + rect.size * 0.5 + rotated;

    var out: FragmentInput;
    out.clip_position = vec4<f32>(
        pixel.x / screen.size.x * 2.0 - 1.0,
        1.0 - pixel.y / screen.size.y * 2.0,
        0.0,
        1.0,
    );
    out.color = rect.color;
    return out;
}

@fragment
fn fs_main(in: FragmentInput) -> @location(0) vec4<f32> {
    return in.color;
}
