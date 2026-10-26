//! GPU drawing for the 3D views.
//!
//! Each 3D view renders into its own offscreen texture (4× multisampled,
//! with a depth buffer) and egui shows that texture as an image. Wires and
//! grid lines are drawn as screen-space thick lines; the pattern is a lit,
//! colour-mapped triangle mesh. Colours are kept gamma-encoded end to end,
//! which is what egui expects of a texture it draws.

use std::sync::Arc;

use eframe::egui;
use eframe::egui_wgpu::{self, wgpu};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

const SAMPLES: u32 = 4;
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

const SHADER: &str = r#"
struct Uniforms {
    view_proj: mat4x4<f32>,
    viewport: vec4<f32>,
    light_dir: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: Uniforms;

struct LineIn {
    @location(0) a: vec3<f32>,
    @location(1) width: f32,
    @location(2) b: vec3<f32>,
    @location(3) color: vec4<f32>,
};

struct ColourOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_line(@builtin(vertex_index) vi: u32, inst: LineIn) -> ColourOut {
    let ca = u.view_proj * vec4<f32>(inst.a, 1.0);
    let cb = u.view_proj * vec4<f32>(inst.b, 1.0);
    let half = u.viewport.xy * 0.5;
    let sa = ca.xy / ca.w * half;
    let sb = cb.xy / cb.w * half;
    var dir = sb - sa;
    let len = length(dir);
    if (len < 1e-4) {
        dir = vec2<f32>(1.0, 0.0);
    } else {
        dir = dir / len;
    }
    let normal = vec2<f32>(-dir.y, dir.x);
    // Two triangles: (a-, a+, b-) and (b-, a+, b+).
    var ends = array<f32, 6>(0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    var sides = array<f32, 6>(-1.0, 1.0, -1.0, -1.0, 1.0, 1.0);
    let t = ends[vi];
    let side = sides[vi];
    let c = mix(ca, cb, t);
    // Extend each end by half the width so joints between segments close.
    let along = (t * 2.0 - 1.0) * inst.width * 0.5;
    let offset = (normal * side * inst.width * 0.5 + dir * along) / half;
    var out: ColourOut;
    out.pos = vec4<f32>(c.xy + offset * c.w, c.z, c.w);
    out.color = inst.color;
    return out;
}

@fragment
fn fs_colour(in: ColourOut) -> @location(0) vec4<f32> {
    return in.color;
}

struct MeshIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
};

struct MeshOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_mesh(v: MeshIn) -> MeshOut {
    var out: MeshOut;
    out.pos = u.view_proj * vec4<f32>(v.pos, 1.0);
    out.normal = v.normal;
    out.color = v.color;
    return out;
}

@fragment
fn fs_mesh(in: MeshOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let l = normalize(u.light_dir.xyz);
    // Two-sided lighting, done in linear light, returned gamma-encoded.
    let shade = 0.5 + 0.5 * abs(dot(n, l));
    let linear = pow(in.color.rgb, vec3<f32>(2.2)) * shade;
    return vec4<f32>(pow(linear, vec3<f32>(1.0 / 2.2)), in.color.a);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    viewport: [f32; 4],
    light_dir: [f32; 4],
}

/// One thick line from `a` to `b`, `width` pixels wide.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LineInstance {
    pub a: [f32; 3],
    pub width: f32,
    pub b: [f32; 3],
    pub _pad: f32,
    pub color: [f32; 4],
}

impl LineInstance {
    pub fn new(a: Vec3, b: Vec3, width: f32, color: egui::Color32) -> Self {
        Self {
            a: a.to_array(),
            width,
            b: b.to_array(),
            _pad: 0.0,
            color: rgba(color),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MeshVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 4],
}

/// Gamma-encoded RGBA 0-1 from an egui colour (unmultiplied alpha).
pub fn rgba(c: egui::Color32) -> [f32; 4] {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    [r, g, b, a].map(|v| f32::from(v) / 255.0)
}

/// Geometry for one 3D view, on the CPU.
#[derive(Default, Clone)]
pub struct Scene {
    pub lines: Vec<LineInstance>,
    pub mesh: Vec<MeshVertex>,
    pub mesh_indices: Vec<u32>,
    /// Draw the mesh see-through (as an overlay on the antenna).
    pub mesh_translucent: bool,
}

/// A scene's geometry on the GPU.
pub struct SceneBuffers {
    lines: Option<(wgpu::Buffer, u32)>,
    mesh: Option<(wgpu::Buffer, wgpu::Buffer, u32)>,
    translucent: bool,
}

/// Orbit camera: Z is up, as in NEC.
#[derive(Clone, Copy, Debug)]
pub struct OrbitCamera {
    pub target: Vec3,
    pub distance: f32,
    /// Rotation about Z, radians (0 looks along +Y, from -Y).
    pub yaw: f32,
    /// Elevation above the XY plane, radians.
    pub pitch: f32,
    /// Size of the scene, for near/far planes.
    pub scene_radius: f32,
}

impl OrbitCamera {
    pub fn looking_at(centre: [f32; 3], radius: f32) -> Self {
        Self {
            target: Vec3::from(centre),
            distance: radius * 3.2,
            yaw: -0.6,
            pitch: 0.45,
            scene_radius: radius,
        }
    }

