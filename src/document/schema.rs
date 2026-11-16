//! Public .pen property rules with explicitly documented interoperability cases.
//! https://docs.pencil.dev/for-developers/the-pen-format#typescript-schema
use super::*;

const ENTITY: &[&str] = &[
    "id",
    "type",
    "name",
    "context",
    "reusable",
    "theme",
    "enabled",
    "opacity",
    "flipX",
    "flipY",
    "layoutPosition",
    "metadata",
    "rotation",
    "x",
    "y",
];
const SIZE: &[&str] = &["width", "height"];
const STROKE: &[&str] = &[
    "stroke",
    "strokeWidth",
    "strokeAlignment",
    "strokeLinecap",
    "strokeLinejoin",
];
const LAYOUT: &[&str] = &[
    "layout",
    "gap",
    "layoutIncludeStroke",
    "padding",
    "justifyContent",
    "alignItems",
];
const TEXT: &[&str] = &[
    "fontFamily",
    "fontSize",
    "fontWeight",
    "letterSpacing",
    "fontStyle",
    "underline",
    "lineHeight",
    "textAlign",
    "textAlignVertical",
    "strikethrough",
    "href",
    "content",
    "textGrowth",
];

pub(super) fn known_property(kind: Option<&str>, key: &str) -> bool {
    // Refs and partial overrides are checked against their effective node type
    // after expansion. The public Ref schema has an open property index.
    let Some(kind) = kind.filter(|kind| *kind != "ref") else {
        return true;
    };
    ENTITY.contains(&key)
        || SIZE.contains(&key) && kind != "group"
        || key == "effect" && !matches!(kind, "note" | "script")
        || STROKE.contains(&key)
            && matches!(
                kind,
                "frame" | "rectangle" | "ellipse" | "polygon" | "path" | "text" | "browser"
            )
        || key == "fill"
            && matches!(
                kind,
                "frame" | "rectangle" | "ellipse" | "polygon" | "path" | "text" | "icon"
            )
        || key == "cornerRadius" && matches!(kind, "frame" | "rectangle" | "polygon" | "browser")
        || match kind {
            "frame" => {
                LAYOUT.contains(&key) || matches!(key, "children" | "clip" | "placeholder" | "slot")
            }
            "group" => key == "children",
            "ellipse" => matches!(key, "innerRadius" | "startAngle" | "sweepAngle"),
            "polygon" => key == "polygonCount",
            "path" => matches!(key, "geometry" | "viewBox" | "fillRule"),
            "text" => TEXT.contains(&key),
            "note" => matches!(key, "content" | "color"),
            "icon" => matches!(key, "library" | "icon" | "weight"),
            "script" => matches!(key, "scriptUri" | "inputs" | "clip"),
            "browser" => matches!(key, "url" | "deviceId" | "zoom" | "scrollX" | "scrollY"),
            _ => false,
        }
}

pub(super) fn validate(
    properties: &serde_json::Map<String, Value>,
    id: &str,
    authored: bool,
) -> Result<()> {
    let kind = properties.get("type").and_then(Value::as_str);
    if let Some(kind) = kind
        && !NODE_TYPES.contains(&kind)
    {
        return Err(invalid(format!(
            "{id}.type has unsupported node type {kind:?}"
        )));
    }
    if let Some(diagnostic) = node_property_diagnostics(properties, id).into_iter().next() {
        return Err(invalid(diagnostic.message).details(json!({"node_id":id})));
    }
    for (key, value) in properties {
        validate_property(key, value, id, authored).map_err(|mut error| {
            let details = error.details.get_or_insert_with(|| json!({}));
            details["node_id"] = json!(id);
            details["property"] = json!(key);
            error
        })?;
    }
    Ok(())
}

