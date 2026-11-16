#[cfg(feature = "rendering")]
use crate::{Error, Result};
#[cfg(feature = "rendering")]
use skia_safe::Color;

pub(crate) fn hex_rgba(value: &str) -> Option<[u8; 4]> {
    let digits = value.strip_prefix('#').filter(|s| s.is_ascii())?;
    let step = match digits.len() {
        3 => 1,
        6 | 8 => 2,
        _ => return None,
    };
    let mut rgba = [0, 0, 0, 255];
    for (index, component) in rgba.iter_mut().enumerate().take(digits.len() / step) {
        let byte = u8::from_str_radix(&digits[index * step..(index + 1) * step], 16).ok()?;
        *component = if step == 1 { byte * 17 } else { byte };
    }
    Some(rgba)
}

#[cfg(feature = "rendering")]
pub fn parse(value: &str) -> Result<Color> {
    let [r, g, b, a] = hex_rgba(value).ok_or_else(|| invalid(value))?;
    Ok(Color::from_argb(a, r, g, b))
}

#[cfg(feature = "rendering")]
fn invalid(value: &str) -> Error {
    Error::new(
        "INVALID_DOCUMENT",
        format!("Unsupported color {value:?}; expected #RGB, #RRGGBB, or #RRGGBBAA"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colors_expand_each_channel_and_default_only_missing_alpha() {
        for (literal, expected) in [
            ("#AbC", [170, 187, 204, 255]),
            ("#123456", [18, 52, 86, 255]),
            ("#12345600", [18, 52, 86, 0]),
        ] {
            assert_eq!(hex_rgba(literal), Some(expected), "{literal}");
        }
        for literal in [
            "",
            "123456",
            "#12",
            "#AbCd",
            "#12345",
            "#1234567",
            "#ggg",
            "#１２３",
            "#fff ",
        ] {
            assert_eq!(hex_rgba(literal), None, "{literal}");
        }
    }
}
