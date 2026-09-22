use std::sync::Arc;

use eframe::{egui, egui_wgpu, wgpu};

use crate::platform::windows::desktop_capture::DesktopFrame;

const SHADER: &str = include_str!("liquid_glass.wgsl");
const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const UNIFORM_SIZE: u64 = 64;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    refraction: f32,
    frost: f32,
    dispersion: f32,
    magnification: f32,
    squircle: bool,
    depth: f32,
    tint: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            refraction: 0.8,
            frost: 0.45,
            dispersion: 3.0,
            magnification: 1.0,
            squircle: false,
            depth: 0.3,
            tint: 0.14,
        }
    }
}

impl Settings {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.add(egui::Slider::new(&mut self.refraction, 0.0..=1.2).text("Refraction"));
        ui.add(egui::Slider::new(&mut self.frost, 0.0..=1.0).text("Frost"));
        ui.add(egui::Slider::new(&mut self.dispersion, 0.0..=8.0).text("Chromatic aberration"));
        ui.add(egui::Slider::new(&mut self.magnification, 1.0..=1.12).text("Magnification"));
        ui.add(egui::Slider::new(&mut self.depth, 0.05..=0.45).text("Lens depth"));
        ui.add(egui::Slider::new(&mut self.tint, 0.0..=0.6).text("Dark tint"));
        ui.checkbox(&mut self.squircle, "Squircle corners");
        if ui.button("Reset glass settings").clicked() {
            *self = Self::default();
        }
    }
}

pub fn visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(egui::Color32::from_rgb(235, 243, 250));
    visuals.panel_fill = egui::Color32::TRANSPARENT;
    visuals.window_fill = egui::Color32::from_rgb(27, 39, 55);
    visuals.extreme_bg_color = egui::Color32::from_rgba_unmultiplied(8, 17, 29, 140);
    visuals.faint_bg_color = egui::Color32::from_white_alpha(8);
    visuals.window_stroke = egui::Stroke::new(1.0, egui::Color32::from_white_alpha(50));
    visuals.window_corner_radius = egui::CornerRadius::same(12);
    visuals.menu_corner_radius = egui::CornerRadius::same(10);
    visuals.selection.bg_fill = egui::Color32::from_rgb(36, 106, 151);
    visuals.widgets.noninteractive.bg_fill = egui::Color32::from_white_alpha(10);
    visuals.widgets.inactive.bg_fill = egui::Color32::from_white_alpha(18);
    visuals.widgets.hovered.bg_fill = egui::Color32::from_white_alpha(38);
    visuals.widgets.active.bg_fill = egui::Color32::from_white_alpha(55);
    visuals
}

pub fn release(state: &egui_wgpu::RenderState) {
    state
        .renderer
        .write()
        .callback_resources
        .remove::<Resources>();
}

pub fn callback(
    rect: egui::Rect,
    background: Arc<DesktopFrame>,
    settings: Settings,
    format: wgpu::TextureFormat,
) -> egui::PaintCallback {
    egui_wgpu::Callback::new_paint_callback(
        rect,
        GlassCallback {
            rect,
            background,
            settings,
            format,
        },
    )
}

struct GlassCallback {
    rect: egui::Rect,
    background: Arc<DesktopFrame>,
    settings: Settings,
    format: wgpu::TextureFormat,
}

impl GlassCallback {
    fn uniforms(&self, pixels_per_point: f32) -> Vec<u8> {
        let source_uv = self.background.source_uv();
        let values = [
            self.rect.width().max(1.0),
            self.rect.height().max(1.0),
            pixels_per_point,
            f32::from(self.format.is_srgb()),
            self.settings.refraction,
            self.settings.frost,
            self.settings.dispersion,
            self.settings.magnification,
            100.0,
            f32::from(self.settings.squircle),
            self.settings.depth,
            self.settings.tint,
            source_uv[0],
            source_uv[1],
            source_uv[2],
            source_uv[3],
        ];
        values.into_iter().flat_map(f32::to_ne_bytes).collect()
    }
}

