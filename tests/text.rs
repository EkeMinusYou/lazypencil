#![cfg(feature = "rendering")]

use lazypencil::{
    document::hash,
    text::{DEFAULT_FONT_BYTES, DEFAULT_FONT_FAMILY, Fonts, TextSpec},
};
use serde_json::Value;

#[test]
fn the_bundled_font_matches_its_pinned_manifest_and_loads_without_system_installation() {
    let manifest: Value =
        serde_json::from_str(include_str!("../resources/fonts/manifest.json")).unwrap();
    let file = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["file"] == "NotoSansJP.ttf")
        .unwrap();
    assert_eq!(hash(DEFAULT_FONT_BYTES), file["sha256"]);
    let mut fonts = Fonts::new(&[]).unwrap();
    let inventory = fonts.inventory().unwrap();
    assert_eq!(inventory[0].family, DEFAULT_FONT_FAMILY);
    assert_eq!(inventory[0].source, "bundled");
}

#[test]
fn captured_font_bytes_can_be_used_without_reading_the_source_path_again() {
    use std::sync::Arc;
    let sources = vec![(
        std::path::PathBuf::from("/nonexistent/captured-font.ttf"),
        Arc::<[u8]>::from(lazypencil::text::DEFAULT_FONT_BYTES),
    )];
    let mut fonts = lazypencil::text::Fonts::from_sources(&sources).unwrap();
    let block = fonts
        .layout(
            &lazypencil::text::TextSpec {
                content: "日本語 Snapshot".into(),
                ..Default::default()
            },
            None,
        )
        .unwrap();
    assert!(block.metrics.width > 0.0);
    assert_eq!(
        block.metrics.fonts[0].source,
        "/nonexistent/captured-font.ttf"
    );
    assert_eq!(
        block.metrics.fonts[0].hash,
        lazypencil::document::hash(lazypencil::text::DEFAULT_FONT_BYTES)
    );
}

#[test]
fn japanese_wrapping_uses_the_same_paragraph_metrics_that_are_painted() {
    let mut fonts = Fonts::new(&[]).unwrap();
    let spec = TextSpec {
        content: "日本語と English を組み合わせた文章の折り返しを確認します。".into(),
        size: 20.0,
        line_height: Some(1.5),
        ..Default::default()
    };
    let wide = fonts.layout(&spec, Some(600.0)).unwrap();
    let narrow = fonts.layout(&spec, Some(180.0)).unwrap();
    assert!(narrow.metrics.line_count > wide.metrics.line_count);
    assert!(narrow.metrics.height > wide.metrics.height);
    assert_eq!(narrow.metrics.height, narrow.paragraph.height());
    assert!(narrow.metrics.longest_line <= 181.0);
    assert!(narrow.metrics.baseline > 0.0);
    assert_eq!(narrow.metrics.fonts[0].family, DEFAULT_FONT_FAMILY);
}

#[test]
fn missing_fonts_and_missing_glyphs_are_reported_instead_of_silently_substituted() {
    let mut fonts = Fonts::new(&[]).unwrap();
    let spec = TextSpec {
        content: "Hello".into(),
        family: "Lazypencil Definitely Missing Font 8db4e".into(),
        ..Default::default()
    };
    assert_eq!(
        fonts.layout(&spec, None).err().unwrap().code,
        "ASSET_NOT_FOUND"
    );
    let spec = TextSpec {
        content: "\u{10ffff}".into(),
        ..Default::default()
    };
    assert_eq!(
        fonts.layout(&spec, None).err().unwrap().code,
        "ASSET_NOT_FOUND"
    );
}

#[test]
fn variable_font_weights_change_the_actual_rendered_pixels() {
    fn render(fonts: &mut Fonts, weight: i32) -> Vec<u8> {
        let spec = TextSpec {
            content: "Hello 日本語".into(),
            weight,
            size: 32.0,
            ..Default::default()
        };
        let block = fonts.layout(&spec, Some(320.0)).unwrap();
        let mut surface = skia_safe::surfaces::raster_n32_premul((360, 100)).unwrap();
        surface.canvas().clear(skia_safe::Color::WHITE);
        block.paragraph.paint(surface.canvas(), (16.0, 16.0));
        skia_safe::png_encoder::encode_image(None, &surface.image_snapshot(), &Default::default())
            .unwrap()
            .as_bytes()
            .to_vec()
    }
    let mut fonts = Fonts::new(&[]).unwrap();
    assert_ne!(
        hash(&render(&mut fonts, 400)),
        hash(&render(&mut fonts, 700))
    );
}
