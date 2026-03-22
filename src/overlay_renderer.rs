use std::{borrow::Cow, mem, ptr::NonNull};

use anyhow::{Context, Result};
use bytemuck::{Pod, Zeroable};
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use smithay_client_toolkit::shm::{Shm, slot::SlotPool};
use tracing::warn;
use wayland_client::{
    Connection, Proxy,
    protocol::{wl_shm, wl_surface},
};
use wgpu::util::DeviceExt;

use crate::{
    geometry::{Rect, Size},
    render::{
        PanelAssets, PixelSurface, paint_background, paint_masks_and_border, paint_panel,
        panel_location,
    },
    state::SelectionModel,
};

const MAX_SOLID_INSTANCES: usize = 12;

#[derive(Debug, Clone)]
pub struct RendererOutputInit {
    pub wl_surface: wl_surface::WlSurface,
    pub logical_size: Size,
    pub scale_factor: i32,
    pub with_pointer: PixelSurface,
    pub without_pointer: PixelSurface,
}

pub(crate) enum OverlayRenderer {
    Wgpu(WgpuRenderer),
    Shm(ShmRenderer),
}

impl OverlayRenderer {
    pub fn new(
        conn: &Connection,
        shm: &Shm,
        outputs: Vec<RendererOutputInit>,
        panels: PanelAssets,
    ) -> Result<Self> {
        match WgpuRenderer::new(conn, &outputs, panels.clone()) {
            Ok(renderer) => Ok(Self::Wgpu(renderer)),
            Err(err) => {
                warn!("failed to initialize wgpu overlay renderer: {err:#}");
                warn!("falling back to cairo/shm overlay renderer");
                Ok(Self::Shm(ShmRenderer::new(shm, outputs, panels)?))
            }
        }
    }

    pub fn resize_output(
        &mut self,
        index: usize,
        logical_size: Size,
        scale_factor: i32,
    ) -> Result<()> {
        match self {
            Self::Wgpu(renderer) => renderer.resize_output(index, logical_size, scale_factor),
            Self::Shm(renderer) => renderer.resize_output(index, logical_size, scale_factor),
        }
    }

    pub fn draw(&mut self, model: &SelectionModel) -> Result<()> {
        match self {
            Self::Wgpu(renderer) => renderer.draw(model),
            Self::Shm(renderer) => renderer.draw(model),
        }
    }
}

pub(crate) struct ShmRenderer {
    outputs: Vec<ShmOutput>,
    panels: PanelAssets,
}

struct ShmOutput {
    wl_surface: wl_surface::WlSurface,
    logical_size: Size,
    pool: SlotPool,
    with_pointer: PixelSurface,
    without_pointer: PixelSurface,
}

impl ShmRenderer {
    fn new(shm: &Shm, outputs: Vec<RendererOutputInit>, panels: PanelAssets) -> Result<Self> {
        let mut renderer_outputs = Vec::with_capacity(outputs.len());
        for output in outputs {
            output.wl_surface.set_buffer_scale(1);
            renderer_outputs.push(ShmOutput {
                wl_surface: output.wl_surface,
                logical_size: output.logical_size,
                pool: SlotPool::new(
                    (output.logical_size.width.max(1) * output.logical_size.height.max(1) * 4)
                        as usize,
                    shm,
                )
                .context("failed to create shm pool")?,
                with_pointer: output.with_pointer,
                without_pointer: output.without_pointer,
            });
        }

        Ok(Self {
            outputs: renderer_outputs,
            panels,
        })
    }

    fn resize_output(
        &mut self,
        index: usize,
        logical_size: Size,
        _scale_factor: i32,
    ) -> Result<()> {
        let Some(output) = self.outputs.get_mut(index) else {
            anyhow::bail!("invalid output index {index} for shm renderer");
        };
        output.logical_size = logical_size;
        Ok(())
    }

