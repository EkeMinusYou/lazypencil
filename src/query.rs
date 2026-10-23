use crate::{Error, Result};
use serde_json::Value;

pub(crate) fn page_offset(
    identity: &str,
    cursor: Option<&str>,
    limit: usize,
    total: usize,
) -> Result<usize> {
    if !(1..=500).contains(&limit) {
        return Err(Error::new("INVALID_ARGUMENT", "Page limit must be 1..500"));
    }
    let offset = match cursor {
        None => 0,
        Some(cursor) => {
            let (expected, offset) = cursor
                .rsplit_once(':')
                .ok_or_else(|| Error::new("INVALID_ARGUMENT", "Malformed page cursor"))?;
            if expected != identity {
                return Err(Error::new(
                    "REVISION_CONFLICT",
                    "The document, dependencies, or query changed; restart pagination",
                ));
            }
            offset
                .parse::<usize>()
                .map_err(|_| Error::new("INVALID_ARGUMENT", "Malformed page cursor offset"))?
        }
    };
    if offset > total {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Cursor offset is outside this result",
        ));
    }
    Ok(offset)
}

pub(crate) fn response(value: Value) -> Result<Value> {
    if value.to_string().len() > 4 * 1024 * 1024 {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Query response exceeds 4 MiB; reduce the page limit or select a smaller query",
        ));
    }
    Ok(value)
}