    pub fn eye(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        self.target + Vec3::new(sy * cp, -cy * cp, sp) * self.distance
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let near = (self.distance - self.scene_radius * 4.0).max(self.distance * 0.01);
        let far = self.distance + self.scene_radius * 4.0;
        // wgpu's clip space: depth 0..1, Y up (glam's "directx" convention).
        let proj = glam::camera::rh::proj::directx::perspective(
            35f32.to_radians(),
            aspect.max(1e-3),
            near,
            far,
        );
        let view = glam::camera::rh::view::look_at_mat4(self.eye(), self.target, Vec3::Z);
        proj * view
    }

    /// Mouse handling: drag to rotate, right/middle-drag to pan, scroll or
    /// pinch to zoom.
    pub fn handle_input(&mut self, ui: &egui::Ui, response: &egui::Response) -> bool {
        let mut changed = false;
        if response.dragged_by(egui::PointerButton::Primary) {
            let d = response.drag_delta();
            self.yaw -= d.x * 0.008;
            self.pitch = (self.pitch + d.y * 0.008).clamp(-1.55, 1.55);
            changed = true;
        }
        if response.dragged_by(egui::PointerButton::Secondary)
            || response.dragged_by(egui::PointerButton::Middle)
        {
            let d = response.drag_delta();
            let forward = (self.target - self.eye()).normalize();
            let right = forward.cross(Vec3::Z).normalize_or(Vec3::X);
            let up = right.cross(forward);
            let scale = self.distance * 0.0016;
            self.target += (-right * d.x + up * d.y) * scale;
            changed = true;
        }
        if response.hovered() {
            let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = (-scroll * 0.0015).exp() / zoom;
            if (factor - 1.0).abs() > 1e-4 {
                self.distance = (self.distance * factor)
                    .clamp(self.scene_radius * 0.05, self.scene_radius * 60.0);
                changed = true;
            }
        }
        changed
    }

    /// Ray from the eye through a point of the view, `ndc` in -1..1.
    pub fn ray(&self, ndc: egui::Vec2, aspect: f32) -> (Vec3, Vec3) {
        let inv = self.view_proj(aspect).inverse();
        let near = inv.project_point3(Vec3::new(ndc.x, ndc.y, 0.0));
        let far = inv.project_point3(Vec3::new(ndc.x, ndc.y, 1.0));
        (near, (far - near).normalize())
    }
}

/// Shared GPU objects: pipelines and the device.
pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Arc<egui::mutex::RwLock<egui_wgpu::Renderer>>,
    layout: wgpu::BindGroupLayout,
    line_pipeline: wgpu::RenderPipeline,
    mesh_pipeline: wgpu::RenderPipeline,
    mesh_blend_pipeline: wgpu::RenderPipeline,
}

impl Gpu {
    pub fn new(rs: &egui_wgpu::RenderState) -> Gpu {
        let device = rs.device.clone();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nec-3d"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nec-3d uniforms"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nec-3d"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let blend = Some(wgpu::BlendState::ALPHA_BLENDING);
        let depth = |write: bool| {
            Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(write),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            })
        };
        let line_attributes =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32, 2 => Float32x3, 3 => Float32x4];
        // Attribute 2 starts after the padding word.
        let line_attributes = [
            line_attributes[0],
            line_attributes[1],
            wgpu::VertexAttribute {
                offset: 16,
                ..line_attributes[2]
            },
            wgpu::VertexAttribute {
                offset: 32,
                ..line_attributes[3]
            },
        ];
        let line_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nec-3d lines"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_line"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<LineInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &line_attributes,
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: depth(true),
            multisample: wgpu::MultisampleState {
                count: SAMPLES,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_colour"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOR_FORMAT,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let mesh_attributes =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4];
        let mesh_pipeline_for = |label: &str, translucent: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_mesh"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<MeshVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &mesh_attributes,
                    })],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: depth(!translucent),
                multisample: wgpu::MultisampleState {
                    count: SAMPLES,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_mesh"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let mesh_pipeline = mesh_pipeline_for("nec-3d mesh", false);
        let mesh_blend_pipeline = mesh_pipeline_for("nec-3d mesh overlay", true);
        Gpu {
            device,
            queue: rs.queue.clone(),
            renderer: rs.renderer.clone(),
            layout,
            line_pipeline,
            mesh_pipeline,
            mesh_blend_pipeline,
        }
    }

    pub fn upload(&self, scene: &Scene) -> SceneBuffers {
        let lines = (!scene.lines.is_empty()).then(|| {
            let buf = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("nec-3d lines"),
                    contents: bytemuck::cast_slice(&scene.lines),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            (buf, scene.lines.len() as u32)
        });
        let mesh = (!scene.mesh_indices.is_empty()).then(|| {
            let vb = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("nec-3d mesh"),
                    contents: bytemuck::cast_slice(&scene.mesh),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            let ib = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("nec-3d mesh indices"),
                    contents: bytemuck::cast_slice(&scene.mesh_indices),
                    usage: wgpu::BufferUsages::INDEX,
                });
            (vb, ib, scene.mesh_indices.len() as u32)
        });
        SceneBuffers {
            lines,
            mesh,
            translucent: scene.mesh_translucent,
        }
    }
}

