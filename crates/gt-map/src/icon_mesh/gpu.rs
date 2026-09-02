//! GPU-instanced icon rendering.
//!
//! [install] uploads every icon template (all size buckets) into one static
//! vertex/index buffer pair and builds a render pipeline whose color path
//! mirrors egui's own (`egui.wgsl`): vertex colors in gamma space,
//! premultiplied-alpha blending, gamma or linear framebuffer output chosen
//! by the target format, and the same interleaved-gradient dither.
//! Per frame, [IconMeshBatch](super::IconMeshBatch) segments above the
//! [GPU_MIN_INSTANCES] threshold are painted as one
//! [egui_wgpu::Callback] that issues one instanced draw per (icon, bucket)
//! group, uploading 32 bytes per instance instead of kilobytes of
//! transformed vertices.

use std::num::NonZeroU64;
use std::sync::OnceLock;

use egui::epaint::PaintCallbackInfo;
use egui_wgpu::{CallbackResources, CallbackTrait, RenderState, ScreenDescriptor, wgpu};
use gt_icon_tessellate::TemplateVertex;
use rustc_hash::FxHashMap;
use wgpu::util::DeviceExt as _;

use crate::icon_mesh::{IconId, IconMeshLibrary};

/// Minimum instances in a flush segment for the GPU path. Smaller segments go
/// through the CPU mesh path, so barrier-heavy zoomed-in frames do not create a
/// stream of tiny buffers and draw calls.
pub const GPU_MIN_INSTANCES: usize = 32;

/// Marker in the egui context data store: GPU icon resources are installed
/// in this context's renderer.
fn installed_flag() -> egui::Id {
    egui::Id::new("gt_icon_gpu_installed")
}

pub(crate) fn is_installed(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(installed_flag()))
        .unwrap_or(false)
}

/// One instance, as laid out in the wgpu instance buffer (32 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct GpuIconInstance {
    pub center: [f32; 2],
    pub col_x: [f32; 2],
    pub col_y: [f32; 2],
    /// Premultiplied sRGB tints, packed like egui vertex colors
    /// (r in the low byte), one per template tint slot.
    pub tints: [u32; 2],
}

/// A template vertex as laid out in the static GPU vertex buffer (16 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuTemplateVertex {
    pos: [f32; 2],
    /// Premultiplied sRGB, packed like egui vertex colors.
    color: u32,
    tint_slot: u32,
}

/// Uniforms mirroring egui's `Locals`: the screen size in points plus the
/// dithering toggle, so icon pixels match what epaint meshes produce.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Locals {
    screen_size_in_points: [f32; 2],
    dithering: u32,
    _padding: u32,
}

/// Where one (icon, bucket) template lives in the static buffers.
#[derive(Debug, Clone, Copy)]
struct TemplateRange {
    first_index: u32,
    index_count: u32,
    base_vertex: i32,
}

/// The per-renderer GPU state, stored in egui_wgpu's [CallbackResources].
struct IconGpuResources {
    pipeline: wgpu::RenderPipeline,
    locals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    template_vertices: wgpu::Buffer,
    template_indices: wgpu::Buffer,
    ranges: FxHashMap<(IconId, usize), TemplateRange>,
    dithering: bool,
}

fn pack_color(color: [u8; 4]) -> u32 {
    u32::from_le_bytes(color)
}

pub(crate) fn pack_color32(color: egui::Color32) -> u32 {
    pack_color(color.to_array())
}

/// Build the static template buffers and pipeline, and register them with
/// the renderer behind `render_state`.
///
/// Call once at startup (eframe's `CreationContext::wgpu_render_state`) or
/// harness setup. Idempotent per context.
///
/// `dithering` must match the egui renderer's own setting (not readable
/// from [RenderState]): eframe defaults to on, kittest's PREDICTABLE
/// options turn it off. Mismatch costs only a sub-LSB quantization
/// difference against epaint-rendered pixels.
pub fn install(
    egui_ctx: &egui::Context,
    render_state: &RenderState,
    library: &IconMeshLibrary,
    dithering: bool,
) {
    let device = &render_state.device;

    let mut vertices: Vec<GpuTemplateVertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut ranges = FxHashMap::default();
    for icon in <IconId as strum::IntoEnumIterator>::iter() {
        let tessellation = library.tessellation(icon);
        for (bucket, mesh) in tessellation.buckets().iter().enumerate() {
            let base_vertex = vertices.len() as i32;
            let first_index = indices.len() as u32;
            vertices.extend(mesh.mesh.vertices.iter().map(|vertex| {
                let &TemplateVertex {
                    pos,
                    color,
                    tint_slot,
                } = vertex;
                GpuTemplateVertex {
                    pos,
                    color: pack_color(color),
                    tint_slot: u32::from(tint_slot),
                }
            }));
            indices.extend_from_slice(&mesh.mesh.indices);
            ranges.insert(
                (icon, bucket),
                TemplateRange {
                    first_index,
                    index_count: mesh.mesh.indices.len() as u32,
                    base_vertex,
                },
            );
        }
    }

    let template_vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("gt_icon_template_vertices"),
        contents: bytemuck::cast_slice(&vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let template_indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("gt_icon_template_indices"),
        contents: bytemuck::cast_slice(&indices),
        usage: wgpu::BufferUsages::INDEX,
    });

    let locals = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gt_icon_locals"),
        size: std::mem::size_of::<Locals>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("gt_icon_bind_group_layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(std::mem::size_of::<Locals>() as u64),
            },
            count: None,
        }],
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("gt_icon_bind_group"),
        layout: &bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: locals.as_entire_binding(),
        }],
    });

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("gt_icon_instanced"),
        source: wgpu::ShaderSource::Wgsl(include_str!("icon_instanced.wgsl").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("gt_icon_pipeline_layout"),
        bind_group_layouts: &[Some(&bind_group_layout)],
        immediate_size: 0,
    });
    let target_format = render_state.target_format;
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("gt_icon_pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[
                Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuTemplateVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2,
                        1 => Unorm8x4,
                        2 => Uint32,
                    ],
                }),
                Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuIconInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        3 => Float32x2,
                        4 => Float32x2,
                        5 => Float32x2,
                        6 => Unorm8x4,
                        7 => Unorm8x4,
                    ],
                }),
            ],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            // Mirror egui's framebuffer handling: sRGB-aware targets get the
            // linear-output entry, everything else stays in gamma space.
            entry_point: Some(if target_format.is_srgb() {
                "fs_main_linear_framebuffer"
            } else {
                "fs_main_gamma_framebuffer"
            }),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                // egui's premultiplied-alpha blend state, verbatim.
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview_mask: None,
        cache: None,
    });

    render_state
        .renderer
        .write()
        .callback_resources
        .insert(IconGpuResources {
            pipeline,
            locals,
            bind_group,
            template_vertices,
            template_indices,
            ranges,
            dithering,
        });
    egui_ctx.data_mut(|d| d.insert_temp(installed_flag(), true));
}

