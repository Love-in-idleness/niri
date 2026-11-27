vec4 FsrRcasLoadF(ivec2 p) {
    return texelFetch(inputImage, clamp(p, ivec2(0), textureSize(inputImage, 0) - 1), 0);
}
void FsrRcasInputF(inout float r, inout float g, inout float b) {}
void main() {
    vec3 color;
    // Gamescope's default sharpness of 2 corresponds to 0.2 stops.
    FsrRcasF(color.r, color.g, color.b, uvec2(gl_FragCoord.xy), uvec4(floatBitsToUint(exp2(-0.2))));
    outputColor = vec4(color, 1.0);
}