/// One 3D view's render targets and the egui texture showing them.
pub struct Viewport {
    size: [u32; 2],
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolve: wgpu::TextureView,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    pub texture_id: egui::TextureId,
}

impl Viewport {
    pub fn new(gpu: &Gpu, size: [u32; 2]) -> Viewport {
        let (msaa, depth, resolve) = Self::targets(gpu, size);
        let uniforms = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nec-3d uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nec-3d uniforms"),
            layout: &gpu.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let texture_id = gpu.renderer.write().register_native_texture(
            &gpu.device,
            &resolve,
            wgpu::FilterMode::Linear,
        );
        Viewport {
            size,
            msaa,
            depth,
            resolve,
            uniforms,
            bind_group,
            texture_id,
        }
    }

    fn targets(
        gpu: &Gpu,
        size: [u32; 2],
    ) -> (wgpu::TextureView, wgpu::TextureView, wgpu::TextureView) {
        let extent = wgpu::Extent3d {
            width: size[0].max(1),
            height: size[1].max(1),
            depth_or_array_layers: 1,
        };
        let make = |label, format, samples, usage| {
            gpu.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        (
            make(
                "nec-3d msaa",
                COLOR_FORMAT,
                SAMPLES,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            ),
            make(
                "nec-3d depth",
                DEPTH_FORMAT,
                SAMPLES,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            ),
            make(
                "nec-3d image",
                COLOR_FORMAT,
                1,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            ),
        )
    }

    fn resize(&mut self, gpu: &Gpu, size: [u32; 2]) {
        if size == self.size {
            return;
        }
        let (msaa, depth, resolve) = Self::targets(gpu, size);
        gpu.renderer.write().update_egui_texture_from_wgpu_texture(
            &gpu.device,
            &resolve,
            wgpu::FilterMode::Linear,
            self.texture_id,
        );
        (self.msaa, self.depth, self.resolve, self.size) = (msaa, depth, resolve, size);
    }

    /// Draws `scenes` (in order) into the view's texture.
    pub fn render(
        &mut self,
        gpu: &Gpu,
        size: [u32; 2],
        camera: &OrbitCamera,
        background: egui::Color32,
        scenes: &[&SceneBuffers],
    ) {
        self.resize(gpu, size);
        let aspect = size[0] as f32 / size[1].max(1) as f32;
        let light = (camera.eye() - camera.target).normalize() + Vec3::Z * 0.6;
        let u = Uniforms {
            view_proj: camera.view_proj(aspect).to_cols_array_2d(),
            viewport: [size[0] as f32, size[1] as f32, 0.0, 0.0],
            light_dir: [light.x, light.y, light.z, 0.0],
        };
        gpu.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));
        let [r, g, b, a] = rgba(background).map(f64::from);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("nec-3d"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("nec-3d"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.msaa,
                    depth_slice: None,
                    resolve_target: Some(&self.resolve),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.bind_group, &[]);
            // Opaque meshes, then lines, then see-through meshes on top.
            for s in scenes.iter().filter(|s| !s.translucent) {
                if let Some((vb, ib, n)) = &s.mesh {
                    pass.set_pipeline(&gpu.mesh_pipeline);
                    pass.set_vertex_buffer(0, vb.slice(..));
                    pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..*n, 0, 0..1);
                }
            }
            for s in scenes {
                if let Some((buf, n)) = &s.lines {
                    pass.set_pipeline(&gpu.line_pipeline);
                    pass.set_vertex_buffer(0, buf.slice(..));
                    pass.draw(0..6, 0..*n);
                }
            }
            for s in scenes.iter().filter(|s| s.translucent) {
                if let Some((vb, ib, n)) = &s.mesh {
                    pass.set_pipeline(&gpu.mesh_blend_pipeline);
                    pass.set_vertex_buffer(0, vb.slice(..));
                    pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..*n, 0, 0..1);
                }
            }
        }
        gpu.queue.submit(Some(encoder.finish()));
    }
}
