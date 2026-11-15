use std::{borrow::Cow, rc::Rc};

use crate::{
    r#async::executor::{Background, Foreground},
    AssetProvider,
};

use super::*;

#[derive(Clone, Copy)]
pub struct Assets;

// Implement the AssetProvider trait here (required by App::new).
impl AssetProvider for Assets {
    fn get(&self, path: &str) -> Result<Cow<'_, [u8]>> {
        match path {
            "animated.webp" => Ok(Cow::Borrowed(include_bytes!("../test_data/animated.webp"))),
            "local.png" => Ok(Cow::Borrowed(include_bytes!("../test_data/local.png"))),
            "numbers-1000ms.gif" => Ok(Cow::Borrowed(include_bytes!(
                "../../facloud_ui/examples/assets/numbers-1000ms.gif"
            ))),
            _ => Err(anyhow!("no asset exists at path {}", path)),
        }
    }
}

fn new_asset_cache() -> AssetCache {
    AssetCache::new(
        Box::new(Assets),
        Foreground::test().into(),
        Background::default().into(),
    )
}

fn load_bundled_image(
    image_cache: &ImageCache,
    asset_cache: &AssetCache,
    path: &'static str,
    bounds: Vector2I,
    fit_type: FitType,
    animated_image_behavior: AnimatedImageBehavior,
) -> Rc<Image> {
    let image = image_cache.image(
        AssetSource::Bundled { path },
        bounds,
        fit_type,
        animated_image_behavior,
        CacheOption::BySize,
        None,
        asset_cache,
    );
    let AssetState::Loaded { data: image } = image else {
        panic!("Bundled asset should be available immediately!");
    };
    image
}

#[test]
fn test_passes_through_asset_cache_original() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();

    let source = AssetSource::Bundled { path: "local.png" };
    let image_asset: AssetState<ImageType> = asset_cache.load_asset(source.clone());
    let AssetState::Loaded { data: image } = image_asset else {
        panic!("Bundled asset should be available immediately!");
    };
    let ImageType::StaticBitmap { image } = image.as_ref() else {
        panic!("Expected static image but got dynamic image!");
    };
    let image_asset_weak = Arc::downgrade(image);

    let bounds = Vector2I::new(1024, 1024);
    let image = image_cache.image(
        source,
        bounds,
        FitType::Cover,
        AnimatedImageBehavior::FullAnimation,
        CacheOption::Original,
        None,
        &asset_cache,
    );
    let AssetState::Loaded { data: image } = image else {
        panic!("Bundled asset should be available immediately!");
    };
    let Image::Static(image) = image.as_ref() else {
        panic!("Expected static image but got dynamic image!");
    };

    // Assert that the image returned from the image cache and the asset stored
    // in the asset cache point to the same underlying data (i.e.: there were
    // no copies made).
    assert!(image_asset_weak.ptr_eq(&Arc::downgrade(image)));
}

#[test]
fn test_passes_through_asset_cache_original_when_target_size_matches_source_size() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();

    let source = AssetSource::Bundled { path: "local.png" };
    let image_asset: AssetState<ImageType> = asset_cache.load_asset(source.clone());
    let AssetState::Loaded { data: image } = image_asset else {
        panic!("Bundled asset should be available immediately!");
    };
    let ImageType::StaticBitmap { image } = image.as_ref() else {
        panic!("Expected static image but got dynamic image!");
    };
    let image_asset_weak = Arc::downgrade(image);

    // Load the image with `CacheOption::BySize` but use the source asset's
    // size as the bounds.
    let bounds = image.size();
    let image = image_cache.image(
        source,
        bounds,
        FitType::Cover,
        AnimatedImageBehavior::FullAnimation,
        CacheOption::BySize,
        None,
        &asset_cache,
    );
    let AssetState::Loaded { data: image } = image else {
        panic!("Bundled asset should be available immediately!");
    };
    let Image::Static(image) = image.as_ref() else {
        panic!("Expected static image but got dynamic image!");
    };

    // Assert that the image returned from the image cache and the asset stored
    // in the asset cache point to the same underlying data (i.e.: there were
    // no copies made).
    assert!(image_asset_weak.ptr_eq(&Arc::downgrade(image)));
}

