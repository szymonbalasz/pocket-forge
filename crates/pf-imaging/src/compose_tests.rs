use super::*;

/// Deterministic xorshift for test data and random rects (no extra dependency).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u32) -> u32 {
        (self.next() % u64::from(n)) as u32
    }
}

fn noise_rgb(w: u32, h: u32, seed: u64) -> Arc<RgbImage> {
    let mut rng = Rng(seed);
    Arc::new(RgbImage::from_fn(w, h, |_, _| {
        let v = rng.next();
        image::Rgb([v as u8, (v >> 8) as u8, (v >> 16) as u8])
    }))
}

/// A noisy RGBA patch whose alpha ramps across, with a fully transparent band on the left.
fn noise_rgba(w: u32, h: u32, seed: u64) -> Arc<RgbaImage> {
    let mut rng = Rng(seed);
    Arc::new(RgbaImage::from_fn(w, h, |x, _| {
        let v = rng.next();
        let a = if x < w / 4 {
            0
        } else {
            (x * 255 / w.max(1)) as u8
        };
        image::Rgba([v as u8, (v >> 8) as u8, (v >> 16) as u8, a])
    }))
}

fn solid_rgba(w: u32, h: u32, px: [u8; 4]) -> Arc<RgbaImage> {
    Arc::new(RgbaImage::from_pixel(w, h, image::Rgba(px)))
}

fn layer(image: Arc<RgbaImage>, placement: Placement, opacity: f32) -> Layer {
    Layer {
        image,
        placement,
        opacity,
        visible: true,
    }
}

fn int_layer(image: Arc<RgbaImage>, x: i64, y: i64, opacity: f32) -> Layer {
    let (w, h) = image.dimensions();
    layer(image, Placement::at(x, y, w, h), opacity)
}

fn full(stack: &LayerStack) -> RgbImage {
    let (w, h) = stack.dimensions();
    compose_rect(stack, PixelRect::new(0, 0, w, h)).unwrap()
}

fn flatten_with(stack: &LayerStack, strip_h: u32) -> RgbImage {
    flatten_full::<ComposeError, _>(stack, strip_h, |_| Ok(())).unwrap()
}

/// Integer-placed layers: partly transparent, partly off the image, overlapping, with opacity.
fn integer_stack(base: &Arc<RgbImage>) -> LayerStack {
    let layers = [
        int_layer(noise_rgba(300, 200, 11), 40, 500, 1.0),
        int_layer(noise_rgba(120, 700, 12), -30, -50, 0.6),
        int_layer(solid_rgba(90, 90, [10, 200, 30, 255]), 380, 300, 1.0),
        int_layer(noise_rgba(250, 250, 13), 300, 1100, 0.8),
    ];
    LayerStack::new(base.clone(), &layers).unwrap()
}

#[test]
fn fr_exp_04_no_layers_equals_original() {
    let base = noise_rgb(257, 1100, 1);
    let stack = LayerStack::new(base.clone(), &[]).unwrap();
    assert_eq!(full(&stack), *base);
    assert_eq!(flatten_with(&stack, STRIP_H), *base);
}

#[test]
fn fr_exp_01_compose_rect_returns_rgb8_of_the_rect() {
    let base = noise_rgb(64, 48, 2);
    let stack = LayerStack::new(base.clone(), &[]).unwrap();
    let out = compose_rect(&stack, PixelRect::new(10, 5, 20, 7)).unwrap();
    assert_eq!(out.dimensions(), (20, 7));
    assert_eq!(out.get_pixel(0, 0), base.get_pixel(10, 5));
    assert_eq!(out.get_pixel(19, 6), base.get_pixel(29, 11));
}

#[test]
fn fr_exp_01_rejects_rects_outside_the_image() {
    let stack = LayerStack::new(noise_rgb(64, 48, 3), &[]).unwrap();
    for r in [
        PixelRect::new(0, 0, 0, 10),
        PixelRect::new(60, 0, 5, 10),
        PixelRect::new(0, 40, 10, 9),
        PixelRect::new(u32::MAX, 0, 2, 2),
    ] {
        assert!(
            matches!(
                compose_rect(&stack, r),
                Err(ComposeError::RectOutOfBounds { .. })
            ),
            "{r:?}"
        );
    }
}

#[test]
fn fr_exp_02_layers_draw_in_order_and_hidden_ones_are_skipped() {
    let base = noise_rgb(40, 40, 4);
    let red = int_layer(solid_rgba(10, 10, [255, 0, 0, 255]), 5, 5, 1.0);
    let blue = int_layer(solid_rgba(10, 10, [0, 0, 255, 255]), 10, 10, 1.0);
    let out = full(&LayerStack::new(base.clone(), &[red.clone(), blue.clone()]).unwrap());
    assert_eq!(out.get_pixel(6, 6).0, [255, 0, 0]);
    assert_eq!(
        out.get_pixel(12, 12).0,
        [0, 0, 255],
        "the later layer is on top"
    );

    let mut hidden = blue.clone();
    hidden.visible = false;
    let stack = LayerStack::new(base.clone(), &[red, hidden]).unwrap();
    assert_eq!(stack.drawn_layers(), 1);
    let out = full(&stack);
    assert_eq!(out.get_pixel(12, 12).0, [255, 0, 0]);
    assert_eq!(out.get_pixel(17, 17), base.get_pixel(17, 17));
}

