//! Copies the renderer's output into a window's surface (or any colour target) whose format
//! may differ: a full-screen triangle samples the output and writes it, sRGB-encoding it in the
//! shader when the target does not do that itself.
//!
//! VisionCraft's app hands its renderer's output view to egui, which samples it into the
//! surface (`apps/visioncraft/src/lib.rs`, `register_native_texture`); without a UI toolkit this
//! is the same step, done directly.

use crate::RenderError;

/// The shader: a full-screen triangle, a plain copy for sRGB targets and an encoding copy for
/// the others.
const SHADER: &str = r"
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var linear_clamp: sampler;

struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Varyings {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: Varyings;
    out.position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_copy(in: Varyings) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSample(source, linear_clamp, in.uv).rgb, 1.0);
}

fn srgb_encode(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055, x * 12.92, x <= vec3<f32>(0.0031308));
}

@fragment
fn fs_encode(in: Varyings) -> @location(0) vec4<f32> {
    return vec4<f32>(srgb_encode(textureSample(source, linear_clamp, in.uv).rgb), 1.0);
}
";

/// The blit's shader, layout and sampler, and a pipeline per target format met so far.
pub(crate) struct Blit {
    module: wgpu::ShaderModule,
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    sampler: wgpu::Sampler,
    pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
}

impl Blit {
    /// The shader and layouts on `device`; pipelines are made on first use per format.
    pub(crate) fn new(device: &wgpu::Device) -> Blit {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gamecraft blit"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gamecraft blit"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gamecraft blit"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gamecraft blit"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Blit {
            module,
            layout,
            pipeline_layout,
            sampler,
            pipelines: Vec::new(),
        }
    }

    /// Checks that `target` can be drawn into: a single-sampled colour texture that is a
    /// render attachment. The error names what is wrong.
    pub(crate) fn check_target(
        device: &wgpu::Device,
        target: &wgpu::TextureView,
    ) -> Result<wgpu::TextureFormat, RenderError> {
        let texture = target.texture();
        let format = texture.format();
        let renderable = format
            .guaranteed_format_features(device.features())
            .allowed_usages
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT);
        if format.is_depth_stencil_format() || !renderable {
            return Err(RenderError::BadRequest(format!(
                "the target's format {format:?} is not a colour format that can be drawn into; \
                 use for example Rgba8UnormSrgb or Bgra8Unorm"
            )));
        }
        if !texture
            .usage()
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        {
            return Err(RenderError::BadRequest(
                "the target texture was made without RENDER_ATTACHMENT usage; add it".into(),
            ));
        }
        if texture.sample_count() != 1 {
            return Err(RenderError::BadRequest(format!(
                "the target texture has {} samples; draw into a single-sampled texture",
                texture.sample_count()
            )));
        }
        Ok(format)
    }

    /// Records the copy of `source` (the renderer's output view) into `target`, stretched to
    /// its size. `format` must be the one [`Blit::check_target`] returned for `target`.
    pub(crate) fn record(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
    ) {
        let index = match self.pipelines.iter().position(|(f, _)| *f == format) {
            Some(i) => i,
            None => {
                let pipeline = self.pipeline(device, format);
                self.pipelines.push((format, pipeline));
                self.pipelines.len() - 1
            }
        };
        let Some((_, pipeline)) = self.pipelines.get(index) else {
            return;
        };
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gamecraft blit"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("gamecraft blit"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// The pipeline for a target of `format`: a plain copy when the target encodes sRGB itself,
    /// else one that encodes in the shader.
    fn pipeline(&self, device: &wgpu::Device, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        let fragment = if format.is_srgb() {
            "fs_copy"
        } else {
            "fs_encode"
        };
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gamecraft blit"),
            layout: Some(&self.pipeline_layout),
            vertex: wgpu::VertexState {
                module: &self.module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &self.module,
                entry_point: Some(fragment),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    }
}
