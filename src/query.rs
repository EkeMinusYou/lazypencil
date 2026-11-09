use crate::{Error, Result};
use serde::Serialize;
use serde_json::Value;
use std::io::{self, Write};
use tokio_util::sync::CancellationToken;

pub(crate) const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

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

/// Count serialized bytes without constructing a second complete JSON string.
pub(crate) fn serialized_size(value: &impl Serialize) -> usize {
    let mut counter = Counter::new(usize::MAX, None);
    serde_json::to_writer(&mut counter, value)
        .expect("Serializable data writes into an in-memory counter");
    counter.used
}

pub(crate) fn check_response_size(
    value: &impl Serialize,
    limit: usize,
    cancel: Option<&CancellationToken>,
    limit_message: &str,
) -> Result<()> {
    checked_serialized_size(value, limit, cancel, limit_message).map(|_| ())
}

pub(crate) fn checked_serialized_size(
    value: &impl Serialize,
    limit: usize,
    cancel: Option<&CancellationToken>,
    limit_message: &str,
) -> Result<usize> {
    crate::cancel::check(cancel)?;
    let mut counter = Counter::new(limit, cancel);
    if serde_json::to_writer(&mut counter, value).is_err() {
        crate::cancel::check(cancel)?;
        if counter.exceeded {
            return Err(Error::new("INVALID_ARGUMENT", limit_message));
        }
        return Err(Error::new(
            "INTERNAL_ERROR",
            "Cannot serialize query response",
        ));
    }
    crate::cancel::check(cancel)?;
    Ok(counter.used)
}

pub(crate) fn response(value: Value) -> Result<Value> {
    check_response_size(
        &value,
        MAX_RESPONSE_BYTES,
        None,
        "Query response exceeds 4 MiB; reduce the page limit or select a smaller query",
    )?;
    Ok(value)
}

struct Counter<'a> {
    used: usize,
    limit: usize,
    exceeded: bool,
    cancel: Option<&'a CancellationToken>,
}

impl<'a> Counter<'a> {
    fn new(limit: usize, cancel: Option<&'a CancellationToken>) -> Self {
        Self {
            used: 0,
            limit,
            exceeded: false,
            cancel,
        }
    }
}

impl Write for Counter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        // Interrupted errors would make write_all retry forever after cancellation.
        if self.cancel.is_some_and(CancellationToken::is_cancelled) {
            return Err(io::Error::other("Query cancelled"));
        }
        if bytes.len() > self.limit - self.used {
            self.exceeded = true;
            return Err(io::Error::other("Query response limit"));
        }
        self.used += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn response_budget_counts_json_escaping_and_utf8_at_the_exact_boundary() {
        let value = json!({"text":"日本語\n\"\\\t", "nested":[true, null, 1.25]});
        let bytes = value.to_string().len();
        assert_eq!(serialized_size(&value), bytes);
        check_response_size(&value, bytes, None, "too large").unwrap();
        let error = check_response_size(&value, bytes - 1, None, "too large").unwrap_err();
        assert_eq!(error.code, "INVALID_ARGUMENT");
        assert_eq!(error.message, "too large");
    }

    #[test]
    fn cancellation_keeps_priority_over_response_limits() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        let error =
            check_response_size(&json!("large"), 0, Some(&cancel), "too large").unwrap_err();
        assert_eq!(error.code, "CANCELLED");
    }

    #[test]
    fn serialized_size_stops_before_visiting_the_rest_of_an_oversized_result() {
        use serde::ser::SerializeSeq;
        struct ResultRows;
        impl Serialize for ResultRows {
            fn serialize<S: serde::Serializer>(
                &self,
                serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                let mut rows = serializer.serialize_seq(Some(2))?;
                rows.serialize_element("first row exceeds the tiny budget")?;
                panic!("An oversized result must stop before evaluating later rows");
            }
        }
        assert_eq!(
            check_response_size(&ResultRows, 8, None, "too large")
                .unwrap_err()
                .code,
            "INVALID_ARGUMENT"
        );
    }
}