/// [install], with the icon library embedded in the binary and dithering off,
/// as a hook a snapshot harness hands its render state to: kittest's
/// PREDICTABLE renderer options turn dithering off.
///
/// A blob that does not decode installs nothing and leaves the CPU mesh path
/// in place. [NavMap::new](crate::NavMap::new) logs that decode failure.
pub fn install_embedded_library_without_dithering(
    egui_ctx: &egui::Context,
    render_state: &RenderState,
) {
    if let Ok(library) = IconMeshLibrary::embedded() {
        install(egui_ctx, render_state, &library, false);
    }
}

/// One instanced-draw group inside a callback: every instance of one
/// (icon, bucket) template, in first-push order.
pub(crate) struct InstanceGroup {
    pub icon: IconId,
    pub bucket: usize,
    pub instances: Vec<GpuIconInstance>,
}

/// The paint callback for one flush segment.
///
/// Owns its instance data. The instance buffer is created in `prepare` and
/// kept on the callback itself, so no cross-frame bookkeeping lives in
/// [CallbackResources].
pub(crate) struct IconDrawCallback {
    pub groups: Vec<InstanceGroup>,
    instance_buffer: OnceLock<wgpu::Buffer>,
}

impl IconDrawCallback {
    pub fn new(groups: Vec<InstanceGroup>) -> Self {
        Self {
            groups,
            instance_buffer: OnceLock::new(),
        }
    }
}

impl CallbackTrait for IconDrawCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(resources) = callback_resources.get::<IconGpuResources>() else {
            return Vec::new();
        };
        let ppp = screen_descriptor.pixels_per_point;
        let [width_px, height_px] = screen_descriptor.size_in_pixels;
        let locals = Locals {
            screen_size_in_points: [width_px as f32 / ppp, height_px as f32 / ppp],
            dithering: u32::from(resources.dithering),
            _padding: 0,
        };
        queue.write_buffer(&resources.locals, 0, bytemuck::bytes_of(&locals));

        let instances: Vec<GpuIconInstance> = self
            .groups
            .iter()
            .flat_map(|group| group.instances.iter().copied())
            .collect();
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gt_icon_instances"),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::VERTEX,
        });
        self.instance_buffer.set(buffer).ok();
        Vec::new()
    }

    fn paint(
        &self,
        info: PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<IconGpuResources>() else {
            return;
        };
        let Some(instance_buffer) = self.instance_buffer.get() else {
            return;
        };

        // egui-wgpu sets the render-pass viewport to this callback's clip rect
        // before calling us, so its NDC would span only the map widget. Our
        // instances carry absolute screen-point positions and the shader maps
        // them against the full framebuffer (`screen_size_in_points`, written
        // in `prepare`), exactly like epaint's own mesh path. Reset the
        // viewport to the whole framebuffer so that mapping is correct; the
        // scissor rect egui-wgpu set to the clip rect stays in place, so the
        // icons are still clipped to the map widget. Without this, zoomed-out
        // frames (>= GPU_MIN_INSTANCES icons, so the instanced path) draw every
        // icon offset and scaled into a corner.
        let [width_px, height_px] = info.screen_size_px;
        render_pass.set_viewport(0.0, 0.0, width_px as f32, height_px as f32, 0.0, 1.0);

        render_pass.set_pipeline(&resources.pipeline);
        render_pass.set_bind_group(0, &resources.bind_group, &[]);
        render_pass.set_vertex_buffer(0, resources.template_vertices.slice(..));
        render_pass.set_vertex_buffer(1, instance_buffer.slice(..));
        render_pass.set_index_buffer(
            resources.template_indices.slice(..),
            wgpu::IndexFormat::Uint32,
        );

        let mut first_instance = 0u32;
        for group in &self.groups {
            let count = group.instances.len() as u32;
            let Some(range) = resources.ranges.get(&(group.icon, group.bucket)) else {
                first_instance += count;
                continue;
            };
            render_pass.draw_indexed(
                range.first_index..range.first_index + range.index_count,
                range.base_vertex,
                first_instance..first_instance + count,
            );
            first_instance += count;
        }
    }
}
