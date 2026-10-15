use crate::profile_span;
use crate::render::graph::TargetMask;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderScene {
    SponzaLite,
    Platformer2D,
}

impl RenderScene {
    pub fn name(self) -> &'static str {
        match self {
            Self::SponzaLite => "sponza_lite",
            Self::Platformer2D => "2d_platformer",
        }
    }

    fn mode(self) -> u32 {
        match self {
            Self::SponzaLite => 0,
            Self::Platformer2D => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderHarnessOptions {
    pub width: u32,
    pub height: u32,
    pub no_fallback: bool,
}

/// Default headless harness extent (matches historical `HeadlessRenderer::new`).
pub const DEFAULT_HARNESS_WIDTH: u32 = 320;
/// Default headless harness extent (matches historical `HeadlessRenderer::new`).
pub const DEFAULT_HARNESS_HEIGHT: u32 = 180;
/// Upper bound for `--width`/`--height` (covers native 1440p + 4K harness runs).
pub const MAX_HARNESS_WIDTH: u32 = 3840;
/// Upper bound for `--width`/`--height` (covers native 1440p + 4K harness runs).
pub const MAX_HARNESS_HEIGHT: u32 = 2160;

impl Default for RenderHarnessOptions {
    fn default() -> Self {
        Self {
            width: DEFAULT_HARNESS_WIDTH,
            height: DEFAULT_HARNESS_HEIGHT,
            no_fallback: false,
        }
    }
}

impl RenderHarnessOptions {
    /// Validated constructor shared by the `render-test --width/--height/--no-fallback` path.
    pub fn new(width: u32, height: u32, no_fallback: bool) -> anyhow::Result<Self> {
        if width == 0 || width > MAX_HARNESS_WIDTH {
            anyhow::bail!("bad --width '{width}': want 1..={}", MAX_HARNESS_WIDTH);
        }
        if height == 0 || height > MAX_HARNESS_HEIGHT {
            anyhow::bail!("bad --height '{height}': want 1..={}", MAX_HARNESS_HEIGHT);
        }
        Ok(Self {
            width,
            height,
            no_fallback,
        })
    }

    /// Parse a `--width` flag value.
    pub fn parse_width(raw: &str) -> anyhow::Result<u32> {
        match raw.parse::<u32>() {
            Ok(width) if width > 0 && width <= MAX_HARNESS_WIDTH => Ok(width),
            _ => anyhow::bail!("bad --width '{raw}': want 1..={}", MAX_HARNESS_WIDTH),
        }
    }

    /// Parse a `--height` flag value.
    pub fn parse_height(raw: &str) -> anyhow::Result<u32> {
        match raw.parse::<u32>() {
            Ok(height) if height > 0 && height <= MAX_HARNESS_HEIGHT => Ok(height),
            _ => anyhow::bail!("bad --height '{raw}': want 1..={}", MAX_HARNESS_HEIGHT),
        }
    }

    /// Build options from `render-test` harness flags.
    ///
    /// `width`/`height` are the raw `--width`/`--height` values (None = default);
    /// `no_fallback` is whether `--no-fallback` was passed.
    pub fn from_harness_flags(
        width: Option<&str>,
        height: Option<&str>,
        no_fallback: bool,
    ) -> anyhow::Result<Self> {
        let width = match width {
            Some(raw) => Self::parse_width(raw)?,
            None => DEFAULT_HARNESS_WIDTH,
        };
        let height = match height {
            Some(raw) => Self::parse_height(raw)?,
            None => DEFAULT_HARNESS_HEIGHT,
        };
        Self::new(width, height, no_fallback)
    }

    /// Extent as `(width, height)`.
    pub fn dimensions(self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Value to pass as `force_fallback_adapter` when requesting a wgpu adapter.
    /// `--no-fallback` disables the software-fallback adapter.
    pub fn force_fallback_adapter(self) -> bool {
        !self.no_fallback
    }
}

#[derive(Debug, Clone)]
pub struct RenderFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub frame_index: u64,
    pub render_ms: f64,
}

const PROCEDURAL_WGSL: &str = r#"
struct Uniforms {
  mode: u32,
  frame: u32,
  width: u32,
  height: u32,
};
@group(0) @binding(0) var<uniform> u: Uniforms;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
  var pos = array<vec2<f32>, 3>(
    vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
  return vec4<f32>(pos[i], 0.0, 1.0);
}

fn hash21(p: vec2<f32>) -> f32 {
  var h = dot(p, vec2<f32>(127.1, 311.7));
  return fract(sin(h) * 43758.5453);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = pos.xy / vec2<f32>(f32(u.width), f32(u.height));
  let t = f32(u.frame) / 60.0;
  if (u.mode == 0u) {
    // sponza_lite proxy: warm atrium gradient + column grid + 256-light shimmer.
    var col = mix(vec3<f32>(0.10, 0.08, 0.07), vec3<f32>(0.55, 0.38, 0.24), uv.y);
    let grid = vec2<f32>(8.0, 6.0);
    let cell = floor(uv * grid);
    let center = (cell + 0.5) / grid;
    let d = length((uv - center) * vec2<f32>(1.6, 1.0));
    let shade = smoothstep(0.42, 0.18, d);
    let li = u32(cell.y * 8.0 + cell.x);
    let flicker = 0.75 + 0.25 * sin(t * 2.0 + f32(li) * 0.7);
    col = mix(col, vec3<f32>(0.72, 0.62, 0.50) * flicker, shade * 0.85);
    let grain = hash21(floor(pos.xy) + f32(u.frame));
    col += (grain - 0.5) * 0.02;
    return vec4<f32>(col, 1.0);
  } else {
    // 2d_platformer proxy: sky + tile checker + player quad + parallax.
    var col = mix(vec3<f32>(0.20, 0.42, 0.72), vec3<f32>(0.62, 0.83, 0.95), uv.y);
    let tile = vec2<f32>(20.0, 12.0);
    let cell = floor(uv * tile);
    let check = f32((u32(cell.x) + u32(cell.y)) % 2u);
    let ground = step(uv.y, 0.25);
    col = mix(col, mix(vec3<f32>(0.16, 0.30, 0.16), vec3<f32>(0.24, 0.44, 0.24), check), ground);
    let px = 0.5 + 0.25 * sin(t * 1.5);
    let in_player = step(abs(uv.x - px), 0.03) * step(abs(uv.y - 0.38), 0.05);
    col = mix(col, vec3<f32>(0.85, 0.25, 0.22), in_player);
    return vec4<f32>(col, 1.0);
  }
}
"#;

pub struct HeadlessRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    target: wgpu::Texture,
    target_view: wgpu::TextureView,
    width: u32,
    height: u32,
    subgroups_off: bool,
    no_fallback: bool,
}

impl HeadlessRenderer {
    pub fn new(target: TargetMask) -> anyhow::Result<Self> {
        Self::new_with_options(target, RenderHarnessOptions::default())
    }

    /// Harness constructor for the `render-test --width/--height/--no-fallback` path.
    pub fn new_with_options(
        target: TargetMask,
        options: RenderHarnessOptions,
    ) -> anyhow::Result<Self> {
        profile_span!("render");
        let (width, height) = options.dimensions();
        let subgroups_off = target.intersects(TargetMask::WEB);
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: options.force_fallback_adapter(),
        }))
        .ok_or_else(|| anyhow::anyhow!("no wgpu adapter (set WGPU_BACKEND=gl for software GL)"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("game-studio-headless"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                memory_hints: wgpu::MemoryHints::default(),
            },
            None,
        ))?;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("procedural"),
            source: wgpu::ShaderSource::Wgsl(PROCEDURAL_WGSL.into()),
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uniform-layout"),
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
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("uniforms"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("procedural-layout"),
            bind_group_layouts: &[&bind_layout],
            push_constant_ranges: &[],
        });
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("procedural"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("headless-target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let no_fallback = options.no_fallback;
        Ok(Self {
            device,
            queue,
            pipeline,
            uniform_buffer,
            bind_group,
            target,
            target_view,
            width,
            height,
            subgroups_off,
            no_fallback,
        })
    }

    pub fn subgroups_off(&self) -> bool {
        self.subgroups_off
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn no_fallback(&self) -> bool {
        self.no_fallback
    }

    pub fn harness_options(&self) -> RenderHarnessOptions {
        RenderHarnessOptions {
            width: self.width,
            height: self.height,
            no_fallback: self.no_fallback,
        }
    }

    pub fn render(&self, scene: RenderScene, frame_index: u64) -> anyhow::Result<RenderFrame> {
        profile_span!("render");
        let start = std::time::Instant::now();
        let uniforms: [u32; 4] = [scene.mode(), frame_index as u32, self.width, self.height];
        let bytes: [u8; 16] = bytemuck_cast_u32x4(uniforms);
        self.queue.write_buffer(&self.uniform_buffer, 0, &bytes);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("procedural-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        let padded_row = padded_bytes_per_row(self.width);
        let buffer_size = (padded_row as u64) * (self.height as u64);
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));
        let slice = readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        let _ = self.device.poll(wgpu::Maintain::Wait);
        receiver
            .recv()
            .map_err(|_| anyhow::anyhow!("map_async dropped"))?
            .map_err(|err| anyhow::anyhow!("map failed: {err:?}"))?;
        let data = slice.get_mapped_range().to_vec();
        readback.unmap();
        let mut rgba = Vec::with_capacity((self.width * self.height * 4) as usize);
        for row in 0..self.height as usize {
            let start = row * padded_row as usize;
            rgba.extend_from_slice(&data[start..start + (self.width as usize) * 4]);
        }
        let render_ms = start.elapsed().as_secs_f64() * 1000.0;
        Ok(RenderFrame {
            width: self.width,
            height: self.height,
            rgba,
            frame_index,
            render_ms,
        })
    }

    pub fn render_sequence(
        &self,
        scene: RenderScene,
        frames: u64,
    ) -> anyhow::Result<Vec<RenderFrame>> {
        let mut out = Vec::with_capacity(frames as usize);
        for index in 0..frames {
            out.push(self.render(scene, index)?);
        }
        Ok(out)
    }
}

