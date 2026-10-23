//! Bounded OpenType collection unpacking for backends without TTC index support.

use crate::{Error, Result};
use std::{borrow::Cow, collections::BTreeMap};

const MAX_BYTES: usize = 64 * 1024 * 1024;

fn invalid(message: &str) -> Error {
    Error::new("INVALID_ARGUMENT", message)
}
fn word(bytes: &[u8], offset: usize) -> Result<u32> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| invalid("Font offset overflow"))?;
    let value = bytes
        .get(offset..end)
        .ok_or_else(|| invalid("Truncated font container"))?;
    Ok(u32::from_be_bytes(value.try_into().expect("four bytes")))
}

pub(super) fn count(bytes: &[u8]) -> Result<u32> {
    if !bytes.starts_with(b"ttcf") {
        return Ok(1);
    }
    let version = word(bytes, 4)?;
    if !matches!(version, 0x10000 | 0x20000) {
        return Err(invalid("Unsupported font collection version"));
    }
    let count = word(bytes, 8)?;
    if !(1..=256).contains(&count) {
        return Err(invalid("A font collection needs 1..256 faces"));
    }
    let header = 12 + count as usize * 4 + if version == 0x20000 { 12 } else { 0 };
    if bytes.len() < header {
        return Err(invalid("Truncated font collection offsets"));
    }
    for index in 0..count as usize {
        let offset = word(bytes, 12 + index * 4)? as usize;
        if offset < header
            || !offset.is_multiple_of(4)
            || offset.checked_add(12).is_none_or(|end| end > bytes.len())
        {
            return Err(invalid("Invalid font collection face offset"));
        }
    }
    Ok(count)
}

fn checksum(bytes: &[u8]) -> u32 {
    let (words, tail) = bytes.as_chunks::<4>();
    let mut sum = words.iter().fold(0u32, |sum, word| {
        sum.wrapping_add(u32::from_be_bytes(*word))
    });
    if !tail.is_empty() {
        let mut padded = [0u8; 4];
        padded[..tail.len()].copy_from_slice(tail);
        sum = sum.wrapping_add(u32::from_be_bytes(padded));
    }
    sum
}

/// Rebuild directory offsets and checksums; glyph and shaping tables stay unchanged.
pub(super) fn extract(bytes: &[u8], index: u32) -> Result<Cow<'_, [u8]>> {
    let count = count(bytes)?;
    if index >= count {
        return Err(invalid("Font collection index is out of range"));
    }
    if !bytes.starts_with(b"ttcf") {
        return Ok(Cow::Borrowed(bytes));
    }
    let offset = word(bytes, 12 + index as usize * 4)? as usize;
    let magic = word(bytes, offset)?;
    if !matches!(magic, 0x10000 | 0x4f54544f | 0x74727565) {
        return Err(invalid("Unsupported collection face format"));
    }
    let table_count = (word(bytes, offset + 4)? >> 16) as usize;
    if !(1..=4095).contains(&table_count) {
        return Err(invalid("A collection face needs 1..4095 tables"));
    }
    let end = offset + 12 + table_count * 16;
    if end > bytes.len() {
        return Err(invalid("Truncated collection face directory"));
    }
    let mut tables = BTreeMap::new();
    for entry in 0..table_count {
        let position = offset + 12 + entry * 16;
        let tag = word(bytes, position)?;
        let start = word(bytes, position + 8)? as usize;
        let length = word(bytes, position + 12)? as usize;
        let end = start
            .checked_add(length)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| invalid("Collection table lies outside the file"))?;
        if !start.is_multiple_of(4) {
            return Err(invalid("Collection table is not aligned"));
        }
        if tables.insert(tag, &bytes[start..end]).is_some() {
            return Err(invalid("Collection contains duplicate table tags"));
        }
    }
    // A signature over the original container cannot describe the new directory.
    tables.remove(&u32::from_be_bytes(*b"DSIG"));
    let count = tables.len();
    if count == 0 {
        return Err(invalid("Collection face has no font tables"));
    }
    let power = 1usize << (usize::BITS - 1 - count.leading_zeros());
    let mut output = vec![0u8; 12 + count * 16];
    output[..4].copy_from_slice(&magic.to_be_bytes());
    for (position, value) in [
        (4, count),
        (6, power * 16),
        (8, power.trailing_zeros() as usize),
        (10, count * 16 - power * 16),
    ] {
        output[position..position + 2].copy_from_slice(&(value as u16).to_be_bytes());
    }
    let mut head = None;
    for (index, (tag, body)) in tables.into_iter().enumerate() {
        let offset = output.len();
        if offset + body.len() + 3 > MAX_BYTES {
            return Err(invalid("Extracted font exceeds 64 MiB"));
        }
        output.extend_from_slice(body);
        output.resize(output.len().next_multiple_of(4), 0);
        if tag == u32::from_be_bytes(*b"head") {
            if body.len() < 12 {
                return Err(invalid("Font head table is truncated"));
            }
            output[offset + 8..offset + 12].fill(0);
            head = Some(offset);
        }
        let sum = checksum(&output[offset..offset + body.len()]);
        let record = 12 + index * 16;
        for (position, value) in [
            (record, tag),
            (record + 4, sum),
            (record + 8, offset as u32),
            (record + 12, body.len() as u32),
        ] {
            output[position..position + 4].copy_from_slice(&value.to_be_bytes());
        }
    }
    let head = head.ok_or_else(|| invalid("Collection face has no head table"))?;
    let adjustment = 0xb1b0afbau32.wrapping_sub(checksum(&output));
    output[head + 8..head + 12].copy_from_slice(&adjustment.to_be_bytes());
    Ok(Cow::Owned(output))
}