#[test]
fn test_respects_max_dimensions_for_cacheoption_original() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();

    // We pass a very small value for bounds, which should get ignored due to
    // use of `CacheOption::Original`.
    let bounds = Vector2I::new(10, 10);

    let image = image_cache.image(
        AssetSource::Bundled { path: "local.png" },
        bounds,
        FitType::Cover,
        AnimatedImageBehavior::FullAnimation,
        CacheOption::Original,
        None,
        &asset_cache,
    );
    let AssetState::Loaded { data: image } = image else {
        panic!("Bundled asset should be available immediately!");
    };

    let Image::Static(image) = image.as_ref() else {
        panic!("Expected static image but got dynamic image!");
    };
    // Assert that the image, without resizing or a max dimension, matches our expectations.
    assert_eq!(image.img.dimensions(), (1024, 1024));

    let image = image_cache.image(
        AssetSource::Bundled { path: "local.png" },
        bounds,
        FitType::Cover,
        AnimatedImageBehavior::FullAnimation,
        CacheOption::Original,
        Some(512),
        &asset_cache,
    );
    let AssetState::Loaded { data: image } = image else {
        panic!("Bundled asset should be available immediately!");
    };

    let Image::Static(image) = image.as_ref() else {
        panic!("Expected static image but got dynamic image!");
    };
    // Assert that, when we specify a max dimension of 512, the image is resized accordingly.
    assert_eq!(image.img.dimensions(), (512, 512));
}

#[test]
fn test_first_frame_preview_returns_static_for_animated_gif() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();

    let image = load_bundled_image(
        &image_cache,
        &asset_cache,
        "numbers-1000ms.gif",
        Vector2I::new(16, 16),
        FitType::Contain,
        AnimatedImageBehavior::FirstFramePreview,
    );

    let Image::Static(image) = image.as_ref() else {
        panic!("Expected static image but got animated image!");
    };
    assert_eq!(image.img.dimensions(), (16, 16));
}

#[test]
fn test_first_frame_preview_keeps_full_animation_in_asset_cache() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();

    for path in ["numbers-1000ms.gif", "animated.webp"] {
        let image = load_bundled_image(
            &image_cache,
            &asset_cache,
            path,
            Vector2I::new(16, 16),
            FitType::Contain,
            AnimatedImageBehavior::FirstFramePreview,
        );

        assert!(matches!(image.as_ref(), Image::Static(_)));

        let asset: AssetState<ImageType> = asset_cache.load_asset(AssetSource::Bundled { path });
        let AssetState::Loaded { data } = asset else {
            panic!("Animated asset should be available immediately!");
        };
        assert!(matches!(data.as_ref(), ImageType::AnimatedBitmap { .. }));
    }
}

#[test]
fn test_first_frame_preview_returns_static_for_animated_webp() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();

    let image = load_bundled_image(
        &image_cache,
        &asset_cache,
        "animated.webp",
        Vector2I::new(16, 16),
        FitType::Contain,
        AnimatedImageBehavior::FirstFramePreview,
    );

    let Image::Static(image) = image.as_ref() else {
        panic!("Expected static image but got animated image!");
    };
    assert_eq!(image.img.dimensions(), (16, 16));
}

#[test]
fn test_full_animation_still_returns_animated_for_gif_and_webp() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();

    for path in ["numbers-1000ms.gif", "animated.webp"] {
        let image = load_bundled_image(
            &image_cache,
            &asset_cache,
            path,
            Vector2I::new(16, 16),
            FitType::Contain,
            AnimatedImageBehavior::FullAnimation,
        );

        let Image::Animated(image) = image.as_ref() else {
            panic!("Expected animated image but got static image!");
        };
        assert!(image.frames.len() > 1);
    }
}

