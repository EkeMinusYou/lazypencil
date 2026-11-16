//! Spatial effect parameters shared by layout and native compositing.
use crate::{Error, Result, geometry::Affine};
use serde_json::Value;

pub(crate) fn active(value: &Value) -> impl Iterator<Item = &Value> {
    value
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_else(|| std::slice::from_ref(value))
        .iter()
        .filter(|effect| effect["enabled"] != false)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Geometry {
    pub blur: f32,
    pub offset: [f32; 2],
}

impl Geometry {
    pub fn parse(effect: &Value) -> Result<Self> {
        fn number(value: Option<&Value>) -> Result<f32> {
            value
                .map(|value| {
                    value
                        .as_f64()
                        .map(|value| value as f32)
                        .filter(|value| value.is_finite())
                        .ok_or_else(|| {
                            Error::new("INVALID_DOCUMENT", "Expected a finite effect parameter")
                        })
                })
                .unwrap_or(Ok(0.0))
        }
        let property = match effect["type"].as_str() {
            Some("blur" | "background_blur") => Some("radius"),
            Some("shadow") => Some("blur"),
            _ => None,
        };
        let blur = number(property.and_then(|property| effect.get(property)))?;
        if blur < 0.0 {
            return Err(Error::new(
                "INVALID_DOCUMENT",
                "Effect blur must be a nonnegative number",
            ));
        }
        let offset = if effect["type"] == "shadow" {
            let offset = effect.get("offset");
            [
                number(offset.and_then(|value| value.get("x")))?,
                number(offset.and_then(|value| value.get("y")))?,
            ]
        } else {
            [0.0; 2]
        };
        Ok(Self { blur, offset })
    }

    pub fn scaled(self, transform: Affine, scale: f32) -> Result<Self> {
        let [a, b, c, d, _, _] = transform.0;
        let [x, y] = self.offset;
        let result = Self {
            blur: self.blur * scale,
            offset: [(a * x + c * y) * scale, (b * x + d * y) * scale],
        };
        if !result.blur.is_finite() || result.offset.iter().any(|value| !value.is_finite()) {
            return Err(Error::new(
                "INVALID_DOCUMENT",
                "Scaled effect parameters must remain finite",
            ));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn effect_geometry_uses_typed_defaults_and_only_the_linear_transform_for_offsets() {
        assert_eq!(
            Geometry::parse(&json!({"type":"shadow"})).unwrap(),
            Geometry::default()
        );
        let geometry =
            Geometry::parse(&json!({"type":"shadow","blur":3,"offset":{"x":4,"y":-5}})).unwrap();
        assert_eq!(
            geometry
                .scaled(Affine([0.0, -2.0, 3.0, 0.0, 99.0, -19.0]), 2.0)
                .unwrap(),
            Geometry {
                blur: 6.0,
                offset: [-30.0, -16.0]
            }
        );
        assert_eq!(
            Geometry::parse(&json!({"type":"blur","radius":4,"blur":"unused"}))
                .unwrap()
                .blur,
            4.0
        );
    }

    #[test]
    fn active_effects_skip_invalid_disabled_values_and_reject_unrepresentable_geometry() {
        let effects =
            json!([{"type":"blur","radius":"invalid","enabled":false},{"type":"blur","radius":2}]);
        assert_eq!(
            active(&effects)
                .map(Geometry::parse)
                .collect::<Result<Vec<_>>>()
                .unwrap()
                .len(),
            1
        );
        for effect in [
            json!({"type":"blur","radius":-1}),
            json!({"type":"background_blur","radius":1e100}),
            json!({"type":"shadow","offset":{"x":"invalid"}}),
        ] {
            assert_eq!(
                Geometry::parse(&effect).unwrap_err().code,
                "INVALID_DOCUMENT"
            );
        }
        let huge = Geometry {
            blur: f32::MAX,
            offset: [0.0; 2],
        };
        assert_eq!(
            huge.scaled(Affine([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]), 2.0)
                .unwrap_err()
                .code,
            "INVALID_DOCUMENT"
        );
    }
}
