// Shader concepts adapted from liquidGL (NaughtyDuk) and LiquidGlass
// (Sepehr Kalanaki). See docs/dev/sections/CH3_GUI.md for sources and
// licenses/liquid-glass-notices.txt for MIT notices included in user packages.
struct Parameters {
    // Source pixels cover the monitor; viewport UVs move independently.
    viewport: vec4<f32>, // logical width, height, pixels/point, sRGB target
    optics: vec4<f32>,   // refraction, frost, dispersion, magnification
    surface: vec4<f32>,  // corner radius, squircle blend, lens depth fraction, dark tint
    source_rect: vec4<f32>, // monitor-normalized client origin and extent
};
@group(0) @binding(0) var<uniform> u: Parameters;
@group(1) @binding(0) var source: texture_2d<f32>;
@group(1) @binding(1) var frost: texture_2d<f32>;
@group(1) @binding(2) var linear_sampler: sampler;

struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};
@vertex fn vertex(@builtin(vertex_index) index: u32) -> Vertex {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var result: Vertex;
    result.position = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    result.uv = uv;
    return result;
}

fn sample_source(uv: vec2<f32>) -> vec4<f32> {
    let half_texel = 0.5 / vec2<f32>(textureDimensions(source));
    return textureSampleLevel(source, linear_sampler, clamp(uv, half_texel, 1.0 - half_texel), 0.0);
}

// Bilinear paired Gaussian taps from OverShifted's Blur.glsl (13-tap kernel).
fn gaussian(uv: vec2<f32>, direction: vec2<f32>) -> vec4<f32> {
    let step = direction * (0.35 + u.optics.y * 2.5) / vec2<f32>(textureDimensions(source));
    var color = sample_source(uv) * 0.1964825501511404;
    color += (sample_source(uv + step * 1.411764705882353) + sample_source(uv - step * 1.411764705882353)) * 0.2969069646728344;
    color += (sample_source(uv + step * 3.2941176470588234) + sample_source(uv - step * 3.2941176470588234)) * 0.09447039785044732;
    color += (sample_source(uv + step * 5.176470588235294) + sample_source(uv - step * 5.176470588235294)) * 0.010381362401148057;
    return color;
}
@fragment fn blur_horizontal(v: Vertex) -> @location(0) vec4<f32> {
    return gaussian(v.uv, vec2<f32>(1.0, 0.0));
}
@fragment fn blur_vertical(v: Vertex) -> @location(0) vec4<f32> {
    return gaussian(v.uv, vec2<f32>(0.0, 1.0));
}

fn distance_to_panel(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - half_size + radius;
    let outside = max(q, vec2<f32>(0.0));
    let rounded = length(outside) + min(max(q.x, q.y), 0.0) - radius;
    // n=4 superellipse at the corners; implicit f / |gradient(f)| near
    // its boundary. Keep the exact box distance on the straight edges.
    let t = outside / max(radius, 0.001);
    let t2 = t * t;
    let t3 = t2 * t;
    let implicit = dot(t2, t2) - 1.0;
    let squircle = radius * implicit / max(4.0 * length(t3), 0.001);
    let corner = select(rounded, squircle, min(q.x, q.y) > 0.0);
    return mix(rounded, corner, u.surface.y);
}

fn desktop_uv(uv: vec2<f32>) -> vec2<f32> {
    return u.source_rect.xy + uv * u.source_rect.zw;
}

fn glass_sample(view_uv: vec2<f32>) -> vec3<f32> {
    let uv = desktop_uv(view_uv);
    let half_texel = 0.5 / vec2<f32>(textureDimensions(frost));
    let safe_uv = clamp(uv, half_texel, 1.0 - half_texel);
    return mix(sample_source(safe_uv).rgb,
        textureSampleLevel(frost, linear_sampler, safe_uv, 0.0).rgb, u.optics.y);
}
fn to_linear(color: vec3<f32>) -> vec3<f32> {
    return select(color / 12.92, pow((color + 0.055) / 1.055, vec3<f32>(2.4)), color > vec3<f32>(0.04045));
}
@fragment fn glass(v: Vertex) -> @location(0) vec4<f32> {
    let half_size = u.viewport.xy * 0.5;
    let p = v.uv * u.viewport.xy - half_size;
    let radius = min(u.surface.x, min(half_size.x, half_size.y));
    let d = distance_to_panel(p, half_size, radius);
    let aa = max(1.0 / u.viewport.z, 0.35);
    let coverage = 1.0 - smoothstep(-aa, aa, d);
    let size = max(min(u.viewport.x, u.viewport.y), 1.0);
    let depth = max(u.surface.z * size, 1.0);
    let edge = 1.0 - clamp(max(-d, 0.0) / depth, 0.0, 1.0);
    // Circular sag curve: broad, curved lens rather than a thin panel bevel.
    let sag = 1.0 - sqrt(max(1.0 - edge * edge, 0.0));
    let direction = p / max(length(p), 0.0001);
    let offset = sag * direction * half_size * u.optics.x;
    let mapped = (half_size + p / u.optics.w - offset) / u.viewport.xy;
    let chroma = direction * u.optics.z * (0.25 + 0.75 * edge) / u.viewport.xy;
    var lens = vec3<f32>(glass_sample(mapped - chroma).r,
        glass_sample(mapped).g, glass_sample(mapped + chroma).b);
    lens = mix(lens, vec3<f32>(0.045, 0.06, 0.075), u.surface.w);
    let h = 0.5;
    let gradient = vec2<f32>(
        distance_to_panel(p + vec2<f32>(h, 0.0), half_size, radius) - distance_to_panel(p - vec2<f32>(h, 0.0), half_size, radius),
        distance_to_panel(p + vec2<f32>(0.0, h), half_size, radius) - distance_to_panel(p - vec2<f32>(0.0, h), half_size, radius));
    let normal = gradient / max(length(gradient), 0.0001);
    let light = max(dot(normal, normalize(vec2<f32>(-0.65, -0.76))), 0.0);
    let rim = exp(-abs(d) * 0.7);
    let fresnel = pow(edge, 5.0);
    lens += vec3<f32>(0.8, 0.9, 1.0) * (rim * (0.08 + light * 0.2) + fresnel * pow(light, 8.0) * 0.09);
    let noise = fract(sin(dot(floor(v.uv * u.viewport.xy * u.viewport.z), vec2<f32>(12.9898, 78.233))) * 43758.5453) - 0.5;
    lens += noise * 0.004;
    let color = clamp(mix(sample_source(desktop_uv(v.uv)).rgb, lens, coverage), vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(select(color, to_linear(color), u.viewport.w > 0.5), 1.0);
}