    fn draw(&mut self, model: &SelectionModel) -> Result<()> {
        for (index, output) in self.outputs.iter_mut().enumerate() {
            let width = output.logical_size.width.max(1) as u32;
            let height = output.logical_size.height.max(1) as u32;
            let stride = width as i32 * 4;
            let (buffer, canvas) = output
                .pool
                .create_buffer(
                    width as i32,
                    height as i32,
                    stride,
                    wl_shm::Format::Argb8888,
                )
                .context("failed to create overlay buffer")?;
            let mut surface =
                cairo::ImageSurface::create(cairo::Format::ARgb32, width as i32, height as i32)?;
            {
                let cr = cairo::Context::new(&surface)?;
                let screenshot = if model.show_pointer {
                    &mut output.with_pointer
                } else {
                    &mut output.without_pointer
                };
                paint_background(&cr, screenshot, output.logical_size)?;
                paint_masks_and_border(&cr, output.logical_size, model.selection_on_output(index))?;

                let panel = if model.show_pointer {
                    &mut self.panels.hide_pointer
                } else {
                    &mut self.panels.show_pointer
                };
                let _panel_rect =
                    paint_panel(&cr, panel, output.logical_size, model.dragging_selection())?;
            }

            surface.flush();
            let data = surface.data().context(
                "overlay render surface still had live cairo references during shm copy",
            )?;
            canvas.copy_from_slice(&data);

            output
                .wl_surface
                .damage_buffer(0, 0, width as i32, height as i32);
            buffer
                .attach_to(&output.wl_surface)
                .context("failed to attach overlay buffer")?;
            output.wl_surface.commit();
        }

        Ok(())
    }
}

pub(crate) struct WgpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    quad_vertices: wgpu::Buffer,
    textured_pipeline: wgpu::RenderPipeline,
    solid_pipeline: wgpu::RenderPipeline,
    panel_show: GpuTexture,
    panel_hide: GpuTexture,
    outputs: Vec<GpuOutput>,
}

struct GpuOutput {
    wl_surface: wl_surface::WlSurface,
    logical_size: Size,
    scale_factor: i32,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    output_uniform: wgpu::Buffer,
    solid_bind_group: wgpu::BindGroup,
    with_pointer_bind_group: wgpu::BindGroup,
    without_pointer_bind_group: wgpu::BindGroup,
    panel_show_bind_group: wgpu::BindGroup,
    panel_hide_bind_group: wgpu::BindGroup,
    textured_instances: wgpu::Buffer,
    solid_instances: wgpu::Buffer,
    _with_pointer_texture: GpuTexture,
    _without_pointer_texture: GpuTexture,
}