impl egui_wgpu::CallbackTrait for GlassCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if resources.get::<Resources>().is_none() {
            resources.insert(Resources::new(device, self.format));
        }
        if let Some(resources) = resources.get_mut::<Resources>() {
            let size = self.background.size;
            if resources
                .targets
                .as_ref()
                .is_none_or(|targets| targets.size != size)
            {
                resources.targets = Some(Targets::new(device, &resources.texture_layout, size));
                resources.cached_frost = None;
                resources.cached_sequence = None;
            }
            queue.write_buffer(
                &resources.uniforms,
                0,
                &self.uniforms(screen.pixels_per_point),
            );
            let new_frame = resources.cached_sequence != Some(self.background.sequence);
            if resources.cached_frost != Some(self.settings.frost) || new_frame {
                if let Some(targets) = &resources.targets {
                    if new_frame {
                        queue.write_texture(
                            targets.scene_texture.as_image_copy(),
                            &self.background.rgba,
                            wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(size[0] * 4),
                                rows_per_image: None,
                            },
                            targets.scene_texture.size(),
                        );
                    }
                    resources.offscreen(
                        encoder,
                        &targets.horizontal,
                        &resources.blur_horizontal,
                        Some(&targets.scene_binding),
                    );
                    resources.offscreen(
                        encoder,
                        &targets.blurred,
                        &resources.blur_vertical,
                        Some(&targets.horizontal_binding),
                    );
                }
                resources.cached_frost = Some(self.settings.frost);
                resources.cached_sequence = Some(self.background.sequence);
            }
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(resources) = resources.get::<Resources>()
            && let Some(targets) = &resources.targets
        {
            pass.set_pipeline(&resources.glass);
            pass.set_bind_group(0, &resources.uniform_binding, &[]);
            pass.set_bind_group(1, &targets.glass_binding, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
// Mirror the capture downscale when exercising synthetic GPU inputs.
fn texture_size(size: egui::Vec2, pixels_per_point: f32, limit: u32) -> [u32; 2] {
    let physical = size * pixels_per_point;
    let scale = (0.5_f32).min(limit.clamp(1, 2048) as f32 / physical.max_elem().max(1.0));
    [
        (physical.x * scale).ceil().max(1.0) as u32,
        (physical.y * scale).ceil().max(1.0) as u32,
    ]
}

struct Resources {
    uniforms: wgpu::Buffer,
    uniform_binding: wgpu::BindGroup,
    texture_layout: wgpu::BindGroupLayout,
    blur_horizontal: wgpu::RenderPipeline,
    blur_vertical: wgpu::RenderPipeline,
    glass: wgpu::RenderPipeline,
    targets: Option<Targets>,
    cached_frost: Option<f32>,
    cached_sequence: Option<u64>,
}

impl Resources {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Liquid Glass WGSL"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Glass uniforms"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(UNIFORM_SIZE),
                },
                count: None,
            }],
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Glass uniforms"),
            size: UNIFORM_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Glass uniforms"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Glass textures"),
            entries: &[
                texture_entry(0),
                texture_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampling_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Glass sampling layout"),
            bind_group_layouts: &[Some(&uniform_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        Self {
            blur_horizontal: pipeline(
                device,
                &shader,
                &sampling_layout,
                "blur_horizontal",
                OFFSCREEN_FORMAT,
            ),
            blur_vertical: pipeline(
                device,
                &shader,
                &sampling_layout,
                "blur_vertical",
                OFFSCREEN_FORMAT,
            ),
            glass: pipeline(device, &shader, &sampling_layout, "glass", format),
            uniforms,
            uniform_binding,
            texture_layout,
            targets: None,
            cached_frost: None,
            cached_sequence: None,
        }
    }

    fn offscreen(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        pipeline: &wgpu::RenderPipeline,
        textures: Option<&wgpu::BindGroup>,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Liquid Glass offscreen"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.uniform_binding, &[]);
        if let Some(textures) = textures {
            pass.set_bind_group(1, textures, &[]);
        }
        pass.draw(0..3, 0..1);
    }
}

fn pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    entry: &str,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vertex"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(entry),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        multiview_mask: None,
        cache: None,
    })
}

struct Targets {
    size: [u32; 2],
    scene_texture: wgpu::Texture,
    horizontal: wgpu::TextureView,
    blurred: wgpu::TextureView,
    scene_binding: wgpu::BindGroup,
    horizontal_binding: wgpu::BindGroup,
    glass_binding: wgpu::BindGroup,
}

impl Targets {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, size: [u32; 2]) -> Self {
        let texture = |label| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: OFFSCREEN_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let scene_texture = texture("Captured desktop");
        let scene = scene_texture.create_view(&Default::default());
        let horizontal = texture("Glass horizontal blur").create_view(&Default::default());
        let blurred = texture("Glass vertical blur").create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Glass clamp sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let binding = |first: &wgpu::TextureView, second: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Glass sampled textures"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(first),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(second),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            })
        };
        Self {
            size,
            scene_binding: binding(&scene, &scene),
            horizontal_binding: binding(&horizontal, &horizontal),
            glass_binding: binding(&scene, &blurred),
            scene_texture,
            horizontal,
            blurred,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offscreen_size_handles_dpi_zero_and_limits() {
        assert_eq!(
            texture_size(egui::vec2(1280.0, 800.0), 1.0, 8192),
            [640, 400]
        );
        assert_eq!(
            texture_size(egui::vec2(1280.0, 800.0), 2.0, 8192),
            [1280, 800]
        );
        assert_eq!(
            texture_size(egui::vec2(7680.0, 4320.0), 2.0, 8192),
            [2048, 1152]
        );
        assert_eq!(texture_size(egui::Vec2::ZERO, 1.0, 8192), [1, 1]);
        assert_eq!(
            texture_size(egui::vec2(4000.0, 2000.0), 1.0, 1024),
            [1024, 512]
        );
    }
}

