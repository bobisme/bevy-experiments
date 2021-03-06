struct View {
    view_proj: mat4x4<f32>;
};

struct Mesh {
    transform: mat4x4<f32>;
};

struct Vertex {
    [[location(0)]] position: vec3<f32>;
    [[location(1)]] color: vec4<f32>;
};

struct VertexOutput {
    [[builtin(position)]] clip_position: vec4<f32>;
    [[location(0)]] color: vec4<f32>;
};

[[group(0), binding(0)]]
var<uniform> view: View;

[[group(1), binding(0)]]
var<uniform> mesh: Mesh;

[[stage(vertex)]]
fn vertex(in: Vertex) -> VertexOutput {
    let world_position = mesh.transform * vec4<f32>(in.position, 1.0);

    var out: VertexOutput;
    out.color = in.color;
    out.clip_position = view.view_proj * world_position;
    return out;
}

struct FragmentInput {
    [[builtin(front_facing)]] is_front: bool;
    [[location(0)]] color: vec4<f32>;
};

[[stage(fragment)]]
fn fragment(in: FragmentInput) -> [[location(0)]] vec4<f32> {
    return in.color;
}