fn validate_property(key: &str, value: &Value, id: &str, authored: bool) -> Result<()> {
    let path = format!("{id}.{key}");
    let enumeration: Option<&[&str]> = match key {
        "layout" => Some(&["none", "horizontal", "vertical"]),
        "layoutPosition" => Some(&["auto", "absolute"]),
        "justifyContent" => Some(&["start", "center", "end", "space_between", "space_around"]),
        "alignItems" => Some(&["start", "center", "end"]),
        "textGrowth" => Some(&["auto", "fixed-width", "fixed-width-height"]),
        "textAlign" => Some(&["left", "center", "right", "justify"]),
        // Older Pencil documents retain this spelling; new edits use middle.
        "textAlignVertical" => Some(&["top", "middle", "center", "bottom"]),
        "strokeAlignment" => Some(&["inner", "center", "outer"]),
        "strokeLinecap" => Some(&["butt", "round", "square"]),
        "strokeLinejoin" => Some(&["miter", "bevel", "round"]),
        "fillRule" => Some(&["nonzero", "evenodd"]),
        _ => None,
    };
    if let Some(allowed) = enumeration {
        choice(value, &path, allowed)?;
    }
    match key {
        "width" | "height" => {
            if !value.as_f64().is_some_and(|value| value >= 0.0)
                && !value
                    .as_str()
                    .is_some_and(|value| (authored || !value.starts_with('$')) && valid_size(value))
            {
                return Err(invalid(format!(
                    "{path} must be a nonnegative dimension or sizing expression"
                )));
            }
        }
        "fill" | "stroke" => each(value, &path, |value, path| paint(value, path, authored))?,
        "effect" => each(value, &path, |value, path| effect(value, path, authored))?,
        "color" => color(value, &path, authored)?,
        "padding" => numbers(value, &path, &[2, 4], authored)?,
        "cornerRadius" => numbers(value, &path, &[4], authored)?,
        "viewBox" => array(value, &path, &[4], false)?,
        "strokeWidth" => {
            if value.is_object() {
                for (key, value) in fields(value, &path, &["top", "right", "bottom", "left"])? {
                    number(value, &format!("{path}.{key}"), authored)?;
                }
            } else {
                number(value, &path, authored)?;
            }
        }
        "slot" => {
            if value != &Value::Bool(false)
                && !value
                    .as_array()
                    .is_some_and(|values| values.iter().all(Value::is_string))
            {
                return Err(invalid(format!(
                    "{path} must be false or an array of component IDs"
                )));
            }
        }
        "metadata" => {
            if !value.is_object() {
                return Err(invalid(format!("{path} must be an object")));
            }
        }
        "x" | "y" | "rotation" | "opacity" | "fontSize" | "letterSpacing" | "lineHeight"
        | "gap" | "innerRadius" | "startAngle" | "sweepAngle" | "polygonCount" | "weight" => {
            number(value, &path, authored)?;
        }
        "zoom" | "scrollX" | "scrollY" => number(value, &path, false)?,
        "fontFamily" | "fontWeight" | "fontStyle" | "content" | "library" | "icon" | "name"
        | "context" | "href" | "scriptUri" | "url" | "deviceId" | "geometry" => {
            if !value.is_string() {
                return Err(invalid(format!("{path} must be a string")));
            }
        }
        "inputs" => {
            let inputs = value
                .as_object()
                .ok_or_else(|| invalid(format!("{path} must be an object")))?;
            for (name, value) in inputs {
                if !matches!(value, Value::String(_) | Value::Number(_) | Value::Bool(_)) {
                    return Err(invalid(format!(
                        "{path}.{name} must be a string, number, or boolean"
                    )));
                }
            }
        }
        _ => (),
    }
    Ok(())
}

pub(crate) fn valid_size(value: &str) -> bool {
    if value.starts_with('$') && value.len() > 1 {
        return true;
    }
    for prefix in ["fit_content", "fill_container"] {
        if value == prefix {
            return true;
        }
        if let Some(number) = value
            .strip_prefix(prefix)
            .and_then(|value| value.strip_prefix('('))
            .and_then(|value| value.strip_suffix(')'))
            && number
                .parse::<f64>()
                .is_ok_and(|number| number.is_finite() && number >= 0.0)
        {
            return true;
        }
    }
    false
}