fn bytemuck_cast_u32x4(values: [u32; 4]) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    for (slot, value) in values.iter().enumerate() {
        bytes[slot * 4..slot * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn padded_bytes_per_row(width: u32) -> u32 {
    let row = width * 4;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    ((row + align - 1) / align) * align
}

pub fn save_png(frame: &RenderFrame, path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let buffer = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(
        frame.width,
        frame.height,
        frame.rgba.clone(),
    )
    .ok_or_else(|| anyhow::anyhow!("bad frame dimensions"))?;
    buffer.save(path)?;
    Ok(())
}

pub fn load_rgba(path: &Path) -> anyhow::Result<(u32, u32, Vec<u8>)> {
    let image = image::open(path)?.to_rgba8();
    let (width, height) = (image.width(), image.height());
    Ok((width, height, image.into_raw()))
}

/// Hybrid Y-MSSIM + UV-RMS *difference* in [0, 1].
/// image-compare 0.4.2 returns a similarity (1 = identical, verified
/// empirically: same-vs-same scores 1.0, vs-black scores ~0.0), so the
/// reported difference is `1 - similarity`. Golden budget: < 0.005 (0.5%).
pub fn compare_frames(rendered: &RenderFrame, golden_path: &Path) -> anyhow::Result<f64> {
    let (width, height, golden) = load_rgba(golden_path)?;
    if width != rendered.width || height != rendered.height {
        anyhow::bail!(
            "golden size {width}x{height} != rendered {}x{}",
            rendered.width,
            rendered.height
        );
    }
    let rendered_rgb =
        image::RgbaImage::from_raw(rendered.width, rendered.height, rendered.rgba.clone())
            .ok_or_else(|| anyhow::anyhow!("bad rendered dimensions"))?;
    let golden_rgb = image::RgbaImage::from_raw(width, height, golden)
        .ok_or_else(|| anyhow::anyhow!("bad golden dimensions"))?;
    let rendered_rgb = image::DynamicImage::ImageRgba8(rendered_rgb).to_rgb8();
    let golden_rgb = image::DynamicImage::ImageRgba8(golden_rgb).to_rgb8();
    let result = image_compare::rgb_hybrid_compare(&rendered_rgb, &golden_rgb)
        .map_err(|err| anyhow::anyhow!("compare failed: {err:?}"))?;
    Ok(1.0 - result.score)
}

use std::path::Path;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn procedural_shader_is_naga_valid() {
        let module = naga::front::wgsl::parse_str(PROCEDURAL_WGSL).expect("procedural wgsl parses");
        assert!(!module.entry_points.is_empty());
    }

    #[test]
    fn row_padding_respects_copy_alignment() {
        assert_eq!(padded_bytes_per_row(320), 1280);
        assert_eq!(
            padded_bytes_per_row(1) % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT,
            0
        );
    }

    #[test]
    fn scenes_have_stable_names_and_modes() {
        assert_eq!(RenderScene::SponzaLite.name(), "sponza_lite");
        assert_eq!(RenderScene::Platformer2D.name(), "2d_platformer");
        assert_ne!(
            RenderScene::SponzaLite.mode(),
            RenderScene::Platformer2D.mode()
        );
    }

    #[test]
    fn hybrid_compare_scores_identical_frames_at_zero_diff() {
        // image-compare 0.4.2 returns a similarity (1 = identical); the
        // engine reports difference = 1 - similarity. Regression guard for
        // the inverted-gate bug found 2026-10-13.
        let frame = RenderFrame {
            width: 8,
            height: 8,
            rgba: vec![128u8; 8 * 8 * 4],
            frame_index: 0,
            render_ms: 0.0,
        };
        let path = std::env::temp_dir().join(format!(
            "game-studio-golden-{}.png",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ));
        save_png(&frame, &path).expect("save golden");
        let diff = compare_frames(&frame, &path).expect("compare");
        assert!(
            diff < 0.005,
            "identical frames must score diff < 0.5%, got {diff}"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn harness_options_default_matches_legacy_extent() {
        let options = RenderHarnessOptions::default();
        assert_eq!(options.width, DEFAULT_HARNESS_WIDTH);
        assert_eq!(options.height, DEFAULT_HARNESS_HEIGHT);
        assert_eq!((options.width, options.height), (320, 180));
        assert!(!options.no_fallback);
        assert!(options.force_fallback_adapter());
        assert_eq!(options.dimensions(), (320, 180));
    }

    #[test]
    fn harness_options_from_flags_round_trips() {
        let options = RenderHarnessOptions::from_harness_flags(Some("640"), Some("360"), true)
            .expect("valid flags");
        assert_eq!(options.dimensions(), (640, 360));
        assert!(options.no_fallback);
        assert!(!options.force_fallback_adapter());

        let defaults =
            RenderHarnessOptions::from_harness_flags(None, None, false).expect("defaults");
        assert_eq!(defaults, RenderHarnessOptions::default());
    }

    #[test]
    fn harness_options_rejects_bad_extents() {
        assert!(RenderHarnessOptions::new(0, 180, false).is_err());
        assert!(RenderHarnessOptions::new(320, 0, false).is_err());
        assert!(RenderHarnessOptions::parse_width("0").is_err());
        assert!(RenderHarnessOptions::parse_height("0").is_err());
        assert!(RenderHarnessOptions::parse_width("abc").is_err());
        assert!(RenderHarnessOptions::parse_height("-1").is_err());
        assert!(RenderHarnessOptions::from_harness_flags(Some("99999"), None, false).is_err());
        assert!(RenderHarnessOptions::from_harness_flags(None, Some("99999"), false).is_err());
    }
}