#[test]
fn test_first_frame_preview_does_not_regress_static_formats() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();

    let image = load_bundled_image(
        &image_cache,
        &asset_cache,
        "local.png",
        Vector2I::new(16, 16),
        FitType::Contain,
        AnimatedImageBehavior::FirstFramePreview,
    );

    let Image::Static(image) = image.as_ref() else {
        panic!("Expected static image but got animated image!");
    };
    assert_eq!(image.img.dimensions(), (16, 16));
}

#[test]
fn test_preview_and_full_animation_requests_do_not_collide_in_rendered_image_cache() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();
    let bounds = Vector2I::new(16, 16);

    let preview = load_bundled_image(
        &image_cache,
        &asset_cache,
        "numbers-1000ms.gif",
        bounds,
        FitType::Contain,
        AnimatedImageBehavior::FirstFramePreview,
    );
    let full = load_bundled_image(
        &image_cache,
        &asset_cache,
        "numbers-1000ms.gif",
        bounds,
        FitType::Contain,
        AnimatedImageBehavior::FullAnimation,
    );
    let preview_again = load_bundled_image(
        &image_cache,
        &asset_cache,
        "numbers-1000ms.gif",
        bounds,
        FitType::Contain,
        AnimatedImageBehavior::FirstFramePreview,
    );
    let full_again = load_bundled_image(
        &image_cache,
        &asset_cache,
        "numbers-1000ms.gif",
        bounds,
        FitType::Contain,
        AnimatedImageBehavior::FullAnimation,
    );

    assert!(matches!(preview.as_ref(), Image::Static(_)));
    assert!(matches!(full.as_ref(), Image::Animated(_)));
    assert!(Rc::ptr_eq(&preview, &preview_again));
    assert!(Rc::ptr_eq(&full, &full_again));
    assert!(!Rc::ptr_eq(&preview, &full));
}

#[test]
fn test_svg_text_rasterizes_with_loaded_system_fonts() {
    let image_type = ImageType::try_from_bytes(
        br##"<svg width="160" height="40" viewBox="0 0 160 40" xmlns="http://www.w3.org/2000/svg">
  <text x="10" y="24" font-size="20" fill="#000000">Warp</text>
</svg>
"##,
    )
    .expect("SVG should parse");
    let ImageType::Svg { svg } = &image_type else {
        panic!("Expected SVG image type");
    };
    let font_family = svg
        .fontdb()
        .faces()
        .flat_map(|face| face.families.iter().map(|(family, _)| family.as_str()))
        .find(|family| {
            matches!(
                *family,
                "Arial"
                    | "Helvetica"
                    | "Inter"
                    | "DejaVu Sans"
                    | "Liberation Sans"
                    | "Noto Sans"
                    | "Cantarell"
                    | "Segoe UI"
            )
        })
        .or_else(|| {
            svg.fontdb()
                .faces()
                .find_map(|face| face.families.first().map(|(family, _)| family.as_str()))
        })
        .expect("System fonts should be loaded");

    let svg = format!(
        "<svg width=\"160\" height=\"40\" viewBox=\"0 0 160 40\" xmlns=\"http://www.w3.org/2000/svg\">\
  <text x=\"10\" y=\"24\" font-family=\"{font_family}\" font-size=\"20\" fill=\"#000000\">Warp</text>\
</svg>"
    );

    let image_type =
        ImageType::try_from_bytes(svg.as_bytes()).expect("SVG with installed font should parse");
    let image = image_type
        .to_image(
            Vector2I::new(160, 40),
            FitType::Contain,
            true,
            AnimatedImageBehavior::FullAnimation,
        )
        .expect("SVG should rasterize");
    let Image::Static(image) = image else {
        panic!("Expected static image");
    };

    assert!(image
        .rgba_bytes()
        .chunks_exact(4)
        .any(|pixel| pixel[3] != 0));
}