fn number(value: &Value, path: &str, authored: bool) -> Result<()> {
    if !value.is_number()
        && !(authored
            && value
                .as_str()
                .is_some_and(|value| value.starts_with('$') && value.len() > 1))
    {
        return Err(invalid(format!(
            "{path} must be a number{}",
            if authored { " or variable" } else { "" }
        )));
    }
    Ok(())
}

fn numbers(value: &Value, path: &str, lengths: &[usize], authored: bool) -> Result<()> {
    if value.is_array() {
        array(value, path, lengths, authored)
    } else {
        number(value, path, authored)
    }
}

fn array(value: &Value, path: &str, lengths: &[usize], authored: bool) -> Result<()> {
    let values = value
        .as_array()
        .filter(|values| lengths.contains(&values.len()))
        .ok_or_else(|| {
            invalid(format!(
                "{path} must be a numeric array of length {lengths:?}"
            ))
        })?;
    for (index, value) in values.iter().enumerate() {
        number(value, &format!("{path}[{index}]"), authored)?;
    }
    Ok(())
}

fn each(value: &Value, path: &str, check: impl Fn(&Value, &str) -> Result<()>) -> Result<()> {
    if let Some(values) = value.as_array() {
        for (index, value) in values.iter().enumerate() {
            check(value, &format!("{path}[{index}]"))?;
        }
        Ok(())
    } else {
        check(value, path)
    }
}

fn choice(value: &Value, path: &str, allowed: &[&str]) -> Result<()> {
    if !value.as_str().is_some_and(|value| allowed.contains(&value)) {
        return Err(
            invalid(format!("{path} must be one of {}", allowed.join(", ")))
                .details(json!({"property":path,"allowed_values":allowed})),
        );
    }
    Ok(())
}

fn color(value: &Value, path: &str, authored: bool) -> Result<()> {
    if !value.as_str().is_some_and(|value| {
        authored && value.starts_with('$') && value.len() > 1
            || crate::color::hex_rgba(value).is_some()
    }) {
        return Err(invalid(format!(
            "{path} must be #RGB, #RRGGBB, or #RRGGBBAA{}",
            if authored { " or a color variable" } else { "" }
        )));
    }
    Ok(())
}

fn fields<'a>(
    value: &'a Value,
    path: &str,
    allowed: &[&str],
) -> Result<&'a serde_json::Map<String, Value>> {
    let properties = value
        .as_object()
        .ok_or_else(|| invalid(format!("{path} must be an object")))?;
    for key in properties.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(invalid(format!(
                "{path}.{key} is not in the public .pen schema"
            )));
        }
    }
    Ok(properties)
}

