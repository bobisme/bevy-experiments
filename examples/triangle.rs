use bevy::prelude::*;
use itertools::Itertools;

#[derive(Clone, Component)]
pub struct Triangle {
    a: Vec2,
    b: Vec2,
    c: Vec2,
    rgba: [f32; 4],
}

#[derive(Component)]
pub struct TriangleMeshHandle(pub Handle<Mesh>);

impl TriangleMeshHandle {
    pub fn clone_weak(&self) -> Self {
        Self(self.0.clone_weak())
    }
}

impl Triangle {
    pub fn side(len: f32) -> Self {
        let height = (len.powi(2) - (len / 2.0).powi(2)).sqrt();
        Self {
            a: Vec2::new(0.0, height / 2.0),
            b: Vec2::new(-len / 2.0, -height / 2.0),
            c: Vec2::new(len / 2.0, -height / 2.0),
            rgba: [0.5, 0.5, 0.5, 0.5],
        }
    }

    pub fn with_rgba(self, rgba: [f32; 4]) -> Self {
        Self { rgba, ..self }
    }
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .insert_resource(Msaa { samples: 4 })
        .insert_resource(ClearColor(Color::rgb(0.9, 0.9, 0.9)))
        .add_plugin(render::plugin::TriangleRenderPlugin)
        .add_startup_system(setup)
        .add_system(triangle_mesh_system)
        .run();
}

fn setup(mut commands: Commands) {
    commands.spawn_bundle(OrthographicCameraBundle::new_2d());
    commands.spawn_bundle((
        Triangle::side(500.0).with_rgba([1.0, 0.0, 0.0, 0.9]),
        Transform::default(),
        GlobalTransform::default(),
        Visibility::default(),
        ComputedVisibility::default(),
        Name::new("Triangle"),
    ));
}

fn triangle_mesh_system(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    triangle_q: Query<(Entity, &Triangle), Without<TriangleMeshHandle>>,
) {
    for (entity, triangle) in triangle_q.iter() {
        let mut mesh = Mesh::new(wgpu::PrimitiveTopology::TriangleList);
        mesh.set_attribute(
            Mesh::ATTRIBUTE_POSITION,
            [triangle.a, triangle.b, triangle.c]
                .into_iter()
                .map(|p| [p.x, p.y, 0.0])
                .collect_vec(),
        );
        mesh.set_attribute(
            Mesh::ATTRIBUTE_COLOR,
            std::iter::repeat(triangle.rgba).take(3).collect_vec(),
        );
        mesh.set_attribute(
            Mesh::ATTRIBUTE_UV_0,
            std::iter::repeat([0.0, 1.0]).take(3).collect_vec(),
        );
        let handle = meshes.add(mesh);
        commands.entity(entity).insert(TriangleMeshHandle(handle));
    }
}

pub mod render {
    use bevy::prelude::*;
    use bevy::render::render_resource::std140::AsStd140;

    #[derive(Clone, Component, AsStd140)]
    pub struct TriangleUniform {
        pub transform: Mat4,
    }

    pub mod system {
        use bevy::{
            core::FloatOrd,
            core_pipeline::Transparent2d,
            prelude::*,
            render::{
                render_component::ComponentUniforms,
                render_phase::{DrawFunctions, RenderPhase},
                render_resource::{RenderPipelineCache, SpecializedPipelines},
                renderer::RenderDevice,
                view::{ExtractedView, ViewUniforms, VisibleEntities},
            },
        };
        use itertools::Itertools;

        use super::*;
        use crate::TriangleMeshHandle;
        use pipeline::TrianglePipeline;

        pub fn extract_triangle_meshes(
            mut commands: Commands,
            triangle_mesh_q: Query<(
                Entity,
                &TriangleMeshHandle,
                &GlobalTransform,
                &ComputedVisibility,
            )>,
        ) {
            let components = triangle_mesh_q
                .iter()
                .filter_map(
                    |(entity, triangle_mesh_handle, tform, vis)| match vis.is_visible {
                        true => Some((entity, triangle_mesh_handle, tform)),
                        false => None,
                    },
                )
                .map(|(entity, triangle_mesh_handle, tform)| {
                    let uniform = TriangleUniform {
                        transform: tform.compute_matrix(),
                    };
                    (entity, (triangle_mesh_handle.clone_weak(), uniform))
                })
                .collect_vec();
            commands.insert_or_spawn_batch(components);
        }