#[test]
fn test_evict_image_drops_arc_for_resized_bysize() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();
    let source = AssetSource::Bundled { path: "local.png" };

    // Request the image at a smaller size than its 1024x1024 source, which forces a resize
    // and allocates a fresh Arc<StaticImage> not shared with AssetCache.
    let bounds = Vector2I::new(64, 64);
    let weak = {
        let image = image_cache.image(
            source.clone(),
            bounds,
            FitType::Cover,
            AnimatedImageBehavior::FullAnimation,
            CacheOption::BySize,
            None,
            &asset_cache,
        );
        let AssetState::Loaded { data: image } = image else {
            panic!("Bundled asset should be available immediately!");
        };
        let Image::Static(arc) = image.as_ref() else {
            panic!("Expected static image!");
        };
        Arc::downgrade(arc)
        // The local Rc<Image> clone is dropped here; only ImageCache holds the entry now.
    };

    assert_eq!(
        weak.strong_count(),
        1,
        "ImageCache should be the sole strong holder after the caller drops its Rc clone"
    );

    // Evicting from ImageCache should make the Arc releasable by TextureCache.
    image_cache.evict_image(&source);
    assert_eq!(
        weak.strong_count(),
        0,
        "After evict_image, the resized Arc should have no strong holders (cascade invariant)"
    );
}

#[test]
fn test_evict_size_drops_arc_only_for_targeted_entry() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();
    let source = AssetSource::Bundled { path: "local.png" };

    // Cache the same asset at two distinct sizes.
    let small_bounds = Vector2I::new(32, 32);
    let large_bounds = Vector2I::new(256, 256);

    let weak_small = {
        let image = image_cache.image(
            source.clone(),
            small_bounds,
            FitType::Cover,
            AnimatedImageBehavior::FullAnimation,
            CacheOption::BySize,
            None,
            &asset_cache,
        );
        let AssetState::Loaded { data: image } = image else {
            panic!("Bundled asset should be available immediately!");
        };
        let Image::Static(arc) = image.as_ref() else {
            panic!("Expected static image!");
        };
        Arc::downgrade(arc)
    };

    let weak_large = {
        let image = image_cache.image(
            source.clone(),
            large_bounds,
            FitType::Cover,
            AnimatedImageBehavior::FullAnimation,
            CacheOption::BySize,
            None,
            &asset_cache,
        );
        let AssetState::Loaded { data: image } = image else {
            panic!("Bundled asset should be available immediately!");
        };
        let Image::Static(arc) = image.as_ref() else {
            panic!("Expected static image!");
        };
        Arc::downgrade(arc)
    };

    assert_eq!(weak_small.strong_count(), 1);
    assert_eq!(weak_large.strong_count(), 1);

    // Evict only the small size entry.
    image_cache.evict_size(&source, small_bounds, AnimatedImageBehavior::FullAnimation);

    assert_eq!(
        weak_small.strong_count(),
        0,
        "Small size Arc should have no strong holders after evict_size"
    );
    assert_eq!(
        weak_large.strong_count(),
        1,
        "Large size Arc should remain alive; only the small size was evicted"
    );
}

#[test]
fn test_svg_image_size_returns_intrinsic_dimensions() {
    let image_type = ImageType::try_from_bytes(
        br##"<svg width="160" height="40" viewBox="0 0 160 40" xmlns="http://www.w3.org/2000/svg"></svg>"##,
    )
    .expect("SVG should parse");

    assert_eq!(image_type.image_size(), Some(Vector2I::new(160, 40)));
}