fn paint(value: &Value, path: &str, authored: bool) -> Result<()> {
    if value.is_string() {
        return color(value, path, authored);
    }
    if !value.get("type").is_some_and(Value::is_string) {
        return Err(invalid(format!(
            "{path} requires a paint type. Updating fill or stroke replaces the entire paint; retain its type, url and mode"
        )).details(json!({"property":path,"example":{"type":"image","url":"<asset URI returned by put_asset>","mode":"contain"}})));
    }
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let allowed: &[&str] = match kind {
        "color" => &["type", "enabled", "blendMode", "color"],
        "gradient" => &[
            "type",
            "enabled",
            "blendMode",
            "gradientType",
            "opacity",
            "center",
            "size",
            "rotation",
            "colors",
        ],
        "image" => &[
            "type",
            "enabled",
            "blendMode",
            "opacity",
            "url",
            "mode",
            "transform",
        ],
        "shader" => &["type", "enabled", "blendMode", "opacity", "url", "uniforms"],
        "mesh_gradient" => &[
            "type",
            "enabled",
            "blendMode",
            "opacity",
            "columns",
            "rows",
            "colors",
            "points",
        ],
        _ => {
            return Err(invalid(format!(
                "{path}.type has unsupported paint type {kind:?}"
            )));
        }
    };
    let properties = fields(value, path, allowed)?;
    blend(properties.get("blendMode"), path)?;
    for key in ["opacity", "rotation", "columns", "rows"] {
        if let Some(value) = properties.get(key) {
            number(value, &format!("{path}.{key}"), authored)?;
        }
    }
    for (key, allowed) in [
        ("center", &["x", "y"][..]),
        ("size", &["width", "height"][..]),
    ] {
        if let Some(value) = properties.get(key) {
            let path = format!("{path}.{key}");
            for (key, value) in fields(value, &path, allowed)? {
                number(value, &format!("{path}.{key}"), authored)?;
            }
        }
    }
    if let Some(value) = properties.get("transform") {
        array(value, &format!("{path}.transform"), &[6], false)?;
    }
    match kind {
        "color" => color(
            value.get("color").unwrap_or(&Value::Null),
            &format!("{path}.color"),
            authored,
        )?,
        "image" => {
            if let Some(mode) = properties.get("mode") {
                choice(
                    mode,
                    &format!("{path}.mode"),
                    &["cover", "contain", "stretch"],
                )?;
            }
        }
        "gradient" => {
            if let Some(kind) = properties.get("gradientType") {
                choice(
                    kind,
                    &format!("{path}.gradientType"),
                    &["linear", "radial", "angular"],
                )?;
            }
            if let Some(stops) = properties.get("colors") {
                let stops = stops
                    .as_array()
                    .ok_or_else(|| invalid(format!("{path}.colors must be an array")))?;
                for (index, stop) in stops.iter().enumerate() {
                    let stop_path = format!("{path}.colors[{index}]");
                    fields(stop, &stop_path, &["color", "position"])?;
                    color(
                        stop.get("color").unwrap_or(&Value::Null),
                        &format!("{stop_path}.color"),
                        authored,
                    )?;
                    number(
                        stop.get("position").unwrap_or(&Value::Null),
                        &format!("{stop_path}.position"),
                        authored,
                    )?;
                }
            }
        }
        "mesh_gradient" => {
            if let Some(colors) = properties.get("colors") {
                each(colors, &format!("{path}.colors"), |value, path| {
                    color(value, path, authored)
                })?;
            }
        }
        _ => (),
    }
    Ok(())
}

fn effect(value: &Value, path: &str, authored: bool) -> Result<()> {
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let allowed: &[&str] = match kind {
        "blur" | "background_blur" => &["type", "enabled", "radius"],
        "shadow" => &[
            "type",
            "enabled",
            "shadowType",
            "offset",
            "blur",
            "color",
            "blendMode",
        ],
        "shader" => &["type", "enabled", "url", "uniforms"],
        _ => {
            return Err(invalid(format!(
                "{path}.type has unsupported effect type {kind:?}"
            )));
        }
    };
    let properties = fields(value, path, allowed)?;
    blend(properties.get("blendMode"), path)?;
    for key in ["radius", "blur"] {
        if let Some(value) = properties.get(key) {
            number(value, &format!("{path}.{key}"), authored)?;
        }
    }
    if let Some(value) = properties.get("offset") {
        let path = format!("{path}.offset");
        for (key, value) in fields(value, &path, &["x", "y"])? {
            number(value, &format!("{path}.{key}"), authored)?;
        }
    }
    if let Some(kind) = properties.get("shadowType") {
        choice(kind, &format!("{path}.shadowType"), &["inner", "outer"])?;
    }
    if let Some(value) = properties.get("color") {
        color(value, &format!("{path}.color"), authored)?;
    }
    Ok(())
}

fn blend(value: Option<&Value>, path: &str) -> Result<()> {
    if let Some(value) = value {
        choice(
            value,
            &format!("{path}.blendMode"),
            &[
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
            ],
        )?;
    }
    Ok(())
}