struct GpuTexture {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: Size,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct QuadVertex {
    position: [f32; 2],
    uv: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct OutputUniform {
    output_size: [f32; 2],
    _pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TexturedInstance {
    rect: [f32; 4],
    modulate: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SolidInstance {
    rect: [f32; 4],
    color: [f32; 4],
}

impl WgpuRenderer {
    fn new(conn: &Connection, outputs: &[RendererOutputInit], panels: PanelAssets) -> Result<Self> {
        let instance = wgpu::Instance::default();

        let raw_display_handle = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
            NonNull::new(conn.backend().display_ptr() as *mut _)
                .context("wayland display handle is null")?,
        ));

        let mut gpu_outputs = Vec::with_capacity(outputs.len());
        for output in outputs {
            let raw_window_handle = RawWindowHandle::Wayland(WaylandWindowHandle::new(
                NonNull::new(output.wl_surface.id().as_ptr() as *mut _)
                    .context("wayland surface handle is null")?,
            ));
            let surface = unsafe {
                instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(raw_display_handle),
                    raw_window_handle,
                })
            }
            .context("failed to create wgpu surface for layer-shell output")?;

            gpu_outputs.push(GpuSurfaceSeed {
                wl_surface: output.wl_surface.clone(),
                logical_size: output.logical_size,
                scale_factor: output.scale_factor.max(1),
                surface,
                with_pointer: output.with_pointer.clone(),
                without_pointer: output.without_pointer.clone(),
            });
        }

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: gpu_outputs.first().map(|output| &output.surface),
            ..Default::default()
        }))
        .context("failed to find a wgpu adapter for the overlay surfaces")?;

        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))
            .context("failed to request a wgpu device")?;

        let quad_vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("snappers-quad-vertices"),
            contents: bytemuck::cast_slice(&quad_vertices()),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let solid_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("snappers-solid-bind-group-layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: Some(
                            wgpu::BufferSize::new(mem::size_of::<OutputUniform>() as u64)
                                .expect("output uniform is non-empty"),
                        ),
                    },
                    count: None,
                }],
            });

        let textured_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("snappers-textured-bind-group-layout"),
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
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: Some(
                                wgpu::BufferSize::new(mem::size_of::<OutputUniform>() as u64)
                                    .expect("output uniform is non-empty"),
                            ),
                        },
                        count: None,
                    },
                ],
            });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("snappers-overlay-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        let panel_show = upload_texture(&device, &queue, &panels.show_pointer, "panel-show")?;
        let panel_hide = upload_texture(&device, &queue, &panels.hide_pointer, "panel-hide")?;

        let surface_format = choose_surface_format(
            &gpu_outputs
                .first()
                .context("wgpu renderer requires at least one output")?
                .surface
                .get_capabilities(&adapter),
        )
        .context("overlay surface has no supported texture format")?;

        let textured_pipeline =
            create_textured_pipeline(&device, surface_format, &textured_bind_group_layout);
        let solid_pipeline =
            create_solid_pipeline(&device, surface_format, &solid_bind_group_layout);

        let outputs = gpu_outputs
            .into_iter()
            .map(|seed| {
                let output_uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("snappers-output-uniform"),
                    contents: bytemuck::bytes_of(&OutputUniform::new(seed.logical_size)),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                });

                let config = surface_config(
                    &seed.surface.get_capabilities(&adapter),
                    surface_format,
                    seed.logical_size,
                    seed.scale_factor,
                )?;
                seed.surface.configure(&device, &config);
                seed.wl_surface.set_buffer_scale(seed.scale_factor);

                let solid_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("snappers-solid-bind-group"),
                    layout: &solid_bind_group_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: output_uniform.as_entire_binding(),
                    }],
                });

                let with_pointer_texture =
                    upload_texture(&device, &queue, &seed.with_pointer, "with-pointer")?;
                let without_pointer_texture =
                    upload_texture(&device, &queue, &seed.without_pointer, "without-pointer")?;

                let with_pointer_bind_group = create_textured_bind_group(
                    &device,
                    &textured_bind_group_layout,
                    &with_pointer_texture.view,
                    &sampler,
                    &output_uniform,
                    "snappers-with-pointer-bind-group",
                );
                let without_pointer_bind_group = create_textured_bind_group(
                    &device,
                    &textured_bind_group_layout,
                    &without_pointer_texture.view,
                    &sampler,
                    &output_uniform,
                    "snappers-without-pointer-bind-group",
                );
                let panel_show_bind_group = create_textured_bind_group(
                    &device,
                    &textured_bind_group_layout,
                    &panel_show.view,
                    &sampler,
                    &output_uniform,
                    "snappers-panel-show-bind-group",
                );
                let panel_hide_bind_group = create_textured_bind_group(
                    &device,
                    &textured_bind_group_layout,
                    &panel_hide.view,
                    &sampler,
                    &output_uniform,
                    "snappers-panel-hide-bind-group",
                );

                let textured_instances = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("snappers-textured-instances"),
                    size: (2 * mem::size_of::<TexturedInstance>()) as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let solid_instances = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("snappers-solid-instances"),
                    size: (MAX_SOLID_INSTANCES * mem::size_of::<SolidInstance>()) as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });

                Ok(GpuOutput {
                    wl_surface: seed.wl_surface,
                    logical_size: seed.logical_size,
                    scale_factor: seed.scale_factor,
                    surface: seed.surface,
                    config,
                    output_uniform,
                    solid_bind_group,
                    with_pointer_bind_group,
                    without_pointer_bind_group,
                    panel_show_bind_group,
                    panel_hide_bind_group,
                    textured_instances,
                    solid_instances,
                    _with_pointer_texture: with_pointer_texture,
                    _without_pointer_texture: without_pointer_texture,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            device,
            queue,
            quad_vertices,
            textured_pipeline,
            solid_pipeline,
            panel_show,
            panel_hide,
            outputs,
        })
    }

    fn resize_output(&mut self, index: usize, logical_size: Size, scale_factor: i32) -> Result<()> {
        let Some(output) = self.outputs.get_mut(index) else {
            anyhow::bail!("invalid output index {index} for wgpu renderer");
        };
        output.logical_size = logical_size;
        output.scale_factor = scale_factor.max(1);
        output.wl_surface.set_buffer_scale(output.scale_factor);
        output.config.width = physical_extent(logical_size.width, output.scale_factor);
        output.config.height = physical_extent(logical_size.height, output.scale_factor);
        output.surface.configure(&self.device, &output.config);
        self.queue.write_buffer(
            &output.output_uniform,
            0,
            bytemuck::bytes_of(&OutputUniform::new(logical_size)),
        );
        Ok(())
    }

    fn draw(&mut self, model: &SelectionModel) -> Result<()> {
        for index in 0..self.outputs.len() {
            self.draw_output(index, model)?;
        }
        Ok(())
    }

    fn draw_output(&mut self, index: usize, model: &SelectionModel) -> Result<()> {
        let output = self
            .outputs
            .get_mut(index)
            .with_context(|| format!("invalid output index {index}"))?;
        let frame = match acquire_surface_texture(&output.surface, &self.device, &output.config)? {
            Some(frame) => frame,
            None => return Ok(()),
        };

        let panel = if model.show_pointer {
            &self.panel_hide
        } else {
            &self.panel_show
        };
        let textured_instances = textured_instances(
            output.logical_size,
            panel.size(),
            model.dragging_selection(),
        );
        self.queue.write_buffer(
            &output.textured_instances,
            0,
            bytemuck::cast_slice(&textured_instances),
        );

        let solid_instances =
            solid_instances(output.logical_size, model.selection_on_output(index));
        self.queue.write_buffer(
            &output.solid_instances,
            0,
            bytemuck::cast_slice(&solid_instances),
        );

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("snappers-overlay-encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("snappers-overlay-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });

            pass.set_vertex_buffer(0, self.quad_vertices.slice(..));

            pass.set_pipeline(&self.textured_pipeline);
            pass.set_vertex_buffer(
                1,
                output
                    .textured_instances
                    .slice(0..mem::size_of::<TexturedInstance>() as u64),
            );
            let screenshot_bind_group = if model.show_pointer {
                &output.with_pointer_bind_group
            } else {
                &output.without_pointer_bind_group
            };
            pass.set_bind_group(0, screenshot_bind_group, &[]);
            pass.draw(0..6, 0..1);

            pass.set_vertex_buffer(
                1,
                output.textured_instances.slice(
                    mem::size_of::<TexturedInstance>() as u64
                        ..(2 * mem::size_of::<TexturedInstance>()) as u64,
                ),
            );
            let panel_bind_group = if model.show_pointer {
                &output.panel_hide_bind_group
            } else {
                &output.panel_show_bind_group
            };
            pass.set_bind_group(0, panel_bind_group, &[]);
            pass.draw(0..6, 0..1);

            if !solid_instances.is_empty() {
                pass.set_pipeline(&self.solid_pipeline);
                pass.set_vertex_buffer(
                    1,
                    output
                        .solid_instances
                        .slice(0..(solid_instances.len() * mem::size_of::<SolidInstance>()) as u64),
                );
                pass.set_bind_group(0, &output.solid_bind_group, &[]);
                pass.draw(0..6, 0..solid_instances.len() as u32);
            }
        }

        self.queue.submit([encoder.finish()]);
        frame.present();
        Ok(())
    }
}