#[test]
fn test_respects_max_dimensions_for_cacheoption_bysize() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();

    let bounds = Vector2I::new(768, 768);

    let image = image_cache.image(
        AssetSource::Bundled { path: "local.png" },
        bounds,
        FitType::Cover,
        AnimatedImageBehavior::FullAnimation,
        CacheOption::BySize,
        None,
        &asset_cache,
    );
    let AssetState::Loaded { data: image } = image else {
        panic!("Bundled asset should be available immediately!");
    };

    let Image::Static(image) = image.as_ref() else {
        panic!("Expected static image but got dynamic image!");
    };
    // Assert that the image gets resized to match the provided bounds.
    assert_eq!(image.img.dimensions(), (768, 768));

    let image = image_cache.image(
        AssetSource::Bundled { path: "local.png" },
        bounds,
        FitType::Cover,
        AnimatedImageBehavior::FullAnimation,
        CacheOption::BySize,
        Some(512),
        &asset_cache,
    );
    let AssetState::Loaded { data: image } = image else {
        panic!("Bundled asset should be available immediately!");
    };

    let Image::Static(image) = image.as_ref() else {
        panic!("Expected static image but got dynamic image!");
    };
    // Assert that, when we specify a max dimension of 512, the image is resized accordingly.
    assert_eq!(image.img.dimensions(), (512, 512));
}

#[test]
fn test_resize_dimensions_zero_and_header_u32_overflow_safe() {
    assert_eq!(resize_dimensions(0, 100, 64, 64, FitType::Contain), (64, 64));
    assert_eq!(resize_dimensions(100, 0, 64, 64, FitType::Cover), (64, 64));
    assert_eq!(resize_dimensions(100, 100, 0, 0, FitType::Contain), (1, 1));
    assert_eq!(resize_dimensions(100, 100, 0, 0, FitType::Stretch), (1, 1));

    // 65536 * 65536 * 4 overflows u32 to 0 if done in u32; must report full expected usize bytes.
    let err = CustomImageHeader::prepend_custom_header(vec![], 65536, 65536, CustomImageFormat::Rgba)
        .expect_err("empty buffer should mismatch 65536x65536 RGBA size");
    match err {
        CustomHeaderCreationError::ExpectedDataSizeMismatch {
            expected_bytes,
            actual_bytes,
        } => {
            assert_eq!(expected_bytes, 65536usize * 65536usize * 4usize);
            assert_eq!(actual_bytes, 0);
        }
    }
}

#[test]
fn test_svg_size_in_bytes_is_nonzero_and_lru_evicts_oldest_resized_image() {
    let svg_asset = ImageType::try_from_bytes(
        br##"<svg width="160" height="40" viewBox="0 0 160 40" xmlns="http://www.w3.org/2000/svg"></svg>"##,
    )
    .expect("SVG should parse");
    assert!(svg_asset.size_in_bytes() >= 160 * 40 * 4);

    let asset_cache = new_asset_cache();
    // Cap enough for two 32x32 RGBA images (2 * 4096 = 8192 bytes), so inserting a third evicts LRU.
    let image_cache = ImageCache::with_max_bytes(8192);
    let source = AssetSource::Bundled { path: "local.png" };

    let weak_1 = {
        let rc = load_bundled_image(
            &image_cache,
            &asset_cache,
            "local.png",
            Vector2I::new(32, 32),
            FitType::Stretch,
            AnimatedImageBehavior::FullAnimation,
        );
        let Image::Static(arc) = rc.as_ref() else {
            panic!("expected static");
        };
        Arc::downgrade(arc)
    };
    let weak_2 = {
        let rc = load_bundled_image(
            &image_cache,
            &asset_cache,
            "local.png",
            Vector2I::new(16, 64),
            FitType::Stretch,
            AnimatedImageBehavior::FullAnimation,
        );
        let Image::Static(arc) = rc.as_ref() else {
            panic!("expected static");
        };
        Arc::downgrade(arc)
    };
    // Touch weak_1 so weak_2 becomes least recently used.
    let _ = load_bundled_image(
        &image_cache,
        &asset_cache,
        "local.png",
        Vector2I::new(32, 32),
        FitType::Stretch,
        AnimatedImageBehavior::FullAnimation,
    );
    assert_eq!(weak_1.strong_count(), 1);
    assert_eq!(weak_2.strong_count(), 1);

    // Insert a third 32x32 (64x16) entry -> weak_2 should be evicted, weak_1 retained.
    let _ = image_cache.image(
        source,
        Vector2I::new(64, 16),
        FitType::Stretch,
        AnimatedImageBehavior::FullAnimation,
        CacheOption::BySize,
        None,
        &asset_cache,
    );
    assert_eq!(weak_1.strong_count(), 1);
    assert_eq!(weak_2.strong_count(), 0);
    assert!(image_cache.total_cached_bytes() <= 8192);
}

