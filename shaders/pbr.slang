// shaders/pbr.slang — Game-Studio Phase 2 Forward+ PBR material (spec 03 §4).
// Authoring language: Slang. Runtime IR: WGSL (via `slangc -target wgsl`).
// Validator: Naga (in-wgpu tree). Triple pinned in flake.nix: slangc 2025.12.
//
// Lighting: metallic-roughness, Filament-derived BRDF. Equation references:
//   (F1) GGX normal distribution      D = a^2 / (pi * ((n.h)^2 (a^2-1) + 1)^2)
//   (F2) Smith-Schlick-GGX geometry   G = G1(l) * G1(v), G1(x) = n.x / ((n.x)(1-k)+k)
//   (F3) Schlick Fresnel              F = F0 + (1-F0)(1 - v.h)^5
//   (F4) Burley diffuse               f_d = albedo/pi * (1 + (F_D90-1)(1-n.l)^5)(1 + (F_D90-1)(1-n.v)^5)
// Ported from Filament/MIT reference; C++ backend NOT vendored (equations only).
// Precision policy (spec 03 §3.4, R-5): no fast-math; picking/packing IDs use
// uint, never float equality; mediump forbidden on desktop.

// Explicit 256-byte frame uniform block (bind group 0, binding 0).
struct FrameUniforms
{
    float4x4 viewProj;      // 64 B
    float4x4 lightViewProj; // 64 B (CSM split 0; splits 1-3 packed in LightGrid on desktop)
    float3 cameraPos;       // 12 B
    float time;             // 4 B  -> 16 B slot
    float4 sunDirAndIntensity; // 16 B
    float4 ambientSH0;      // 16 B (SH irradiance, band 0)
    float4 ambientSH1;      // 16 B
    float4 fogColorDensity; // 16 B
    uint4 pickBase;         // 16 B (u32 picking IDs; never float)
    float4 misc;            // 16 B -> total 256 B
};

cbuffer FrameCB : register(b0)
{
    FrameUniforms gFrame;
}

Texture2D<float4> albedoMap : register(t0);
Texture2D<float4> normalRoughMap : register(t1); // rgb: packed normal, a: roughness
SamplerState linearSampler : register(s0);

struct VsIn
{
    float3 position : POSITION;
    float3 normal : NORMAL;
    float2 uv : TEXCOORD0;
    float4 tangent : TANGENT;
    uint pickId : PICKID;
};

struct VsOut
{
    float4 pos : SV_Position;
    float3 worldPos : WORLDPOS;
    float3 normal : NORMAL;
    float2 uv : TEXCOORD0;
    nointerpolation uint pickId : PICKID;
};

// (F1) GGX normal distribution.
float ggxD(float roughness, float NoH)
{
    float a = roughness * roughness;
    float a2 = a * a;
    float denom = NoH * NoH * (a2 - 1.0) + 1.0;
    return a2 / (3.14159265 * denom * denom);
}

// (F2) Smith-Schlick-GGX geometry term.
float ggxG(float roughness, float NoV, float NoL)
{
    float k = (roughness + 1.0) * (roughness + 1.0) / 8.0;
    float gV = NoV / (NoV * (1.0 - k) + k);
    float gL = NoL / (NoL * (1.0 - k) + k);
    return gV * gL;
}

// (F3) Schlick Fresnel.
float3 schlickF(float3 f0, float VoH)
{
    float f = pow(1.0 - VoH, 5.0);
    return f0 + (1.0 - f0) * f;
}

[shader("vertex")]
VsOut vsMain(VsIn input)
{
    VsOut output;
    float4 world = float4(input.position, 1.0);
    output.pos = mul(gFrame.viewProj, world);
    output.worldPos = world.xyz;
    output.normal = input.normal;
    output.uv = input.uv;
    output.pickId = input.pickId;
    return output;
}

[shader("fragment")]
float4 psMain(VsOut input) : SV_Target
{
    float4 albedo = albedoMap.Sample(linearSampler, input.uv);
    float4 nr = normalRoughMap.Sample(linearSampler, input.uv);
    float3 N = normalize(input.normal);
    float roughness = clamp(nr.a, 0.04, 1.0);
    float metallic = 0.0; // dielectric default; metalness via ALPHA_BLEND variant push constant in v2

    float3 V = normalize(gFrame.cameraPos - input.worldPos);
    float3 L = normalize(gFrame.sunDirAndIntensity.xyz);
    float3 H = normalize(V + L);
    float NoL = clamp(dot(N, L), 0.0, 1.0);
    float NoV = clamp(dot(N, V), 0.0, 1.0);
    float NoH = clamp(dot(N, H), 0.0, 1.0);
    float VoH = clamp(dot(V, H), 0.0, 1.0);

    float3 f0 = lerp(float3(0.04, 0.04, 0.04), albedo.rgb, metallic);
    float D = ggxD(roughness, NoH);
    float G = ggxG(roughness, NoV, NoL);
    float3 F = schlickF(f0, VoH);
    float3 spec = (D * G) * F / max(4.0 * NoV * NoL, 0.001);

    // (F4) Burley diffuse approximation (single-bounce, no multi-scatter in v1).
    float fd90 = 0.5 + 2.0 * roughness * VoH * VoH;
    float fdl = 1.0 + (fd90 - 1.0) * pow(1.0 - NoL, 5.0);
    float fdv = 1.0 + (fd90 - 1.0) * pow(1.0 - NoV, 5.0);
    float3 diff = albedo.rgb * (fdl * fdv / 3.14159265) * (1.0 - metallic);

    float3 direct = (diff + spec) * NoL * gFrame.sunDirAndIntensity.w;
    float3 ambient = albedo.rgb * gFrame.ambientSH0.rgb + gFrame.ambientSH1.rgb * 0.25;
    float3 color = direct + ambient;
    // Fog: exp2, density in fogColorDensity.w.
    float dist = length(gFrame.cameraPos - input.worldPos);
    float fogF = 1.0 - exp(-gFrame.fogColorDensity.w * gFrame.fogColorDensity.w * dist * dist);
    color = lerp(color, gFrame.fogColorDensity.rgb, clamp(fogF, 0.0, 1.0));
    return float4(color, albedo.a);
}