#[test]
fn fr_exp_02_source_over_with_opacity() {
    let base = Arc::new(RgbImage::from_pixel(8, 8, image::Rgb([0, 0, 0])));
    // White at alpha 128, layer opacity 0.5 → about 64.
    let stack = LayerStack::new(
        base,
        &[int_layer(solid_rgba(8, 8, [255, 255, 255, 128]), 0, 0, 0.5)],
    )
    .unwrap();
    let px = full(&stack).get_pixel(3, 3).0;
    assert!((63..=65).contains(&px[0]), "{px:?}");
}

#[test]
fn fr_exp_03_detects_integer_placement() {
    let p = |cx, cy, scale, rotation| Placement {
        cx,
        cy,
        scale,
        rotation,
    };
    // 10×7 layer: an integer top-left needs cx = n + 5, cy = m + 3.5.
    let cases = [
        (p(5.0, 3.5, 1.0, 0.0), Some((0, 0))),
        (p(105.0, 53.5, 1.0, 0.0), Some((100, 50))),
        (p(-15.0, -6.5, 1.0, 0.0), Some((-20, -10))),
        (p(5.000_000_5, 3.5, 1.0, 0.0), Some((0, 0))),
        (p(5.000_01, 3.5, 1.0, 0.0), None),
        (p(5.5, 3.5, 1.0, 0.0), None),
        (p(5.0, 3.5, 1.0001, 0.0), None),
        (p(5.0, 3.5, 1.0, 15.0), None),
        (p(5.0, 3.5, 1.0, 360.0), Some((0, 0))),
    ];
    for (placement, want) in cases {
        assert_eq!(placement.integer_origin(10, 7), want, "{placement:?}");
    }
}

#[test]
fn fr_exp_03_integer_layer_uses_exact_pixel_sampling() {
    let base = noise_rgb(100, 80, 5);
    let patch = Arc::new(RgbaImage::from_fn(30, 20, |x, y| {
        image::Rgba([(x * 8) as u8, (y * 12) as u8, ((x + y) * 3) as u8, 255])
    }));
    let out =
        full(&LayerStack::new(base.clone(), &[int_layer(patch.clone(), 17, 23, 1.0)]).unwrap());
    for y in 0..20 {
        for x in 0..30 {
            let p = patch.get_pixel(x, y).0;
            assert_eq!(
                out.get_pixel(17 + x, 23 + y).0,
                [p[0], p[1], p[2]],
                "({x},{y})"
            );
        }
    }
    assert_eq!(out.get_pixel(16, 23), base.get_pixel(16, 23));
    assert_eq!(out.get_pixel(47, 43), base.get_pixel(47, 43));
}

#[test]
fn fr_exp_04_alpha_zero_pixels_equal_the_original() {
    let base = noise_rgb(400, 1300, 6);
    let stack = integer_stack(&base);
    let out = full(&stack);
    // Layer 0 (300×200 at 40,500) is transparent in its left quarter.
    for y in 500..700 {
        for x in 40..40 + 75 {
            let covered_by_other = x < 90 && y < 650; // layer 1 spans x -30..90, y -50..650
            if !covered_by_other {
                assert_eq!(out.get_pixel(x, y), base.get_pixel(x, y), "({x},{y})");
            }
        }
    }
    // Outside every layer.
    assert_eq!(out.get_pixel(399, 0), base.get_pixel(399, 0));
}

#[test]
fn fr_exp_04_compose_rect_equals_crop_of_full_flatten_for_200_random_rects() {
    let base = noise_rgb(400, 1300, 7);
    let stack = integer_stack(&base);
    let flat = flatten_with(&stack, STRIP_H);
    let mut rng = Rng(0x5eed);
    for _ in 0..200 {
        let x = rng.below(400);
        let y = rng.below(1300);
        let w = 1 + rng.below(400 - x);
        let h = 1 + rng.below((1300 - y).min(700));
        let got = compose_rect(&stack, PixelRect::new(x, y, w, h)).unwrap();
        let want = image::imageops::crop_imm(&flat, x, y, w, h).to_image();
        assert!(got == want, "rect {x},{y} {w}x{h} differs from the flatten");
    }
}

#[test]
fn fr_exp_04_integer_layers_are_independent_of_strip_height() {
    let base = noise_rgb(400, 1300, 8);
    let stack = integer_stack(&base);
    let reference = flatten_with(&stack, STRIP_H);
    for strip_h in [256, 1300, 7, 1] {
        assert!(
            flatten_with(&stack, strip_h) == reference,
            "strip height {strip_h}"
        );
    }
    assert!(full(&stack) == reference);
}

