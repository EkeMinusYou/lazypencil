//! Typed WebGL inputs prepared from captured GLSL and document properties.

use crate::{
    Error, Result,
    project::{Project, ResolvedNode},
};
use base64::Engine;
use serde_json::{Map, Value, json};

mod declarations;
mod defaults;

#[derive(Debug)]
pub struct Uniform {
    pub name: String,
    pub kind: String,
    pub resolution: bool,
    pub time: bool,
    pub array: bool,
    pub length: Option<usize>,
    pub default: Option<Value>,
    /// Bounds for structure-array indices represented by [] in the name pattern.
    pub indices: Vec<Option<usize>>,
}

impl Uniform {
    pub fn matches(&self, name: &str) -> bool {
        if name.len() > 4096 {
            return false;
        }
        let mut parts = self.name.split("[]");
        let mut rest = name;
        for length in &self.indices {
            let Some(prefix) = parts.next() else {
                return false;
            };
            let Some(tail) = rest
                .strip_prefix(prefix)
                .and_then(|tail| tail.strip_prefix('['))
            else {
                return false;
            };
            let Some((digits, tail)) = tail.split_once(']') else {
                return false;
            };
            if digits.is_empty()
                || digits.len() > 3
                || !digits.bytes().all(|byte| byte.is_ascii_digit())
                || digits.len() > 1 && digits.starts_with('0')
            {
                return false;
            }
            let Ok(index) = digits.parse::<usize>() else {
                return false;
            };
            if index >= 256 || length.is_some_and(|length| index >= length) {
                return false;
            }
            rest = tail;
        }
        parts.next().is_some_and(|suffix| suffix == rest) && parts.next().is_none()
    }
}

pub fn uniforms(source: &str) -> Result<Vec<Uniform>> {
    declarations::parse(source)
}

pub(crate) fn preprocessing_source(source: &str) -> Result<Option<String>> {
    declarations::preprocessing_source(source)
}

