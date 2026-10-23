//! Typed WebGL inputs prepared from captured GLSL and document properties.

use crate::{
    Error, Result,
    project::{Project, ResolvedNode},
};
use base64::Engine;
use serde_json::{Map, Value, json};

mod declarations;

#[derive(Debug)]
pub struct Uniform {
    pub name: String,
    pub kind: String,
    pub resolution: bool,
    pub time: bool,
    pub array: bool,
    pub length: Option<usize>,
}

pub fn uniforms(source: &str) -> Result<Vec<Uniform>> {
    declarations::parse(source)
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
    let definitions = uniforms(source)?;
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
    if overrides
        .keys()
        .any(|name| !definitions.iter().any(|uniform| &uniform.name == name))
    {
        return Err(invalid("Override names must match declared uniforms"));
    }
    let mut values = Map::new();
    let mut arrays = Map::new();
    let mut component_count = 0;
    for uniform in definitions {
        let value = overrides.get(&uniform.name);
        if (uniform.resolution || uniform.time) && value.is_some() {
            return Err(invalid(
                "Built-in @resolution and @time uniforms cannot have stored overrides",
            ));
        }
        let value = if uniform.array {
            arrays.insert(uniform.name.clone(), json!(uniform.length));
            numeric_array(&uniform, value)?
        } else if uniform.resolution {
            json!([width, height])
        } else if uniform.time {
            json!(time)
        } else {
            match uniform.kind.as_str() {
                "sampler2D" => {
                    let encoded = if let Some(value) = value {
                        let uri = value
                            .as_str()
                            .ok_or_else(|| invalid("Sampler uniform requires an image URL"))?;
                        let data = match uri {
                            "pencil:object" => object.ok_or_else(|| {
                                invalid("pencil:object is unavailable at this fill stage")
                            })?,
                            "pencil:background" => background.ok_or_else(|| {
                                invalid("pencil:background is unavailable at this stage")
                            })?,
                            _ => {
                                let path =
                                    project.resolve_file(node.property_source(property), uri)?;
                                project.asset(&path)?.bytes.as_ref()
                            }
                        };
                        let image = crate::image_input::normalize(data, None)?;
                        base64::engine::general_purpose::STANDARD.encode(image.png)
                    } else {
                        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGMAAQAABQABDQottAAAAABJRU5ErkJggg==".into()
                    };
                    json!(format!("data:image/png;base64,{encoded}"))
                }
                _ => numeric(&uniform.kind, value)?,
            }
        };
        component_count += value.as_array().map_or(1, Vec::len);
        if component_count > 4096 {
            return Err(invalid(
                "Shader uniform input exceeds 4096 numeric components",
            ));
        }
        values.insert(uniform.name, value);
    }
    let mut result = json!({"source":source,"source_hash":asset.revision,"width":width,"height":height,"time":time,"uniforms":values});
    if !arrays.is_empty() {
        result["uniform_arrays"] = Value::Object(arrays);
    }
    Ok(result)
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
            "Unsupported numeric uniform type {kind:?}; image sampler arrays are not supported"
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
