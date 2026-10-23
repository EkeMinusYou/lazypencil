use crate::{Error, Result};
use skia_safe::Color;

pub fn parse(value: &str) -> Result<Color> {
    let bytes = value
        .strip_prefix('#')
        .filter(|s| s.is_ascii())
        .ok_or_else(|| invalid(value))?;
    let expanded = match bytes.len() {
        3 => bytes.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 | 8 => bytes.into(),
        _ => return Err(invalid(value)),
    };
    let number = u32::from_str_radix(&expanded, 16).map_err(|_| invalid(value))?;
    Ok(if expanded.len() == 8 {
        Color::from_argb(
            number as u8,
            (number >> 24) as u8,
            (number >> 16) as u8,
            (number >> 8) as u8,
        )
    } else {
        Color::from_rgb((number >> 16) as u8, (number >> 8) as u8, number as u8)
    })
}

fn invalid(value: &str) -> Error {
    Error::new(
        "INVALID_DOCUMENT",
        format!("Unsupported color {value:?}; expected #RGB, #RRGGBB, or #RRGGBBAA"),
    )
}