struct GpuSurfaceSeed {
    wl_surface: wl_surface::WlSurface,
    logical_size: Size,
    scale_factor: i32,
    surface: wgpu::Surface<'static>,
    with_pointer: PixelSurface,
    without_pointer: PixelSurface,
}

impl OutputUniform {
    fn new(output_size: Size) -> Self {
        Self {
            output_size: [
                output_size.width.max(1) as f32,
                output_size.height.max(1) as f32,
            ],
            _pad: [0.0; 2],
        }
    }
}

impl GpuTexture {
    fn size(&self) -> Size {
        self.size
    }
}

fn create_textured_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    output_uniform: &wgpu::Buffer,
    label: &str,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: output_uniform.as_entire_binding(),
            },
        ],
    })
}

fn create_textured_pipeline(
    device: &wgpu::Device,
    surface_format: wgpu::TextureFormat,
    bind_group_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("snappers-textured-shader"),
        source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(TEXTURED_SHADER)),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("snappers-textured-pipeline-layout"),
        bind_group_layouts: &[Some(bind_group_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("snappers-textured-pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[quad_vertex_layout(), textured_instance_layout()],
            compilation_options: Default::default(),
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(premultiplied_alpha_blend()),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn create_solid_pipeline(
    device: &wgpu::Device,
    surface_format: wgpu::TextureFormat,
    bind_group_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("snappers-solid-shader"),
        source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SOLID_SHADER)),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("snappers-solid-pipeline-layout"),
        bind_group_layouts: &[Some(bind_group_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("snappers-solid-pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[quad_vertex_layout(), solid_instance_layout()],
            compilation_options: Default::default(),
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(premultiplied_alpha_blend()),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    surface: &PixelSurface,
    label: &str,
) -> Result<GpuTexture> {
    let size = wgpu::Extent3d {
        width: surface.width.max(1) as u32,
        height: surface.height.max(1) as u32,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8UnormSrgb,
        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &surface.data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some((surface.width.max(1) * 4) as u32),
            rows_per_image: Some(surface.height.max(1) as u32),
        },
        size,
    );

    Ok(GpuTexture {
        view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
        _texture: texture,
        size: Size::new(size.width as i32, size.height as i32),
    })
}

fn choose_surface_format(capabilities: &wgpu::SurfaceCapabilities) -> Option<wgpu::TextureFormat> {
    capabilities
        .formats
        .iter()
        .copied()
        .find(wgpu::TextureFormat::is_srgb)
        .or_else(|| capabilities.formats.first().copied())
}

fn surface_config(
    capabilities: &wgpu::SurfaceCapabilities,
    format: wgpu::TextureFormat,
    logical_size: Size,
    scale_factor: i32,
) -> Result<wgpu::SurfaceConfiguration> {
    let present_mode = if capabilities
        .present_modes
        .contains(&wgpu::PresentMode::Mailbox)
    {
        wgpu::PresentMode::Mailbox
    } else {
        *capabilities
            .present_modes
            .first()
            .context("overlay surface does not expose any present modes")?
    };
    let alpha_mode = if capabilities
        .alpha_modes
        .contains(&wgpu::CompositeAlphaMode::Auto)
    {
        wgpu::CompositeAlphaMode::Auto
    } else {
        *capabilities
            .alpha_modes
            .first()
            .context("overlay surface does not expose any alpha modes")?
    };

    Ok(wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: physical_extent(logical_size.width, scale_factor),
        height: physical_extent(logical_size.height, scale_factor),
        present_mode,
        alpha_mode,
        view_formats: vec![format],
        desired_maximum_frame_latency: 2,
    })
}

fn physical_extent(logical_extent: i32, scale_factor: i32) -> u32 {
    logical_extent.max(1).saturating_mul(scale_factor.max(1)) as u32
}

fn acquire_surface_texture(
    surface: &wgpu::Surface<'static>,
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> Result<Option<wgpu::SurfaceTexture>> {
    match surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(frame)
        | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Ok(Some(frame)),
        wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => Ok(None),
        wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
            surface.configure(device, config);
            match surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame)
                | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Ok(Some(frame)),
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                    Ok(None)
                }
                wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                    anyhow::bail!("overlay surface remained unavailable after reconfigure")
                }
                wgpu::CurrentSurfaceTexture::Validation => {
                    anyhow::bail!(
                        "wgpu returned a validation error while acquiring the overlay frame"
                    )
                }
            }
        }
        wgpu::CurrentSurfaceTexture::Validation => {
            anyhow::bail!("wgpu returned a validation error while acquiring the overlay frame")
        }
    }
}

