use super::*;
use skia_safe::{Blender, RuntimeEffect, runtime_effect::ChildPtr};
use std::sync::OnceLock;

pub const MODES: [&str; 18] = [
    "normal",
    "darken",
    "multiply",
    "linearBurn",
    "colorBurn",
    "light",
    "screen",
    "linearDodge",
    "colorDodge",
    "overlay",
    "softLight",
    "hardLight",
    "difference",
    "exclusion",
    "hue",
    "saturation",
    "color",
    "luminosity",
];

// The overlap color is clamped before source-over composition. Skia's Plus mode
// instead adds alpha, so it is not the linear-dodge color blend for translucent input.
const LINEAR_BURN: &str = r#"
half4 main(half4 s, half4 d) {
    half overlap = s.a * d.a;
    half3 rgb = s.rgb * (1 - d.a) + d.rgb * (1 - s.a)
        + max(s.rgb * d.a + d.rgb * s.a - overlap, half3(0));
    return half4(rgb, s.a + d.a * (1 - s.a));
}
"#;
const LINEAR_DODGE: &str = r#"
half4 main(half4 s, half4 d) {
    half overlap = s.a * d.a;
    half3 rgb = s.rgb * (1 - d.a) + d.rgb * (1 - s.a)
        + min(s.rgb * d.a + d.rgb * s.a, half3(overlap));
    return half4(rgb, s.a + d.a * (1 - s.a));
}
"#;
const SHADOW_SOURCE: &str = r#"
uniform blender mode;
half4 main(half4 s, half4 d) {
    half4 mixed = mode.eval(s, d);
    return half4(clamp(mixed.rgb - d.rgb * (1 - s.a), half3(0), half3(s.a)), s.a);
}
"#;
const INNER_SHADOW: &str = r#"
uniform blender mode;
half4 main(half4 s, half4 d) {
    half4 mixed = mode.eval(s, d);
    return half4(clamp(mixed.rgb - s.rgb * (1 - d.a), half3(0), half3(d.a)), d.a);
}
"#;

thread_local! {
    static SHADOW_PROGRAM: std::result::Result<RuntimeEffect, String> = RuntimeEffect::make_for_blender(SHADOW_SOURCE, None);
    static INNER_PROGRAM: std::result::Result<RuntimeEffect, String> = RuntimeEffect::make_for_blender(INNER_SHADOW, None);
}

fn compilation_error(message: &str) -> Error {
    Error::new("RENDER_ERROR", "Cannot construct a native blend function")
        .details(json!({"reason":message}))
}

fn builtin(source: &str) -> std::result::Result<Blender, String> {
    RuntimeEffect::make_for_blender(source, None)?
        .make_blender(Data::new_copy(&[]), None)
        .ok_or_else(|| "Cannot instantiate the native blend function".into())
}

pub fn blend(value: &str) -> Result<Blender> {
    let mode = match value {
        "linearBurn" | "linearDodge" => {
            static BURN: OnceLock<std::result::Result<Blender, String>> = OnceLock::new();
            static DODGE: OnceLock<std::result::Result<Blender, String>> = OnceLock::new();
            let blender = if value == "linearBurn" {
                BURN.get_or_init(|| builtin(LINEAR_BURN))
            } else {
                DODGE.get_or_init(|| builtin(LINEAR_DODGE))
            };
            return blender
                .as_ref()
                .cloned()
                .map_err(|message| compilation_error(message));
        }
        "normal" => BlendMode::SrcOver,
        "darken" => BlendMode::Darken,
        "multiply" => BlendMode::Multiply,
        "colorBurn" => BlendMode::ColorBurn,
        "light" => BlendMode::Lighten,
        "screen" => BlendMode::Screen,
        "colorDodge" => BlendMode::ColorDodge,
        "overlay" => BlendMode::Overlay,
        "softLight" => BlendMode::SoftLight,
        "hardLight" => BlendMode::HardLight,
        "difference" => BlendMode::Difference,
        "exclusion" => BlendMode::Exclusion,
        "hue" => BlendMode::Hue,
        "saturation" => BlendMode::Saturation,
        "color" => BlendMode::Color,
        "luminosity" => BlendMode::Luminosity,
        _ => {
            let preview = value.chars().take(128).collect::<String>();
            return Err(Error::new(
                "UNSUPPORTED_FEATURE",
                format!("Unknown blend mode {preview:?}"),
            )
            .details(json!({"supported":MODES,"mode_bytes":value.len(),"mode_truncated":preview.len()!=value.len()})));
        }
    };
    Ok(mode.into())
}

pub(super) fn name(value: &Value) -> Result<&str> {
    match value.get("blendMode") {
        None => Ok("normal"),
        Some(Value::String(value)) => Ok(value),
        _ => Err(invalid("blendMode must be a string")),
    }
}

pub(super) fn at_node(mut error: Error, node: &ResolvedNode, property: &str) -> Error {
    let details = error.details.get_or_insert_with(|| json!({}));
    details["node_id"] = json!(node.id());
    details["property"] = json!(property);
    details["source_path"] = json!(node.property_source(property));
    error
}

pub(super) fn paint(
    value: &Value,
    node: &ResolvedNode,
    property: &str,
) -> Result<skia_safe::Blender> {
    name(value)
        .and_then(blend)
        .map_err(|error| at_node(error, node, property))
}

pub(super) fn shadow_type(effect: &Value) -> Result<&str> {
    match effect.get("shadowType") {
        None => Ok("outer"),
        Some(Value::String(kind)) if matches!(kind.as_str(), "inner" | "outer") => Ok(kind),
        _ => Err(invalid("shadowType must be inner or outer")),
    }
}

pub(super) fn outer_needs_background(effect: &Value) -> bool {
    effect["enabled"] != false
        && effect["type"] == "shadow"
        && effect["shadowType"] != "inner"
        && effect.get("blendMode").is_some_and(|mode| mode != "normal")
}

pub(super) fn shadow_source(mode: &str) -> Result<Blender> {
    if mode == "normal" {
        return Ok(BlendMode::Src.into());
    }
    let mode = blend(mode)?;
    SHADOW_PROGRAM.with(|effect| {
        effect
            .as_ref()
            .map_err(|message| compilation_error(message))?
            .make_blender(Data::new_copy(&[]), Some(&[ChildPtr::Blender(mode)][..]))
            .ok_or_else(|| compilation_error("Cannot instantiate the shadow source blend"))
    })
}

pub(super) fn inner_shadow(mode: &str) -> Result<Blender> {
    if mode == "normal" {
        return Ok(BlendMode::SrcATop.into());
    }
    let mode = blend(mode)?;
    INNER_PROGRAM.with(|effect| {
        effect
            .as_ref()
            .map_err(|message| compilation_error(message))?
            .make_blender(Data::new_copy(&[]), Some(&[ChildPtr::Blender(mode)][..]))
            .ok_or_else(|| compilation_error("Cannot instantiate the inner shadow blend"))
    })
}