        pub fn queue_view_bind_groups(
            mut commands: Commands,
            device: Res<RenderDevice>,
            pipeline: Res<TrianglePipeline>,
            view_uniforms: Res<ViewUniforms>,
            views: Query<Entity, With<ExtractedView>>,
        ) {
            let view_binding = match view_uniforms.uniforms.binding() {
                Some(binding) => binding,
                None => return,
            };
            for entity in views.iter() {
                let view_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: view_binding.clone(),
                    }],
                    label: Some("texture view bind group"),
                    layout: &pipeline.view_layout,
                });

                commands
                    .entity(entity)
                    .insert(draw::ViewBindGroup(view_bind_group));
            }
        }

        pub fn queue_mesh_bind_groups(
            mut commands: Commands,
            pipeline: Res<TrianglePipeline>,
            render_device: Res<RenderDevice>,
            mesh_uniforms: Res<ComponentUniforms<TriangleUniform>>,
        ) {
            let binding = match mesh_uniforms.uniforms().binding() {
                Some(binding) => binding,
                None => return,
            };
            commands.insert_resource(draw::MeshBindGroup(render_device.create_bind_group(
                &wgpu::BindGroupDescriptor {
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: binding,
                    }],
                    label: Some("body_mesh_bind_group"),
                    layout: &pipeline.mesh_layout,
                },
            )));
        }

        pub fn queue_triangles(
            mut view_q: Query<(&VisibleEntities, &mut RenderPhase<Transparent2d>)>,
            draw_functions: Res<DrawFunctions<Transparent2d>>,
            msaa: Res<Msaa>,
            mut pipelines: ResMut<SpecializedPipelines<TrianglePipeline>>,
            mut pipeline_cache: ResMut<RenderPipelineCache>,
            pipeline: Res<TrianglePipeline>,
            mesh_q: Query<(Entity, &TriangleUniform)>,
        ) {
            let draw_function = draw_functions
                .read()
                .get_id::<draw::DrawTriangle>()
                .unwrap();
            let key = pipeline::TrianglePipelineKey::from_msaa_samples(msaa.samples);
            let pipeline_id = pipelines.specialize(&mut pipeline_cache, &pipeline, key);
            view_q.iter_mut().for_each(|(visible, mut phase)| {
                for (entity, uniform) in mesh_q.iter() {
                    if !visible.entities.contains(&entity) {
                        continue;
                    }
                    let mesh_z = uniform.transform.w_axis.z;
                    phase.add(Transparent2d {
                        entity,
                        draw_function,
                        pipeline: pipeline_id,
                        sort_key: FloatOrd(mesh_z),
                        batch_range: None,
                    });
                }
            });
        }
    }

    pub mod pipeline {
        use bevy::prelude::*;
        use bevy::render::render_resource::std140::AsStd140;
        use bevy::render::render_resource::{
            BindGroupLayout, FragmentState, RenderPipelineDescriptor, SpecializedPipeline,
            VertexBufferLayout, VertexState,
        };
        use bevy::render::renderer::RenderDevice;
        use bevy::render::texture::BevyDefault;
        use bevy::render::view::ViewUniform;

        use super::*;
        use plugin::SHADER_HANDLE;

        #[derive(Clone)]
        pub struct TrianglePipeline {
            pub view_layout: BindGroupLayout,
            pub mesh_layout: BindGroupLayout,
        }

        impl FromWorld for TrianglePipeline {
            fn from_world(world: &mut World) -> Self {
                let device = world.get_resource::<RenderDevice>().unwrap();
                let view_layout =
                    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        entries: &[
                            // View
                            wgpu::BindGroupLayoutEntry {
                                binding: 0,
                                visibility: wgpu::ShaderStages::VERTEX
                                    | wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Buffer {
                                    ty: wgpu::BufferBindingType::Uniform,
                                    has_dynamic_offset: true,
                                    min_binding_size: wgpu::BufferSize::new(
                                        ViewUniform::std140_size_static() as u64,
                                    ),
                                },
                                count: None,
                            },
                        ],
                        label: Some("triangle view layout"),
                    });

                let mesh_layout =
                    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        entries: &[wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: true,
                                min_binding_size: wgpu::BufferSize::new(
                                    TriangleUniform::std140_size_static() as u64,
                                ),
                            },
                            count: None,
                        }],
                        label: Some("triangle mesh layout"),
                    });
                Self {
                    view_layout,
                    mesh_layout,
                }
            }
        }

        bitflags::bitflags! {
            #[repr(transparent)]
            // See bevy_sprite::render::SpritePipelineKey

            pub struct TrianglePipelineKey: u32 {
                const NONE               = 0;
                const COLORED            = (1 << 0);
                const MSAA_RESERVED_BITS = TrianglePipelineKey::MSAA_MASK_BITS << TrianglePipelineKey::MSAA_SHIFT_BITS;
            }
        }

        impl TrianglePipelineKey {
            const MSAA_MASK_BITS: u32 = 0b111111;
            const MSAA_SHIFT_BITS: u32 = 32 - 6;

            pub fn from_msaa_samples(msaa_samples: u32) -> Self {
                let msaa_bits =
                    ((msaa_samples - 1) & Self::MSAA_MASK_BITS) << Self::MSAA_SHIFT_BITS;
                TrianglePipelineKey::from_bits(msaa_bits).unwrap()
            }

            pub fn msaa_samples(&self) -> u32 {
                ((self.bits >> Self::MSAA_SHIFT_BITS) & Self::MSAA_MASK_BITS) + 1
            }
        }

        impl SpecializedPipeline for TrianglePipeline {
            type Key = TrianglePipelineKey;

            fn specialize(&self, key: Self::Key) -> RenderPipelineDescriptor {
                let vertex_attributes = [
                    // position
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 16,
                        shader_location: 0,
                    },
                    // color
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x4,
                        offset: 0,
                        shader_location: 1,
                    },
                    // uv
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 12 + 16,
                        shader_location: 2,
                    },
                ];
                RenderPipelineDescriptor {
                    vertex: VertexState {
                        shader: SHADER_HANDLE.typed::<Shader>(),
                        entry_point: "vertex".into(),
                        shader_defs: vec![],
                        buffers: vec![VertexBufferLayout {
                            array_stride: vertex_attributes.iter().map(|x| x.format.size()).sum(),
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: vertex_attributes.to_vec(),
                        }],
                    },
                    fragment: Some(FragmentState {
                        shader: SHADER_HANDLE.typed::<Shader>(),
                        shader_defs: vec![],
                        entry_point: "fragment".into(),
                        targets: vec![wgpu::ColorTargetState {
                            format: wgpu::TextureFormat::bevy_default(),
                            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                            write_mask: wgpu::ColorWrites::ALL,
                        }],
                    }),
                    layout: Some(vec![self.view_layout.clone(), self.mesh_layout.clone()]),
                    primitive: wgpu::PrimitiveState {
                        front_face: wgpu::FrontFace::Ccw,
                        cull_mode: Some(wgpu::Face::Back),
                        unclipped_depth: false,
                        polygon_mode: wgpu::PolygonMode::Fill,
                        conservative: false,
                        topology: wgpu::PrimitiveTopology::TriangleList,
                        strip_index_format: None,
                    },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState {
                        count: key.msaa_samples(),
                        mask: !0,
                        alpha_to_coverage_enabled: false,
                    },
                    label: Some("triangle pipeline".into()),
                }
            }
        }
    }

    pub mod plugin {
        use bevy::core_pipeline::Transparent2d;
        use bevy::prelude::*;
        use bevy::reflect::TypeUuid;
        use bevy::render::render_component::UniformComponentPlugin;
        use bevy::render::render_phase::AddRenderCommand;
        use bevy::render::render_resource::SpecializedPipelines;
        use bevy::render::{RenderApp, RenderStage};

        use super::pipeline::TrianglePipeline;
        use super::*;

        pub const SHADER_HANDLE: HandleUntyped =
            HandleUntyped::weak_from_u64(Shader::TYPE_UUID, 0xc648c90f09f1fe7d);

        #[derive(Default)]
        pub struct TriangleRenderPlugin;

        impl Plugin for TriangleRenderPlugin {
            fn build(&self, app: &mut App) {
                let mut shaders = app.world.get_resource_mut::<Assets<Shader>>().unwrap();
                shaders.set_untracked(
                    SHADER_HANDLE,
                    Shader::from_wgsl(include_str!("triangle.wgsl")),
                );
                app.add_plugin(UniformComponentPlugin::<TriangleUniform>::default());
                let render_app = app.get_sub_app_mut(RenderApp).unwrap();
                render_app
                    .add_render_command::<Transparent2d, draw::DrawTriangle>()
                    .init_resource::<TrianglePipeline>()
                    .init_resource::<SpecializedPipelines<TrianglePipeline>>()
                    .add_system_to_stage(RenderStage::Extract, system::extract_triangle_meshes)
                    .add_system_to_stage(RenderStage::Queue, system::queue_view_bind_groups)
                    .add_system_to_stage(RenderStage::Queue, system::queue_mesh_bind_groups)
                    .add_system_to_stage(RenderStage::Queue, system::queue_triangles);
            }
        }
    }

    pub mod draw {
        use bevy::ecs::system::{lifetimeless::*, SystemParamItem};
        use bevy::prelude::*;
        use bevy::render::mesh::GpuBufferInfo;
        use bevy::render::render_asset::RenderAssets;
        use bevy::render::render_component::DynamicUniformIndex;
        use bevy::render::render_phase::{
            EntityRenderCommand, RenderCommandResult, SetItemPipeline, TrackedRenderPass,
        };
        use bevy::render::render_resource::BindGroup;
        use bevy::render::view::ViewUniformOffset;

        use crate::TriangleMeshHandle;

        use super::TriangleUniform;

        pub type DrawTriangle = (
            SetItemPipeline,
            SetViewBindGroup<0>,
            SetMeshBindGroup<1>,
            DrawTriangleMesh,
        );

        #[derive(Clone, Debug, Component)]
        pub struct ViewBindGroup(pub BindGroup);

        #[derive(Clone, Debug, Component)]
        pub struct MeshBindGroup(pub BindGroup);

        pub struct SetViewBindGroup<const I: usize>;
        impl<const I: usize> EntityRenderCommand for SetViewBindGroup<I> {
            type Param = SQuery<(Read<ViewUniformOffset>, Read<ViewBindGroup>)>;
            #[inline]
            fn render<'w>(
                view: Entity,
                _item: Entity,
                view_query: SystemParamItem<'w, '_, Self::Param>,
                pass: &mut TrackedRenderPass<'w>,
            ) -> RenderCommandResult {
                let (view_uniform, view_bind_group) = view_query.get(view).unwrap();
                pass.set_bind_group(I, &view_bind_group.0, &[view_uniform.offset]);

                RenderCommandResult::Success
            }
        }

        pub struct SetMeshBindGroup<const I: usize>;
        impl<const I: usize> EntityRenderCommand for SetMeshBindGroup<I> {
            type Param = (
                SRes<MeshBindGroup>,
                SQuery<Read<DynamicUniformIndex<TriangleUniform>>>,
            );
            #[inline]
            fn render<'w>(
                _view: Entity,
                item: Entity,
                (mesh_bind_group, mesh_query): SystemParamItem<'w, '_, Self::Param>,
                pass: &mut TrackedRenderPass<'w>,
            ) -> RenderCommandResult {
                let mesh_index = mesh_query.get(item).unwrap();
                pass.set_bind_group(I, &mesh_bind_group.into_inner().0, &[mesh_index.index()]);
                RenderCommandResult::Success
            }
        }

        pub struct DrawTriangleMesh;
        impl EntityRenderCommand for DrawTriangleMesh {
            type Param = (SRes<RenderAssets<Mesh>>, SQuery<Read<TriangleMeshHandle>>);
            #[inline]
            fn render<'w>(
                _view: Entity,
                item: Entity,
                (meshes, mesh_query): SystemParamItem<'w, '_, Self::Param>,
                pass: &mut TrackedRenderPass<'w>,
            ) -> RenderCommandResult {
                let mesh_handle = &mesh_query.get(item).unwrap().0;
                let gpu_mesh = match meshes.into_inner().get(mesh_handle) {
                    Some(x) => x,
                    None => {
                        error!("couldn't get mesh");
                        return RenderCommandResult::Failure;
                    }
                };
                pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
                match &gpu_mesh.buffer_info {
                    GpuBufferInfo::Indexed {
                        buffer,
                        index_format,
                        count,
                    } => {
                        pass.set_index_buffer(buffer.slice(..), 0, *index_format);
                        pass.draw_indexed(0..*count, 0, 0..1);
                    }
                    GpuBufferInfo::NonIndexed { vertex_count } => {
                        pass.draw(0..*vertex_count, 0..1);
                    }
                }
                RenderCommandResult::Success
            }
        }
    }
}
