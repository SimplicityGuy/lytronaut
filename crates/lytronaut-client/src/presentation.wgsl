// Explicit viewer EXIF only; imported recipes may be baked and are not replayed.
@group(0) @binding(0) var image: texture_2d<f32>;
struct Parameters { orientation: u32, width: u32, height: u32, reserved: u32 }
@group(0) @binding(1) var<uniform> parameters: Parameters;
@vertex fn vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let positions = array<vec2<f32>, 3>(vec2(-1., -1.), vec2(3., -1.), vec2(-1., 3.));
    return vec4(positions[i], 0., 1.);
}
@fragment fn fragment(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let d = textureDimensions(image);
    var oriented = d;
    if parameters.orientation >= 5u { oriented = d.yx; }
    let x = u32(p.x) * oriented.x / parameters.width;
    let y = u32(p.y) * oriented.y / parameters.height;
    var source = vec2<u32>(x,y);
    switch parameters.orientation {
        case 2u: {source = vec2(d.x-1u-x,y);}
        case 3u: {source = vec2(d.x-1u-x,d.y-1u-y);}
        case 4u: {source = vec2(x,d.y-1u-y);}
        case 5u: {source = vec2(y,x);}
        case 6u: {source = vec2(y,d.y-1u-x);}
        case 7u: {source = vec2(d.x-1u-y,d.y-1u-x);}
        case 8u: {source = vec2(d.x-1u-y,x);}
        default: {}
    }
    return textureLoad(image, vec2<i32>(source), 0);
}
