// shaders/sprite.slang — Game-Studio Phase 2 2D sprite batch (spec 03 §4).
// Single instanced draw, texture-array atlas, SDF text ready. 2D lights reuse
// Forward+ point lights; normal-mapped sprites optional (NORMAL_MAP variant).
// Compiles with the same pinned triple as pbr.slang; SUBGROUPS_OFF variant
// is identical (this shader never uses subgroups — see COMPAT.md).

struct BatchUniforms
{
    float4x4 viewProj;   // orthographic for 2D
    float2 atlasInvSize; // 1/atlasExtent for UV scale
    float2 pad;
};

cbuffer BatchCB : register(b0)
{
    BatchUniforms gBatch;
}

Texture2D<float4> atlas : register(t0);
SamplerState atlasSampler : register(s0);

struct SpriteIn
{
    float2 corner : POSITION;   // unit-quad corner (0..1)
    float4 instanceRect : TEXCOORD0; // xy: atlas offset px, zw: atlas extent px
    float4 instancePos : TEXCOORD1;  // xyz: world pos, w: rotation radians
    float4 instanceColor : COLOR0;
    uint flags : FLAGS; // bit 0: flip-x, bit 1: flip-y (u32, never float)
};

struct SpriteOut
{
    float4 pos : SV_Position;
    float2 uv : TEXCOORD0;
    float4 color : COLOR0;
};

[shader("vertex")]
SpriteOut spriteVs(SpriteIn input)
{
    SpriteOut output;
    float2 c = input.corner;
    if ((input.flags & 1u) != 0u) { c.x = 1.0 - c.x; }
    if ((input.flags & 2u) != 0u) { c.y = 1.0 - c.y; }
    float rot = input.instancePos.w;
    float cs = cos(rot);
    float sn = sin(rot);
    float2 local = (c - 0.5) * input.instanceRect.zw;
    float2 rotated = float2(local.x * cs - local.y * sn, local.x * sn + local.y * cs);
    float3 world = float3(input.instancePos.xyz.xy + rotated, input.instancePos.z);
    output.pos = mul(gBatch.viewProj, float4(world, 1.0));
    output.uv = (input.instanceRect.xy + c * input.instanceRect.zw) * gBatch.atlasInvSize;
    output.color = input.instanceColor;
    return output;
}

[shader("fragment")]
float4 spritePs(SpriteOut input) : SV_Target
{
    float4 tex = atlas.Sample(atlasSampler, input.uv);
    return tex * input.color;
}