#[test]
fn test_local_file_asset_reloads_when_overwritten() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = dir.path().join("dynamic.txt");
    std::fs::write(&file_path, b"first").expect("write v1");

    let fg = Rc::new(Foreground::test());
    let bg = Arc::new(Background::default());
    let asset_cache = AssetCache::new(Box::new(Assets), fg.clone(), bg);
    let source = AssetSource::LocalFile {
        path: file_path.to_string_lossy().to_string(),
    };

    let state: AssetState<String> = asset_cache.load_asset(source.clone());
    let AssetState::Loading { handle } = state else {
        panic!("expected initial load to be async Loading");
    };
    futures::executor::block_on(fg.run(handle.when_loaded(&asset_cache).unwrap()));

    let AssetState::Loaded { data: v1 } = asset_cache.load_asset::<String>(source.clone()) else {
        panic!("expected Loaded v1");
    };
    assert_eq!(v1.as_str(), "first");

    // Overwrite file with different length so (mtime, len) definitely changes even on coarse filesystems.
    std::fs::write(&file_path, b"second-updated").expect("write v2");

    let state_v2: AssetState<String> = asset_cache.load_asset(source.clone());
    let AssetState::Loading { handle: handle_v2 } = state_v2 else {
        panic!("expected overwritten LocalFile to trigger reload");
    };
    futures::executor::block_on(fg.run(handle_v2.when_loaded(&asset_cache).unwrap()));

    let AssetState::Loaded { data: v2 } = asset_cache.load_asset::<String>(source) else {
        panic!("expected Loaded v2");
    };
    assert_eq!(v2.as_str(), "second-updated");
}

#[test]
fn test_asset_cache_unified_lru_evicts_local_file_and_retains_bundled() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p1 = dir.path().join("a.txt");
    let p2 = dir.path().join("b.txt");
    std::fs::write(&p1, b"1234567890").expect("write p1");
    std::fs::write(&p2, b"abcdefghij").expect("write p2");

    let fg = Rc::new(Foreground::test());
    let bg = Arc::new(Background::default());
    // Budget 15 bytes: fits one 10-byte LocalFile asset, not two.
    let asset_cache = AssetCache::with_max_cached_bytes(Box::new(Assets), fg.clone(), bg, 15);

    let s1 = AssetSource::LocalFile {
        path: p1.to_string_lossy().to_string(),
    };
    let s2 = AssetSource::LocalFile {
        path: p2.to_string_lossy().to_string(),
    };

    let AssetState::Loading { handle: h1 } = asset_cache.load_asset::<String>(s1.clone()) else {
        panic!("expected loading s1");
    };
    futures::executor::block_on(fg.run(h1.when_loaded(&asset_cache).unwrap()));
    let weak_1 = match asset_cache.load_asset::<String>(s1.clone()) {
        AssetState::Loaded { data } => Rc::downgrade(&data),
        _ => panic!("expected loaded s1"),
    };
    assert_eq!(weak_1.strong_count(), 1);

    let AssetState::Loading { handle: h2 } = asset_cache.load_asset::<String>(s2.clone()) else {
        panic!("expected loading s2");
    };
    futures::executor::block_on(fg.run(h2.when_loaded(&asset_cache).unwrap()));

    // s1 should have been LRU-evicted, and requesting s1 again cleanly reloads it!
    assert_eq!(weak_1.strong_count(), 0);
    assert!(matches!(
        asset_cache.load_asset::<String>(s1),
        AssetState::Loading { .. }
    ));
}