#[test]
fn fr_exp_04_integer_layers_exact_on_wide_images() {
    // Wider than tiny-skia's 8191 px single-pass limit: the tiled draw path must stay exact.
    let base = noise_rgb(9000, 3, 9);
    let patch = noise_rgba(8200, 2, 10);
    let stack = LayerStack::new(base.clone(), &[int_layer(patch.clone(), 400, 1, 1.0)]).unwrap();
    let out = full(&stack);
    for x in 0..8200 {
        let (p, b) = (patch.get_pixel(x, 0).0, base.get_pixel(400 + x, 1).0);
        let got = out.get_pixel(400 + x, 1).0;
        if p[3] == 0 {
            assert_eq!(got, b, "x {x}");
        } else if p[3] == 255 {
            assert_eq!(got, [p[0], p[1], p[2]], "x {x}");
        }
    }
    assert_eq!(out.get_pixel(8999, 0), base.get_pixel(8999, 0));
}

#[test]
fn fr_exp_05_strip_height_and_compose_version_are_pinned() {
    // Changing either changes output pixels: bump deliberately and regenerate the goldens.
    assert_eq!(STRIP_H, 512);
    assert_eq!(COMPOSE_VERSION, 1);
}

#[test]
fn fr_exp_05_flatten_strips_hands_out_512_row_strips_in_order() {
    let stack = LayerStack::new(noise_rgb(30, 1100, 11), &[]).unwrap();
    let mut seen = Vec::new();
    flatten_strips::<ComposeError, _>(&stack, STRIP_H, |s| {
        assert_eq!(s.rgb.len(), 30 * s.height as usize * 3);
        seen.push((s.y, s.height));
        Ok(())
    })
    .unwrap();
    assert_eq!(seen, [(0, 512), (512, 512), (1024, 76)]);
}

#[test]
fn fr_exp_20_sink_error_stops_between_strips() {
    #[derive(Debug, PartialEq)]
    enum E {
        Canceled,
        Compose(ComposeError),
    }
    impl From<ComposeError> for E {
        fn from(e: ComposeError) -> Self {
            E::Compose(e)
        }
    }
    let stack = LayerStack::new(noise_rgb(30, 1100, 12), &[]).unwrap();
    let mut strips = 0;
    let r = flatten_strips(&stack, STRIP_H, |_| {
        strips += 1;
        Err(E::Canceled)
    });
    assert_eq!(r, Err(E::Canceled));
    assert_eq!(strips, 1);
    let r = flatten_strips::<E, _>(&stack, 0, |_| Ok(()));
    assert_eq!(r, Err(E::Compose(ComposeError::InvalidStripHeight)));
}

#[test]
fn fr_exp_23_buffers_are_reserved_fallibly() {
    assert_eq!(try_vec(usize::MAX).unwrap_err(), ComposeError::OutOfMemory);
}

#[test]
fn fr_exp_03_affine_layer_is_resampled_about_its_centre() {
    let base = Arc::new(RgbImage::from_pixel(200, 200, image::Rgb([0, 0, 0])));
    let patch = solid_rgba(40, 20, [255, 255, 255, 255]);
    // Rotated 90° about (100, 100), scale 2: covers x 80..120, y 60..140.
    let p = Placement {
        cx: 100.0,
        cy: 100.0,
        scale: 2.0,
        rotation: 90.0,
    };
    let out = full(&LayerStack::new(base, &[layer(patch, p, 1.0)]).unwrap());
    assert_eq!(out.get_pixel(100, 100).0, [255, 255, 255]);
    assert_eq!(out.get_pixel(85, 65).0, [255, 255, 255]);
    assert_eq!(out.get_pixel(115, 135).0, [255, 255, 255]);
    assert_eq!(out.get_pixel(75, 100).0, [0, 0, 0]);
    assert_eq!(out.get_pixel(100, 55).0, [0, 0, 0]);
    assert_eq!(out.get_pixel(100, 145).0, [0, 0, 0]);
    // The edge is filtered, not a hard step.
    let edge = out.get_pixel(80, 100).0[0];
    assert!(edge > 0 && edge < 255, "edge {edge}");
}

#[test]
fn fr_exp_03_affine_layer_bounds_skip_strips_without_changing_output() {
    let base = noise_rgb(300, 1200, 13);
    let p = Placement {
        cx: 150.3,
        cy: 800.7,
        scale: 1.3,
        rotation: 15.0,
    };
    let stack = LayerStack::new(base.clone(), &[layer(noise_rgba(100, 80, 14), p, 0.8)]).unwrap();
    let out = flatten_with(&stack, STRIP_H);
    // The first strip is far from the layer and stays the original.
    let top = image::imageops::crop_imm(&out, 0, 0, 300, 512).to_image();
    assert!(top == image::imageops::crop_imm(&*base, 0, 0, 300, 512).to_image());
    assert_ne!(out.get_pixel(150, 800), base.get_pixel(150, 800));
}
