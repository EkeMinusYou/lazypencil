#[cfg(feature = "rendering")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use lazypencil::text::{Fonts, TextSpec};
    use skia_safe::{Color, Paint, Rect};
    let output = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .ok_or("usage: cargo run --example render_text -- OUTPUT.png")?;
    let mut fonts = Fonts::new(&[])?;
    let mut surface = skia_safe::surfaces::raster_n32_premul((960, 640))
        .ok_or("cannot allocate raster surface")?;
    let canvas = surface.canvas();
    canvas.clear(Color::from_rgb(248, 250, 252));
    let mut paint = Paint::default();
    paint.set_color(Color::WHITE);
    paint.set_anti_alias(true);
    canvas.draw_round_rect(
        Rect::from_xywh(48.0, 48.0, 864.0, 544.0),
        16.0,
        16.0,
        &paint,
    );
    let heading = TextSpec {
        content: "lazypencil".into(),
        size: 40.0,
        weight: 700,
        color: Color::from_rgb(15, 23, 42),
        ..Default::default()
    };
    fonts
        .layout(&heading, Some(768.0))?
        .paragraph
        .paint(canvas, (96.0, 88.0));
    let subtitle = TextSpec {
        content: "Rust · Skia CPU · SkParagraph · Noto Sans JP".into(),
        size: 16.0,
        color: Color::from_rgb(71, 85, 105),
        ..Default::default()
    };
    fonts
        .layout(&subtitle, Some(768.0))?
        .paragraph
        .paint(canvas, (96.0, 154.0));
    let body=TextSpec {content:"日本語と English が混在する文章を、同梱フォントで描画しています。\n文字の計測と描画には、同じ段落の組版結果を使います。".into(),size:22.0,line_height:Some(1.7),color:Color::from_rgb(15,23,42),..Default::default()};
    fonts
        .layout(&body, Some(768.0))?
        .paragraph
        .paint(canvas, (96.0, 226.0));
    paint.set_color(Color::from_rgb(37, 99, 235));
    canvas.draw_round_rect(Rect::from_xywh(96.0, 440.0, 240.0, 52.0), 8.0, 8.0, &paint);
    let button = TextSpec {
        content: "デザインをはじめる".into(),
        size: 18.0,
        weight: 500,
        color: Color::WHITE,
        ..Default::default()
    };
    fonts
        .layout(&button, Some(208.0))?
        .paragraph
        .paint(canvas, (112.0, 450.0));
    let image = surface.image_snapshot();
    let encoded = skia_safe::png_encoder::encode_image(None, &image, &Default::default())
        .ok_or("cannot encode PNG")?;
    std::fs::write(&output, encoded.as_bytes())?;
    println!("{}", output.display());
    Ok(())
}

#[cfg(not(feature = "rendering"))]
fn main() {
    eprintln!("This example requires the rendering feature");
    std::process::exit(1);
}