#[allow(clippy::too_many_arguments)]
pub fn request(
    project: &Project,
    node: &ResolvedNode,
    fill: &Value,
    property: &str,
    width: u32,
    height: u32,
    time: f64,
    object: Option<&[u8]>,
    background: Option<&[u8]>,
) -> Result<Value> {
    request_with_coordinates(
        project, node, fill, property, width, height, None, time, object, background,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn request_with_coordinates(
    project: &Project,
    node: &ResolvedNode,
    fill: &Value,
    property: &str,
    width: u32,
    height: u32,
    coordinates: Option<[f64; 2]>,
    time: f64,
    object: Option<&[u8]>,
    background: Option<&[u8]>,
) -> Result<Value> {
    request_with_declarations(
        project,
        node,
        fill,
        property,
        width,
        height,
        coordinates,
        time,
        object,
        background,
        None,
    )
}

pub(crate) fn source<'a>(
    project: &'a Project,
    node: &ResolvedNode,
    fill: &Value,
    property: &str,
) -> Result<(&'a str, &'a str)> {
    let url = fill["url"]
        .as_str()
        .ok_or_else(|| invalid("Shader requires a local source URL"))?;
    let path = project.resolve_file(node.property_source(property), url)?;
    let asset = project.asset(&path)?;
    if asset.bytes.len() > 1024 * 1024 {
        return Err(invalid("Shader source exceeds 1 MiB"));
    }
    let source =
        std::str::from_utf8(&asset.bytes).map_err(|_| invalid("Shader source must be UTF-8"))?;
    Ok((source, &asset.revision))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn request_with_declarations(
    project: &Project,
    node: &ResolvedNode,
    fill: &Value,
    property: &str,
    width: u32,
    height: u32,
    coordinates: Option<[f64; 2]>,
    time: f64,
    object: Option<&[u8]>,
    background: Option<&[u8]>,
    expanded: Option<&str>,
) -> Result<Value> {
    if coordinates.is_some_and(|size| size.iter().any(|value| !value.is_finite() || *value <= 0.0))
    {
        return Err(invalid(
            "Shader logical dimensions must be positive and finite",
        ));
    }
    let (source, source_hash) = source(project, node, fill, property)?;
    let definitions = expanded.map_or_else(|| uniforms(source), declarations::parse_expanded)?;
    let time_dependent = definitions.iter().any(|uniform| uniform.time);
    let empty = Map::new();
    let overrides = fill
        .get("uniforms")
        .map(|value| {
            value
                .as_object()
                .ok_or_else(|| invalid("Shader uniforms must be an object"))
        })
        .transpose()?
        .unwrap_or(&empty);
    if overrides.len() > 4096 {
        return Err(invalid("Shader uniform input exceeds 4096 bindings"));
    }
    if overrides
        .keys()
        .any(|name| !definitions.iter().any(|uniform| uniform.matches(name)))
    {
        return Err(invalid("Override names must match declared uniforms"));
    }
    let mut values = Map::new();
    let mut arrays = Map::new();
    let mut types = Map::new();
    let mut default_arrays = Vec::new();
    let mut component_count = 0;
    let mut sampler_bytes = 0usize;
    for uniform in definitions {
        types.insert(uniform.name.clone(), json!(uniform.kind));
        if uniform.array {
            arrays.insert(uniform.name.clone(), json!(uniform.length));
        }
        let mut bindings = vec![(uniform.name.as_str(), overrides.get(&uniform.name))];
        if !uniform.indices.is_empty() {
            bindings.extend(
                overrides
                    .iter()
                    .filter(|(name, _)| uniform.matches(name))
                    .map(|(name, value)| (name.as_str(), Some(value))),
            );
        }
        for (name, stored) in bindings {
            if (uniform.resolution || uniform.time) && stored.is_some() {
                return Err(invalid(
                    "Built-in @resolution and @time uniforms cannot have stored overrides",
                ));
            }
            let value = stored.or(uniform.default.as_ref());
            let value = if uniform.array {
                if uniform.kind == "sampler2D" {
                    match value {
                        None => Value::Null,
                        Some(value) => {
                            let inputs = value.as_array().filter(|values| {
                            !values.is_empty()
                                && values.len() <= 256
                                && uniform.length.is_none_or(|length| length == values.len())
                        }).ok_or_else(|| invalid("Sampler arrays require 1–256 image URLs matching the declared length"))?;
                            Value::Array(
                                inputs
                                    .iter()
                                    .map(|value| {
                                        sampler(
                                            project,
                                            node,
                                            property,
                                            Some(value),
                                            object,
                                            background,
                                            &mut sampler_bytes,
                                        )
                                    })
                                    .collect::<Result<_>>()?,
                            )
                        }
                    }
                } else if stored.is_none() && uniform.default.is_some() {
                    // Native defaults initialize one element; remaining linked
                    // elements keep zero. Explicit short overrides still fail.
                    default_arrays.push(name.to_owned());
                    let element = numeric(&uniform.kind, value)?;
                    match element {
                        Value::Array(values) => Value::Array(values),
                        Value::Bool(value) => json!([u8::from(value)]),
                        value => json!([value]),
                    }
                } else {
                    numeric_array(&uniform, value)?
                }
            } else if uniform.resolution {
                coordinates.map_or_else(|| json!([width, height]), |size| json!(size))
            } else if uniform.time {
                json!(time)
            } else {
                match uniform.kind.as_str() {
                    "sampler2D" => sampler(
                        project,
                        node,
                        property,
                        value,
                        object,
                        background,
                        &mut sampler_bytes,
                    )?,
                    _ => numeric(&uniform.kind, value)?,
                }
            };
            component_count += value.as_array().map_or(1, Vec::len);
            if component_count > 4096 {
                return Err(invalid(
                    "Shader uniform input exceeds 4096 numeric components",
                ));
            }
            values.insert(name.to_owned(), value);
        }
    }
    let mut result = json!({"source":source,"source_hash":source_hash,"width":width,"height":height,"time":time,"time_dependent":time_dependent,"uniforms":values,"uniform_types":types});
    if let Some(size) = coordinates {
        result["coordinate_size"] = json!(size);
    }
    if !arrays.is_empty() {
        result["uniform_arrays"] = Value::Object(arrays);
    }
    if !default_arrays.is_empty() {
        result["uniform_array_defaults"] = json!(default_arrays);
    }
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn sampler(
    project: &Project,
    node: &ResolvedNode,
    property: &str,
    value: Option<&Value>,
    object: Option<&[u8]>,
    background: Option<&[u8]>,
    supplied_bytes: &mut usize,
) -> Result<Value> {
    let encoded = if let Some(value) = value {
        let uri = value
            .as_str()
            .ok_or_else(|| invalid("Sampler uniform requires an image URL"))?;
        let data = match uri {
            "pencil:object" => {
                object.ok_or_else(|| invalid("pencil:object is unavailable at this fill stage"))?
            }
            "pencil:background" => background
                .ok_or_else(|| invalid("pencil:background is unavailable at this stage"))?,
            _ => {
                let path = project.resolve_file(node.property_source(property), uri)?;
                project.asset(&path)?.bytes.as_ref()
            }
        };
        let image = crate::image_input::normalize(data, None)?;
        base64::engine::general_purpose::STANDARD.encode(image.png)
    } else {
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGMAAQAABQABDQottAAAAABJRU5ErkJggg==".into()
    };
    *supplied_bytes = supplied_bytes.saturating_add(encoded.len());
    if *supplied_bytes > 64 * 1024 * 1024 {
        return Err(invalid("Encoded sampler inputs exceed 64 MiB"));
    }
    Ok(json!(format!("data:image/png;base64,{encoded}")))
}

#[derive(Clone, Copy)]
enum NumberKind {
    Float,
    Integer,
    Boolean,
}

fn numeric_type(kind: &str) -> Result<(usize, NumberKind)> {
    use NumberKind::*;
    match kind {
        "float" => Ok((1, Float)),
        "int" => Ok((1, Integer)),
        "bool" => Ok((1, Boolean)),
        "vec2" => Ok((2, Float)),
        "vec3" => Ok((3, Float)),
        "vec4" => Ok((4, Float)),
        "ivec2" => Ok((2, Integer)),
        "ivec3" => Ok((3, Integer)),
        "ivec4" => Ok((4, Integer)),
        "bvec2" => Ok((2, Boolean)),
        "bvec3" => Ok((3, Boolean)),
        "bvec4" => Ok((4, Boolean)),
        "mat2" => Ok((4, Float)),
        "mat3" => Ok((9, Float)),
        "mat4" => Ok((16, Float)),
        _ => Err(invalid(format!(
            "Unsupported numeric uniform type {kind:?}"
        ))),
    }
}

fn component(kind: NumberKind, value: &Value) -> Result<Value> {
    let number = finite(value)?;
    match kind {
        NumberKind::Float => {
            if number.abs() > f32::MAX as f64 {
                return Err(invalid(
                    "Float uniform exceeds the finite WebGL float range",
                ));
            }
            Ok(json!(number))
        }
        NumberKind::Integer => {
            if number.fract() != 0.0 || number < i32::MIN as f64 || number > i32::MAX as f64 {
                return Err(invalid("Integer uniform is outside the i32 range"));
            }
            Ok(json!(number as i32))
        }
        NumberKind::Boolean => {
            if number != 0.0 && number != 1.0 {
                return Err(invalid(
                    "Boolean vector/array components require numeric 0 or 1",
                ));
            }
            Ok(json!(number as i32))
        }
    }
}

fn numeric(kind: &str, value: Option<&Value>) -> Result<Value> {
    let (width, number) = numeric_type(kind)?;
    if kind == "bool" {
        return Ok(json!(
            value
                .map(|v| v
                    .as_bool()
                    .ok_or_else(|| invalid("Boolean uniform requires a boolean")))
                .transpose()?
                .unwrap_or(false)
        ));
    }
    if width == 1 {
        return component(number, value.unwrap_or(&json!(0)));
    }
    if let Some(Value::String(color)) = value {
        if !matches!(kind, "vec3" | "vec4") {
            return Err(invalid("Colors can only bind vec3 or vec4"));
        }
        let color = crate::color::parse(color)?;
        return Ok(json!(
            [color.r(), color.g(), color.b(), color.a()][..width]
                .iter()
                .map(|v| *v as f64 / 255.0)
                .collect::<Vec<_>>()
        ));
    }
    match value {
        None => Ok(Value::Array(
            (0..width)
                .map(|_| component(number, &json!(0)))
                .collect::<Result<Vec<_>>>()?,
        )),
        Some(value) => {
            let values = value
                .as_array()
                .filter(|v| v.len() == width)
                .ok_or_else(|| invalid("Vector or matrix uniform has the wrong length"))?;
            Ok(Value::Array(
                values
                    .iter()
                    .map(|v| component(number, v))
                    .collect::<Result<Vec<_>>>()?,
            ))
        }
    }
}

fn numeric_array(uniform: &Uniform, value: Option<&Value>) -> Result<Value> {
    let (width, kind) = numeric_type(&uniform.kind)?;
    let Some(value) = value else {
        return Ok(Value::Null);
    };
    let values = value
        .as_array()
        .filter(|v| !v.is_empty() && v.len() <= 4096 && v.len() % width == 0)
        .ok_or_else(|| {
            invalid("Uniform arrays require a flat numeric array of complete elements")
        })?;
    if uniform
        .length
        .is_some_and(|length| values.len() != length * width)
    {
        return Err(invalid("Uniform array has the wrong declared length"));
    }
    Ok(Value::Array(
        values
            .iter()
            .map(|v| component(kind, v))
            .collect::<Result<Vec<_>>>()?,
    ))
}
fn finite(value: &Value) -> Result<f64> {
    value
        .as_f64()
        .filter(|value| value.is_finite())
        .ok_or_else(|| invalid("Uniform value must be a finite number"))
}
fn invalid(message: impl Into<String>) -> Error {
    Error::new("SHADER_COMPILE_FAILED", message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    #[test]
    fn preprocessing_preserves_directive_annotation_barriers_and_source_positions() {
        for directive in ["#define UNUSED 1", "#if 1", "#endif", "#else", "#line 40 7"] {
            let source = format!(
                "precision highp float;\r\n/** @resolution */\r\n{directive}\r\nuniform vec2 size;\r\n/** @default .5 */ DECL(float,gain)\r\n"
            );
            let prepared = preprocessing_source(&source).unwrap().unwrap();
            assert_eq!(prepared.len(), source.len());
            let changed = source
                .bytes()
                .zip(prepared.bytes())
                .enumerate()
                .filter(|(_, (a, b))| a != b)
                .collect::<Vec<_>>();
            assert_eq!(
                changed,
                vec![(source.find("/**").unwrap() + 2, (b'*', b' '))]
            );
            assert!(prepared.contains("/** @default .5 */ DECL"));
            assert_eq!(source.lines().count(), prepared.lines().count());
        }
        for source in [
            "/** @color @default #3a7fc1 */ uniform vec3 tint;",
            "/* #if 0 */ uniform float value;",
            "// #define TYPE int\nuniform float value;",
        ] {
            assert!(preprocessing_source(source).unwrap().is_none());
        }
        let empty = "/**/\n#define TYPE float\nuniform TYPE value;";
        assert_eq!(preprocessing_source(empty).unwrap().unwrap(), empty);
        let source =
            "/** @default .5 */ /* ordinary separator */\n#define TYPE float\nuniform TYPE value;";
        assert_eq!(preprocessing_source(source).unwrap().unwrap(), source);
        let source = "/** @default .5 */\n#define TYPE \\\n float\nuniform TYPE value;";
        let prepared = preprocessing_source(source).unwrap().unwrap();
        assert_eq!(prepared.len(), source.len());
        assert!(prepared.contains("#define TYPE \\\n float"));
        assert!(preprocessing_source(&"#".repeat(1024 * 1024 + 1)).is_err());
    }

    #[test]
    fn prepared_declarations_bind_values_without_replacing_captured_shader_bytes() {
        let root = tempfile::TempDir::new().unwrap();
        let raw = "#define TYPE vec3\nuniform TYPE tint;";
        std::fs::write(root.path().join("input.glsl"), raw).unwrap();
        let document = json!({"version":"2.21","children":[{"type":"rectangle","id":"shape","width":32,"height":16,"fill":{"type":"shader","url":"input.glsl","uniforms":{"tint":[0.25,0.5,0.75]}}}]}).to_string();
        std::fs::write(root.path().join("input.pen"), &document).unwrap();
        let project = Project::from_document(
            &root.path().join("input.pen"),
            Document::parse(document).unwrap(),
            &[],
        )
        .unwrap();
        let scene = project.resolve().unwrap();
        let node = &scene.roots[0];
        let prepared = request_with_declarations(
            &project,
            node,
            &node.properties["fill"],
            "fill",
            64,
            32,
            Some([32., 16.]),
            0.5,
            None,
            None,
            Some("uniform vec3 tint;"),
        )
        .unwrap();
        assert_eq!(prepared["source"], raw);
        assert_eq!(
            prepared["source_hash"],
            crate::document::hash(raw.as_bytes())
        );
        assert_eq!(prepared["uniform_types"]["tint"], "vec3");
        assert_eq!(prepared["uniforms"]["tint"], json!([0.25, 0.5, 0.75]));
        assert_eq!(prepared["time_dependent"], false);
        for invalid in ["uniform float tint;", "uniform vec3 other;"] {
            assert_eq!(
                request_with_declarations(
                    &project,
                    node,
                    &node.properties["fill"],
                    "fill",
                    32,
                    16,
                    None,
                    0.,
                    None,
                    None,
                    Some(invalid)
                )
                .unwrap_err()
                .code,
                "SHADER_COMPILE_FAILED"
            );
        }
        let excessive = " ".repeat(4 * 1024 * 1024 + 1);
        assert!(declarations::parse_expanded(&excessive).is_err());
        // An expanded declaration stream has its own bounded four-MiB budget;
        // the original input still has the independent one-MiB limit.
        let expanded = " ".repeat(1024 * 1024) + "uniform float value;";
        assert!(uniforms(&expanded).is_err());
        assert_eq!(declarations::parse_expanded(&expanded).unwrap().len(), 1);
        assert_eq!(
            std::fs::read_to_string(root.path().join("input.glsl")).unwrap(),
            raw
        );
    }
}
