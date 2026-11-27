vec4 FsrEasuRF(vec2 p) { return textureGather(inputImage, p, 0); }
vec4 FsrEasuGF(vec2 p) { return textureGather(inputImage, p, 1); }
vec4 FsrEasuBF(vec2 p) { return textureGather(inputImage, p, 2); }
void main() {
    vec2 sourceSize = vec2(textureSize(inputImage, 0));
    vec2 scale = sourceSize / outputSize;
    vec2 inv = 1.0 / sourceSize;
    uvec4 c0 = floatBitsToUint(vec4(scale, 0.5 * scale - 0.5));
    uvec4 c1 = floatBitsToUint(vec4(inv, inv * vec2(1.0, -1.0)));
    uvec4 c2 = floatBitsToUint(vec4(inv * vec2(-1.0, 2.0), inv * vec2(1.0, 2.0)));
    uvec4 c3 = floatBitsToUint(vec4(0.0, 4.0 * inv.y, 0.0, 0.0));
    vec3 color;
    FsrEasuF(color, uvec2(gl_FragCoord.xy), c0, c1, c2, c3);
    outputColor = vec4(color, 1.0);
}
