//! MIT independent solid RGB vectors. No photographic pixels or vendor code.
use lytronaut_format::{
    DecodedJpeg, FormatError, Geometry,
    live_photo::{Builder, Plan, bundle},
};
const ID: &str = "A8A8A8A8-1111-4444-8888-121212121212";
fn geometry() -> Geometry {
    Geometry {
        width: 32,
        height: 32,
    }
}
fn image(rgb: [u8; 3]) -> DecodedJpeg {
    DecodedJpeg {
        geometry: geometry(),
        rgba: (0..1024)
            .flat_map(|_| [rgb[0], rgb[1], rgb[2], 255])
            .collect(),
    }
}
#[test]
fn selected_key_order_dimensions_identifier_cancel_and_complete_pair() {
    let plan = Plan::new(geometry(), &[0.0, 3.0, -2.0], 0).unwrap();
    assert_eq!(plan.planes(), vec![2, 0, 1]);
    assert_eq!(plan.geometry(), geometry());
    assert_eq!(plan.selected(), 0);
    assert_eq!(plan.key_seconds(), 1.0);
    assert!(Builder::new(plan.clone(), "bad identifier").is_err());
    let mut b = Builder::new(plan.clone(), ID).unwrap();
    assert_eq!(
        b.push(0, &image([255, 0, 0]), || true),
        Err(FormatError::Codec)
    );
    assert!(b.push(63, &image([255, 0, 0]), || false).is_err());
    for (i, rgb) in [(0, [255, 0, 0]), (1, [0, 255, 0]), (2, [0, 0, 255])] {
        b.push(i, &image(rgb), || false).unwrap();
    }
    assert!(b.push(0, &image([255, 0, 0]), || false).is_err());
    let pair = b.finish().unwrap();
    assert!(pair.jpeg.windows(ID.len()).any(|v| v == ID.as_bytes()));
    assert!(pair.movie.windows(ID.len()).any(|v| v == ID.as_bytes()));
    assert!(pair.movie.windows(5).any(|v| v == [0, 0, 0, 9, 0]));
    assert!(
        pair.movie
            .windows(b"com.apple.quicktime.still-image-time".len())
            .any(|v| v == b"com.apple.quicktime.still-image-time")
    );
    let zip = bundle(pair, b"{\"motion\":\"generated_focus_animation\"}").unwrap();
    assert!(zip.starts_with(b"PK\x03\x04"));
    for name in [b"focus.MOV".as_slice(), b"focus.JPG", b"provenance.json"] {
        assert!(zip.windows(name.len()).any(|v| v == name));
    }
    assert_eq!(&zip[zip.len() - 22..zip.len() - 18], b"PK\x05\x06");
}
#[test]
fn cropped_even_pixels_and_invalid_rgba_are_bounded() {
    let geometry = Geometry {
        width: 30,
        height: 18,
    };
    let plan = Plan::new(geometry, &[-1.0, 1.0], 1).unwrap();
    let mut builder = Builder::new(plan, ID).unwrap();
    let mut image = DecodedJpeg {
        geometry,
        rgba: vec![0; 30 * 18 * 4 - 1],
    };
    assert_eq!(builder.push(0, &image, || false), Err(FormatError::Extent));
    image.rgba = (0..30 * 18).flat_map(|_| [64, 128, 192, 255]).collect();
    builder.push(0, &image, || false).unwrap();
    builder.push(1, &image, || false).unwrap();
    let pair = builder.finish().unwrap();
    assert_eq!(
        lytronaut_format::jpeg_geometry(&pair.jpeg, Default::default()).unwrap(),
        geometry
    );
    assert!(pair.movie.len() < lytronaut_format::live_photo::MOVIE_CAP);
    assert_eq!(pair.plan.key_seconds(), 1.5);
}
#[test]
fn hostile_plan_and_incomplete_resources_fail_before_publication() {
    for lambdas in [
        vec![],
        vec![0.0],
        vec![0.0, 0.0],
        vec![0.0, f64::NAN],
        vec![0.0, f64::INFINITY],
        vec![0.0; 65],
    ] {
        assert!(Plan::new(geometry(), &lambdas, 0).is_err());
    }
    assert!(Plan::new(geometry(), &[0.0, 1.0], 2).is_err());
    for (w, h) in [(0, 32), (31, 32), (32, 31), (2050, 32), (2048, 2048)] {
        assert!(
            Plan::new(
                Geometry {
                    width: w,
                    height: h
                },
                &[0.0, 1.0],
                0
            )
            .is_err()
        );
    }
    let plan = Plan::new(geometry(), &[0.0, 1.0], 0).unwrap();
    assert!(Builder::new(plan.clone(), ID).unwrap().finish().is_err());
    let mut b = Builder::new(plan, ID).unwrap();
    b.push(0, &image([0, 0, 0]), || false).unwrap();
    assert!(b.finish().is_err());
}
#[test]
fn sweep_is_bounded_and_preserves_key_and_extrema() {
    let values: Vec<_> = (0..64).map(f64::from).collect();
    for selected in [0, 31, 63] {
        let p = Plan::new(
            Geometry {
                width: 2048,
                height: 1024,
            },
            &values,
            selected,
        )
        .unwrap();
        assert!(p.planes().len() < 64);
        assert!(p.planes().contains(&selected));
        assert_eq!(p.planes().first(), Some(&0));
        assert_eq!(p.planes().last(), Some(&63));
        assert!(p.planes().windows(2).all(|v| v[0] < v[1]));
        assert!(p.key_seconds() < 3.0);
    }
}
