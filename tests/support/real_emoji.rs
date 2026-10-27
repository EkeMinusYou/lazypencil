use super::*;
use serde_json::json;
use skia_safe::Color;
use std::collections::BTreeSet;

#[test]
#[ignore = "requires pinned upstream fonts prepared by cargo xtask fixtures emoji on GHA"]
fn cbdt_preserves_composition_palettes_and_paragraph_pixels() {
    verify_format(0);
}

#[test]
#[ignore = "requires pinned upstream fonts prepared by cargo xtask fixtures emoji on GHA"]
fn colrv1_preserves_composition_palettes_and_paragraph_pixels() {
    verify_format(1);
}

fn verify_format(format_index: usize) {
    let input = PathBuf::from(std::env::var_os("LAZYPENCIL_TEST_EMOJI_DIR").unwrap());
    let output = PathBuf::from(std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR").unwrap());
    fs::create_dir_all(&output).unwrap();
    let source = fs::read(input.join("SOURCE.json")).unwrap();
    let pin: Value = serde_json::from_slice(&source).unwrap();
    let expected = fs::read(fixture("emoji-upstream.json")).unwrap();
    assert_eq!(pin, serde_json::from_slice::<Value>(&expected).unwrap());
    let license = fs::read(input.join("LICENSE")).unwrap();
    assert_eq!(hash(&license), pin["license"]["sha256"]);
    let mut reports = Vec::new();
    let mut stroke_reports = Vec::new();
    {
        let record = &pin["fonts"][format_index];
        let path = Path::new(record["asset"]["path"].as_str().unwrap());
        let font = input.join(path.file_name().unwrap());
        let bytes = fs::read(&font).unwrap();
        assert_eq!(hash(&bytes), record["asset"]["sha256"]);
        assert_eq!(
            bytes.len() as u64,
            record["asset"]["bytes"].as_u64().unwrap()
        );
        let mut fonts = Fonts::new(std::slice::from_ref(&font))
            .unwrap_or_else(|error| panic!("{} font loading: {error}", record["format"]));
        // Keep the stroke loader separate from the native reference. Its first
        // stroke is cold; subsequent geometry cases reuse identical font data.
        let mut stroke_fonts = Fonts::new(std::slice::from_ref(&font)).unwrap();
        for (case, content, family, expected_glyphs) in [
            ("smile", "😀", "Noto Color Emoji", 1),
            ("zwj", "👩‍💻", "Noto Color Emoji", 1),
            ("modifier", "👩🏻", "Noto Color Emoji", 1),
            ("selector", "❤️", "Noto Color Emoji", 1),
            ("flag", "🇯🇵", "Noto Color Emoji", 1),
            ("family", "👨‍👩‍👧‍👦", "Noto Color Emoji", 1),
            ("japanese", "日👩‍💻", DEFAULT_FONT_FAMILY, 2),
        ] {
            let mut request = spec(content, family);
            request.size = 100.0;
            request.color = Color::BLUE;
            let mut block = fonts
                .layout(&request, Some(360.0))
                .unwrap_or_else(|error| panic!("{} {case} shaping: {error}", record["format"]));
            let mut glyphs = Vec::new();
            block.paragraph.visit(|_, run| {
                if let Some(run) = run {
                    glyphs.extend_from_slice(run.glyphs());
                }
            });
            assert_eq!(
                glyphs.len(),
                expected_glyphs,
                "{} {case}: {glyphs:?}",
                record["format"]
            );
            assert!(glyphs.iter().all(|glyph| *glyph != 0));
            assert!(
                block
                    .metrics
                    .fonts
                    .iter()
                    .any(|used| used.hash == record["asset"]["sha256"])
            );
            assert!(block.metrics.intrinsic_width > 0.0 && block.metrics.height > 0.0);
            if case == "japanese" {
                assert!(
                    block
                        .metrics
                        .fonts
                        .iter()
                        .any(|used| used.hash == hash(DEFAULT_FONT_BYTES))
                );
            }
            let reference = paint_scene(&block);
            let reference_pixels = scene_pixels(&reference);
            let chromatic = reference_pixels
                .iter()
                .filter(|&&[r, g, b, a]| a > 200 && r.max(g).max(b) - r.min(g).min(b) > 30)
                .count();
            let colors = reference_pixels
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len();
            let intrinsic = reference_pixels
                .iter()
                .filter(|&&[r, _, b, a]| a > 200 && r.saturating_sub(b) > 30)
                .count();
            // This upstream revision deliberately draws the family ligature
            // with black, #BDBDBD, #E0E0E0 and white, rather than colored people.
            // See its immutable 2D/svg/emoji_u1f468_200d_1f469_200d_1f467_200d_1f466.svg.
            // Require the intrinsic gray palette; accepting the blue text paint
            // would hide a failure to preserve native font colors.
            let grayscale = (case == "family").then(|| {
                [0, 189, 224, 255].map(|gray| {
                    reference_pixels
                        .iter()
                        .filter(|&&pixel| pixel == [gray, gray, gray, 255])
                        .count()
                })
            });
            let reference_name = format!("codec-emoji-{format_index}-{case}-native.png");
            fs::write(output.join(&reference_name), &reference).unwrap();
            assert!(colors > 16, "{} {case}: colors={colors}", record["format"]);
            if let Some(grayscale) = grayscale {
                assert_eq!((chromatic, intrinsic), (0, 0));
                assert!(
                    grayscale.iter().all(|count| *count > 100),
                    "{} {case}: intrinsic gray palette={grayscale:?}",
                    record["format"]
                );
            } else {
                assert!(
                    chromatic > 100 && intrinsic > 100,
                    "{} {case}: chromatic={chromatic}, intrinsic={intrinsic}",
                    record["format"]
                );
            }
            for version in ["2.19", "2.21"] {
                let directory = tempfile::tempdir().unwrap();
                let (rendered, _) = color_fonts::render_scene(
                    directory.path(),
                    &font,
                    version,
                    json!({"fontFamily":family,"content":content}),
                    &Default::default(),
                );
                let name = format!("codec-emoji-{format_index}-{case}-{version}.png");
                fs::write(output.join(&name), &rendered.png).unwrap();
                let actual = scene_pixels(&rendered.png);
                let differences = actual
                    .iter()
                    .zip(&reference_pixels)
                    .filter(|(a, b)| a != b)
                    .count();
                reports.push(json!({"format":record["format"],"font_sha256":record["asset"]["sha256"],
                    "case":case,"content":content,"schema":version,"glyphs":glyphs,
                    "metrics":block.metrics,"chromatic_pixels":chromatic,"intrinsic_palette_pixels":intrinsic,"grayscale_palette_pixels":grayscale,"distinct_colors":colors,
                    "native_png":reference_name,"native_sha256":hash(&reference),
                    "rendered_png":name,"rendered_sha256":hash(&rendered.png),"different_pixels":differences}));
                fs::write(output.join(format!("codec-emoji-{format_index}-report.json")), serde_json::to_vec_pretty(&json!({
                    "source_commit":std::env::var("GITHUB_SHA").ok(),"upstream":pin,"cases":reports,
                })).unwrap()).unwrap();
                assert_eq!(
                    differences, 0,
                    "{} {case} {version}: native paragraph / ordinary render",
                    record["format"]
                );
            }
            if ["smile", "zwj", "japanese"].contains(&case) {
                for version in ["2.19", "2.21"] {
                    for alignment in ["inner", "center", "outer"] {
                        let directory = tempfile::tempdir().unwrap();
                        let (rendered, _) = color_fonts::render_scene_with_fonts(
                            directory.path(),
                            version,
                            json!({"fontFamily":family,"content":content,"fill":[],
                                "stroke":"#0000ff80","strokeWidth":3,"strokeAlignment":alignment}),
                            &Default::default(),
                            &mut stroke_fonts,
                        );
                        let pixels = scene_pixels(&rendered.png);
                        let mut ink = 0;
                        let mut outside = 0;
                        for y in 2..258 {
                            for x in 2..398 {
                                let color = pixels[y * 400 + x];
                                assert!(
                                    color[3] <= 128,
                                    "{} {case}/{version}/{alignment}: repeated stroke paint",
                                    record["format"]
                                );
                                if color[3] == 0 {
                                    continue;
                                }
                                assert_eq!(&color[..3], &[0, 0, 255]);
                                ink += 1;
                                outside += usize::from(reference_pixels[y * 400 + x][3] == 0);
                                let mut any_ink = false;
                                let mut all_solid = true;
                                for yy in y - 2..=y + 2 {
                                    for xx in x - 2..=x + 2 {
                                        any_ink |= reference_pixels[yy * 400 + xx][3] > 0;
                                        all_solid &= reference_pixels[yy * 400 + xx][3] == 255;
                                    }
                                }
                                if alignment == "inner" {
                                    assert!(
                                        any_ink,
                                        "{} {case}: inner stroke outside native ink at {x},{y}",
                                        record["format"]
                                    );
                                }
                                if alignment == "outer" {
                                    assert!(
                                        !all_solid,
                                        "{} {case}: outer stroke inside solid native ink at {x},{y}",
                                        record["format"]
                                    );
                                }
                            }
                        }
                        assert!(
                            ink > 100,
                            "{} {case}/{version}/{alignment}: missing stroke",
                            record["format"]
                        );
                        if alignment != "inner" {
                            assert!(
                                outside > 20,
                                "{} {case}/{version}/{alignment}: missing outside stroke",
                                record["format"]
                            );
                        }
                        let name = format!(
                            "codec-emoji-{format_index}-stroke-{case}-{version}-{alignment}.png"
                        );
                        fs::write(output.join(&name), &rendered.png).unwrap();
                        stroke_reports.push(json!({"format":record["format"],"font_sha256":record["asset"]["sha256"],"case":case,"content":content,"schema":version,"alignment":alignment,"png":name,"png_sha256":hash(&rendered.png),"native_png":reference_name,"native_sha256":hash(&reference),"ink_pixels":ink,"outside_native_pixels":outside}));
                        fs::write(output.join(format!("codec-emoji-{format_index}-stroke-report.json")), serde_json::to_vec_pretty(&json!({"source_commit":std::env::var("GITHUB_SHA").ok(),"upstream":pin,"cases":stroke_reports})).unwrap()).unwrap();
                    }
                }
            }
        }
    }
    assert_eq!(reports.len(), 14);
    assert_eq!(stroke_reports.len(), 18);
}