fn quad_vertices() -> [QuadVertex; 6] {
    [
        QuadVertex {
            position: [0.0, 0.0],
            uv: [0.0, 0.0],
        },
        QuadVertex {
            position: [1.0, 0.0],
            uv: [1.0, 0.0],
        },
        QuadVertex {
            position: [0.0, 1.0],
            uv: [0.0, 1.0],
        },
        QuadVertex {
            position: [0.0, 1.0],
            uv: [0.0, 1.0],
        },
        QuadVertex {
            position: [1.0, 0.0],
            uv: [1.0, 0.0],
        },
        QuadVertex {
            position: [1.0, 1.0],
            uv: [1.0, 1.0],
        },
    ]
}

fn quad_vertex_layout<'a>() -> wgpu::VertexBufferLayout<'a> {
    wgpu::VertexBufferLayout {
        array_stride: mem::size_of::<QuadVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x2,
            },
            wgpu::VertexAttribute {
                offset: mem::size_of::<[f32; 2]>() as u64,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32x2,
            },
        ],
    }
}

fn textured_instance_layout<'a>() -> wgpu::VertexBufferLayout<'a> {
    wgpu::VertexBufferLayout {
        array_stride: mem::size_of::<TexturedInstance>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &[
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 2,
                format: wgpu::VertexFormat::Float32x4,
            },
            wgpu::VertexAttribute {
                offset: mem::size_of::<[f32; 4]>() as u64,
                shader_location: 3,
                format: wgpu::VertexFormat::Float32x4,
            },
        ],
    }
}

