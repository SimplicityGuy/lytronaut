//! MIT independently authored synthetic vectors; no captured photo or vendor code.
use lytronaut_format::{
    DecodedJpeg, Geometry,
    live_photo::{Builder, Plan, bundle},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let destination =
        std::path::PathBuf::from(arguments.next().ok_or("output directory required")?);
    // Test-only public geometry probes; the ordinary recipe keeps 320x240.
    // Plan validates all dimensions before any pixel allocation or output.
    let (geometry, prepared_still) = match (
        arguments.next(),
        arguments.next(),
        arguments.next(),
        arguments.next(),
    ) {
        (None, None, None, None) => (
            Geometry {
                width: 320,
                height: 240,
            },
            false,
        ),
        (Some(width), Some(height), mode, None)
            if mode.as_deref().is_none_or(|v| v == "--prepared-still") =>
        {
            (
                Geometry {
                    width: width.to_str().ok_or("UTF-8 width required")?.parse()?,
                    height: height.to_str().ok_or("UTF-8 height required")?.parse()?,
                },
                mode.is_some(),
            )
        }
        _ => {
            return Err(
                "expected output directory and optional width height [--prepared-still]".into(),
            );
        }
    };
    let plan = Plan::new(geometry, &[0.0, 3.0, -2.0], 0)?;
    std::fs::create_dir_all(&destination)?;
    let mut builder = Builder::new(plan.clone(), "A8A8A8A8-1111-4444-8888-121212121212")?;
    if prepared_still {
        // Explicit test-tool analogue of the browser's already-encoded selected
        // frame. This does not exercise or relax native JPEG working-set limits.
        let mut jpeg = Vec::new();
        let selected_rgb: Vec<u8> = (0..geometry.width * geometry.height)
            .flat_map(|_| [255, 0, 0])
            .collect();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90).encode(
            &selected_rgb,
            geometry.width,
            geometry.height,
            image::ExtendedColorType::Rgb8,
        )?;
        builder.set_still(&jpeg)?;
    }
    for (index, rgb) in [[255, 0, 0], [0, 255, 0], [0, 0, 255]]
        .into_iter()
        .enumerate()
    {
        let image = DecodedJpeg {
            geometry,
            rgba: (0..geometry.width * geometry.height)
                .flat_map(|_| [rgb[0], rgb[1], rgb[2], 255])
                .collect(),
        };
        builder.push(index, &image, || false)?;
    }
    let pair = builder.finish()?;
    let provenance = format!(
        "{{\"version\":1,\"fixture\":\"independent_mit_rgb\",\"motion\":\"generated_focus_animation\",\"key_seconds\":{},\"selected_rgb\":[255,0,0],\"playback_rgb\":[[0,0,255],[255,0,0],[0,255,0]],\"test_prepared_still\":{}}}",
        plan.key_seconds(),
        prepared_still
    );
    std::fs::write(destination.join("focus.JPG"), &pair.jpeg)?;
    std::fs::write(destination.join("focus.MOV"), &pair.movie)?;
    std::fs::write(destination.join("provenance.json"), &provenance)?;
    std::fs::write(
        destination.join("live-photo.zip"),
        bundle(pair, provenance.as_bytes())?,
    )?;
    println!(
        "independent synthetic Live Photo resources complete; library acceptance still required"
    );
    Ok(())
}
