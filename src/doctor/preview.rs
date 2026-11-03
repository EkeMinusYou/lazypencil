use super::*;
#[cfg(feature = "tui")]
use std::io::IsTerminal;

pub(super) fn diagnose(report: &mut Report, options: &Options, json_output: bool) -> Result<()> {
    #[cfg(feature = "rendering")]
    let png = match test_image() {
        Ok(png) => {
            report.add("preview.renderer", Status::Ok,
                "Test image rendered and PNG round-trip pixels verified", None,
                json!({"width":128,"height":128,"verified_samples":5,"png_bytes":png.len(),"png_sha256":crate::document::hash(&png)}));
            png
        }
        Err(error) => {
            report.add("preview.renderer", Status::Error, "Cannot render or verify the test image",
                Some("Check the rendering build and bundled fonts; install a supported lazypencil build.".into()), json!({"error":error}));
            report.add(
                "preview.terminal",
                Status::NotChecked,
                "Image transport was not attempted because rendering failed",
                None,
                json!({"physical_visibility":"not_verified"}),
            );
            return Ok(());
        }
    };
    #[cfg(feature = "tui")]
    {
        if !json_output && std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
            match crate::tui::doctor::check_preview(&png, options.image_protocol) {
                Ok(details) => {
                    let status = match details["transport_status"].as_str() {
                        Some("accepted") => Status::Ok,
                        Some("rejected") => Status::Error,
                        _ => Status::Warning,
                    };
                    let message = match details["transport_status"].as_str() {
                        Some("accepted") => "Terminal acknowledged test-image upload and placement; physical visibility is not verified",
                        Some("rejected") => "Terminal rejected the test-image upload or placement",
                        _ => "Test image encoded and written; terminal display is unconfirmed because no receipt is available",
                    };
                    report.add("preview.terminal", status, message,
                        (details["transport_status"] != "accepted").then(|| "Try doctor --image-protocol kitty, sixel or halfblocks; terminal/multiplexer support may differ.".into()), details);
                }
                Err(error) if error.code == "CANCELLED" => return Err(error),
                Err(error) => report.add("preview.terminal", Status::Error,
                    "Cannot encode or transmit the terminal test image",
                    Some("Check terminal size and image-protocol support; try doctor --image-protocol halfblocks.".into()), json!({"error":error})),
            }
        } else {
            report.add("preview.terminal", Status::NotChecked,
                "Image transport is not tested in JSON output or without an interactive terminal",
                Some("Run lazypencil doctor in the terminal used for the TUI; the test is automatic and never asks for confirmation.".into()),
                json!({"physical_visibility":"not_verified","interactive_test":false}));
        }
    }
    #[cfg(all(feature = "rendering", not(feature = "tui")))]
    {
        let _ = (png, options, json_output);
        report.add(
            "preview.terminal",
            Status::NotChecked,
            "This build has no terminal preview support",
            None,
            json!({"tui_compiled":false}),
        );
    }
    #[cfg(not(feature = "rendering"))]
    {
        let _ = (options, json_output);
        report.add(
            "preview.renderer",
            Status::NotChecked,
            "This build has no image renderer",
            None,
            json!({"rendering_compiled":false}),
        );
        report.add(
            "preview.terminal",
            Status::NotChecked,
            "This build has no terminal preview support",
            None,
            json!({"tui_compiled":false,"physical_visibility":"not_verified"}),
        );
    }
    Ok(())
}

#[cfg(feature = "rendering")]
fn test_image() -> Result<Vec<u8>> {
    use crate::{
        document::Document,
        project::Project,
        render::{self, RenderOptions},
        text::Fonts,
    };
    use skia_safe::{Color, Data, Image};
    let document = Document::parse(
        json!({"version":"2.21","children":[{
        "id":"test","type":"frame","width":128,"height":128,"layout":"none","children":[
            {"id":"red","type":"rectangle","width":64,"height":64,"fill":"#FF0000"},
            {"id":"green","type":"rectangle","x":64,"width":64,"height":64,"fill":"#00FF00"},
            {"id":"blue","type":"rectangle","y":64,"width":64,"height":64,"fill":"#0000FF"},
            {"id":"white","type":"rectangle","x":64,"y":64,"width":64,"height":64,"fill":"#FFFFFF"},
            {"id":"circle","type":"ellipse","x":52,"y":52,"width":24,"height":24,"fill":"#000000"}
        ]}]})
        .to_string(),
    )?;
    let project = Project::local(document)?;
    let snapshot = crate::layout::compute(project.resolve()?, &mut Fonts::new(&[])?)?;
    let rendered = render::png(&project, &snapshot, &RenderOptions::default())?;
    let image = Image::from_encoded(Data::new_copy(&rendered.png))
        .and_then(|image| image.make_raster_image(None, None))
        .ok_or_else(|| Error::new("RENDER_ERROR", "Cannot decode the test PNG"))?;
    let pixels = image
        .peek_pixels()
        .ok_or_else(|| Error::new("RENDER_ERROR", "Cannot read back the test image"))?;
    if (rendered.width, rendered.height) != (128, 128)
        || ![
            (16, 16, Color::RED),
            (112, 16, Color::GREEN),
            (16, 112, Color::BLUE),
            (112, 112, Color::WHITE),
            (64, 64, Color::BLACK),
        ]
        .iter()
        .all(|(x, y, expected)| pixels.get_color((*x, *y)) == *expected)
    {
        return Err(Error::new(
            "RENDER_ERROR",
            "Rendered test-image pixels do not match the known pattern",
        ));
    }
    Ok(rendered.png)
}