fn solid_instance_layout<'a>() -> wgpu::VertexBufferLayout<'a> {
    wgpu::VertexBufferLayout {
        array_stride: mem::size_of::<SolidInstance>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &[
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 2,
                format: wgpu::VertexFormat::Float32x4,
            },
            wgpu::VertexAttribute {
                offset: mem::size_of::<[f32; 4]>() as u64,
                shader_location: 3,
                format: wgpu::VertexFormat::Float32x4,
            },
        ],
    }
}

fn premultiplied_alpha_blend() -> wgpu::BlendState {
    wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
    }
}

fn textured_instances(
    output_size: Size,
    panel: Size,
    dragging_selection: bool,
) -> [TexturedInstance; 2] {
    let panel_origin = panel_location(output_size, panel);
    [
        TexturedInstance {
            rect: rect_to_f32(Rect::new(0, 0, output_size.width, output_size.height)),
            modulate: [1.0, 1.0, 1.0, 1.0],
        },
        TexturedInstance {
            rect: rect_to_f32(Rect::new(
                panel_origin.x,
                panel_origin.y,
                panel.width,
                panel.height,
            )),
            modulate: [1.0, 1.0, 1.0, if dragging_selection { 0.3 } else { 0.9 }],
        },
    ]
}

fn solid_instances(output_size: Size, selection: Option<Rect>) -> Vec<SolidInstance> {
    let mut instances = Vec::with_capacity(MAX_SOLID_INSTANCES);
    if let Some(selection) = selection {
        let dims = mask_rects(output_size, selection);
        instances.extend(dims.into_iter().map(|rect| SolidInstance {
            rect: rect_to_f32(rect),
            color: [0.0, 0.0, 0.0, 0.5],
        }));
        instances.extend(
            border_rects(selection)
                .into_iter()
                .map(|rect| SolidInstance {
                    rect: rect_to_f32(rect),
                    color: [1.0, 1.0, 1.0, 1.0],
                }),
        );
    } else {
        instances.push(SolidInstance {
            rect: rect_to_f32(Rect::new(0, 0, output_size.width, output_size.height)),
            color: [0.0, 0.0, 0.0, 0.5],
        });
    }
    instances
}

