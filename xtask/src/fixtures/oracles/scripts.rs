//! Expectations for authored input contracts and the public CC0 Mulberry32 algorithm.
//!
//! The generator is independent of the application's JavaScript bootstrap and
//! of any image or JSON observation exported by another application.

use super::Raster;
use serde_json::{Value, json};

pub struct InputCase {
    pub name: &'static str,
    pub stored: Value,
    pub value: Value,
}

impl InputCase {
    pub fn observation(&self) -> Value {
        let kind = if self.value.is_number() {
            "number"
        } else if self.value.is_boolean() {
            "boolean"
        } else {
            "string"
        };
        assert!(self.value.is_number() || self.value.is_boolean() || self.value.is_string());
        json!([kind, self.value, ["value"], null])
    }
}

pub fn input_cases() -> Vec<InputCase> {
    [
        ("number-missing", json!({}), json!(0)),
        ("limited-number-missing", json!({}), json!(2)),
        ("string-missing", json!({}), json!("")),
        ("boolean-missing", json!({}), json!(false)),
        ("color-missing", json!({}), json!("#000000")),
        ("enum-missing", json!({}), json!("one")),
        ("ref-missing", json!({}), json!("")),
        ("number-default-clamp", json!({}), json!(5)),
        ("number-zero-clamp", json!({"value":0}), json!(2)),
        ("boolean-false", json!({"value":false}), json!(false)),
        ("enum-choice", json!({"value":"two"}), json!("two")),
        (
            "ref-selected",
            json!({"value":"template"}),
            json!("template"),
        ),
        ("unknown-input", json!({"value":3,"extra":"kept"}), json!(3)),
        ("number-null", json!({"value":null}), json!(4)),
        ("string-null", json!({"value":null}), json!("fallback")),
        ("boolean-null", json!({"value":null}), json!(true)),
        ("color-null", json!({"value":null}), json!("#123456")),
        ("enum-null", json!({"value":null}), json!("two")),
        ("ref-null", json!({"value":null}), json!("")),
        ("ref-empty", json!({"value":""}), json!("")),
        (
            "unknown-variable-input",
            json!({"value":3,"extra":"$missing"}),
            json!(3),
        ),
        ("negative-maximum", json!({}), json!(-2)),
    ]
    .into_iter()
    .map(|(name, stored, value)| InputCase {
        name,
        stored,
        value,
    })
    .collect()
}

pub fn sample_indices() -> Vec<u32> {
    (0..32).chain([63, 255, 1023, 4095, 65535]).collect()
}

pub fn random_words() -> Vec<u32> {
    // Mulberry32 by Tommy Ettinger, CC0; original notice is retained in
    // resources/licenses/script-random/NOTICE.txt. Unsigned operations model
    // modulo 2^32 arithmetic directly rather than executing JavaScript.
    let indices = sample_indices();
    let mut words = Vec::new();
    let mut state = 0u32;
    for draw in 0..=65535 {
        state = state.wrapping_add(0x6d2b79f5);
        let mut word = (state ^ (state >> 15)).wrapping_mul(state | 1);
        word ^= word.wrapping_add((word ^ (word >> 7)).wrapping_mul(word | 61));
        word ^= word >> 14;
        if indices.get(words.len()) == Some(&draw) {
            words.push(word);
        }
    }
    assert_eq!(words.len(), indices.len());
    words
}

fn color(word: u32) -> [u8; 4] {
    [(word >> 16) as u8, (word >> 8) as u8, word as u8, 255]
}

pub fn random_raster(scale: u32) -> Raster {
    assert!((1..=2).contains(&scale));
    let mut image = Raster::solid(664 * scale, 96 * scale, [255; 4]);
    let words = random_words();
    for row in 0..4 {
        for (column, word) in words.iter().enumerate() {
            // floor((word / 2^32) * 2^48) is exactly word << 16.
            let halves = [word >> 8, (word & 255) << 16];
            for (half, word) in halves.into_iter().enumerate() {
                image.rectangle(
                    column as u32 * 16 * scale,
                    (row * 24 + half as u32 * 8) * scale,
                    12 * scale,
                    8 * scale,
                    color(word),
                );
            }
        }
    }
    image
}

pub fn input_raster(scale: u32) -> Raster {
    assert!((1..=2).contains(&scale));
    let mut image = Raster::solid(256 * scale, 264 * scale, [255; 4]);
    for (row, case) in input_cases().into_iter().enumerate() {
        let text = case.observation().to_string();
        let units = text.encode_utf16().collect::<Vec<_>>();
        assert!(units.len() <= 62);
        let words = std::iter::once(units.len() as u32).chain(units.into_iter().map(u32::from));
        for (column, word) in words.enumerate() {
            image.rectangle(
                column as u32 * 4 * scale,
                row as u32 * 12 * scale,
                4 * scale,
                4 * scale,
                color(word),
            );
        }
    }
    image
}