#[cfg(test)]
mod gpu_tests {
    use super::*;
    use egui_wgpu::CallbackTrait;
    use std::future::Future;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    fn block_on<T>(future: impl Future<Output = T>) -> T {
        struct ThreadWake(std::thread::Thread);
        impl Wake for ThreadWake {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
        let mut context = Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);
        loop {
            if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
                return value;
            }
            std::thread::park();
        }
    }

    #[test]
    fn wgsl_is_valid() -> Result<(), Box<dyn std::error::Error>> {
        let module = wgpu::naga::front::wgsl::parse_str(SHADER)?;
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::empty(),
        )
        .validate(&module)?;
        Ok(())
    }

    #[test]
    #[ignore = "requires a Windows OpenGL GPU; run explicitly"]
    fn opengl_render_resize_and_release() -> Result<(), Box<dyn std::error::Error>> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::GL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = block_on(instance.request_adapter(&Default::default()))?;
        let (device, queue) = block_on(adapter.request_device(&Default::default()))?;
        let errors = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut resources = egui_wgpu::CallbackResources::default();
        for (width, height, dpi, frost, squircle) in [
            (1280, 800, 1.0, 0.65, true),
            (1280, 800, 1.0, 0.65, true),
            (720, 480, 2.0, 0.0, false),
            (1920, 1080, 1.5, 1.0, true),
        ] {
            let size = egui::vec2(width as f32 / dpi, height as f32 / dpi);
            let callback = GlassCallback {
                rect: egui::Rect::from_min_size(egui::Pos2::ZERO, size),
                background: Arc::new(DesktopFrame::test_pattern(texture_size(size, dpi, 2048))),
                settings: Settings {
                    frost,
                    squircle,
                    ..Default::default()
                },
                format: OFFSCREEN_FORMAT,
            };
            assert_eq!(callback.uniforms(dpi).len(), UNIFORM_SIZE as usize);
            let mut encoder = device.create_command_encoder(&Default::default());
            callback.prepare(
                &device,
                &queue,
                &egui_wgpu::ScreenDescriptor {
                    size_in_pixels: [width, height],
                    pixels_per_point: dpi,
                },
                &mut encoder,
                &mut resources,
            );
            let output = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Glass smoke output"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: OFFSCREEN_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let renderer = resources.get::<Resources>().ok_or("missing resources")?;
            let targets = renderer.targets.as_ref().ok_or("missing targets")?;
            renderer.offscreen(
                &mut encoder,
                &output.create_view(&Default::default()),
                &renderer.glass,
                Some(&targets.glass_binding),
            );
            let row_bytes = (width * 4).div_ceil(256) * 256;
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Glass readback"),
                size: u64::from(row_bytes * height),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                output.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row_bytes),
                        rows_per_image: None,
                    },
                },
                output.size(),
            );
            queue.submit([encoder.finish()]);
            let (sender, receiver) = std::sync::mpsc::channel();
            readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let _ = sender.send(result);
                });
            device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(10)),
            })?;
            receiver.recv_timeout(std::time::Duration::from_secs(10))??;
            let mapped = readback.slice(..).get_mapped_range()?;
            let pixels: Vec<u8> = mapped
                .chunks_exact(row_bytes as usize)
                .flat_map(|row| row[..(width * 4) as usize].iter().copied())
                .collect();
            assert!(pixels.chunks_exact(4).all(|pixel| pixel[3] == 255));
            let minimum = pixels
                .chunks_exact(4)
                .map(|pixel| pixel[1])
                .min()
                .ok_or("empty output")?;
            let maximum = pixels
                .chunks_exact(4)
                .map(|pixel| pixel[1])
                .max()
                .ok_or("empty output")?;
            assert!(
                maximum > minimum + 20,
                "glass output must contain the lit scene"
            );
            if width == 1280
                && let Some(path) = std::env::var_os("LIQUID_GLASS_PREVIEW")
            {
                std::fs::write(path, &pixels)?;
            }
            drop(mapped);
            readback.unmap();
        }
        assert!(resources.remove::<Resources>().is_some());
        if let Some(error) = block_on(errors.pop()) {
            return Err(error.into());
        }
        Ok(())
    }
}
