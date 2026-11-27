void main() {
    outputColor = vec4(sampleSgsr1(inputImage, gl_FragCoord.xy / outputSize).rgb, 1.0);
}