fn mask_rects(output_size: Size, selection: Rect) -> [Rect; 4] {
    [
        Rect::new(0, 0, output_size.width, selection.y.max(0)),
        Rect::new(
            0,
            selection.y + selection.height,
            output_size.width,
            (output_size.height - selection.y - selection.height).max(0),
        ),
        Rect::new(0, selection.y, selection.x.max(0), selection.height),
        Rect::new(
            selection.x + selection.width,
            selection.y,
            (output_size.width - selection.x - selection.width).max(0),
            selection.height,
        ),
    ]
}

fn border_rects(selection: Rect) -> [Rect; 4] {
    let border = crate::render::SELECTION_BORDER.max(1);
    [
        Rect::new(
            selection.x - border / 2,
            selection.y - border / 2,
            selection.width + border,
            border,
        ),
        Rect::new(
            selection.x - border / 2,
            selection.y + selection.height - border / 2,
            selection.width + border,
            border,
        ),
        Rect::new(
            selection.x - border / 2,
            selection.y - border / 2,
            border,
            selection.height + border,
        ),
        Rect::new(
            selection.x + selection.width - border / 2,
            selection.y - border / 2,
            border,
            selection.height + border,
        ),
    ]
}

fn rect_to_f32(rect: Rect) -> [f32; 4] {
    [
        rect.x as f32,
        rect.y as f32,
        rect.width.max(0) as f32,
        rect.height.max(0) as f32,
    ]
}

const TEXTURED_SHADER: &str = r#"
struct OutputUniform {
    output_size: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var tex_sampler: sampler;
@group(0) @binding(2) var<uniform> output_uniform: OutputUniform;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) rect: vec4<f32>,
    @location(3) modulate: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) modulate: vec4<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let logical = input.rect.xy + input.position * input.rect.zw;
    let ndc = vec2(
        (logical.x / output_uniform.output_size.x) * 2.0 - 1.0,
        1.0 - (logical.y / output_uniform.output_size.y) * 2.0
    );

    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = input.uv;
    out.modulate = input.modulate;
    return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(tex, tex_sampler, input.uv) * input.modulate;
}
"#;

const SOLID_SHADER: &str = r#"
struct OutputUniform {
    output_size: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> output_uniform: OutputUniform;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) _uv: vec2<f32>,
    @location(2) rect: vec4<f32>,
    @location(3) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let logical = input.rect.xy + input.position * input.rect.zw;
    let ndc = vec2(
        (logical.x / output_uniform.output_size.x) * 2.0 - 1.0,
        1.0 - (logical.y / output_uniform.output_size.y) * 2.0
    );

    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.color = input.color;
    return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return input.color;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_instances_cover_masks_and_border() {
        let instances = solid_instances(Size::new(800, 600), Some(Rect::new(100, 120, 200, 160)));
        assert_eq!(instances.len(), 8);
        assert_eq!(instances[0].rect, rect_to_f32(Rect::new(0, 0, 800, 120)));
    }

    #[test]
    fn solid_instances_dim_whole_output_without_selection() {
        let instances = solid_instances(Size::new(800, 600), None);
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].rect, rect_to_f32(Rect::new(0, 0, 800, 600)));
    }
}
