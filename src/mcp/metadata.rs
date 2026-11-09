//! Bounded document overviews and revision-bound metadata collection pages.
use super::*;

const PREVIEW_ENTRIES: usize = 100;
const PREVIEW_BYTES: usize = 64 * 1024;
const PAGE_BYTES: usize = 3 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Deserialize, serde::Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum Collection {
    Roots,
    Diagnostics,
    Dependencies,
    RuntimeDependencies,
    AssetDiagnostics,
    Attachments,
    FontDiagnostics,
}

impl Collection {
    const ALL: [Self; 7] = [
        Self::Roots,
        Self::Diagnostics,
        Self::Dependencies,
        Self::RuntimeDependencies,
        Self::AssetDiagnostics,
        Self::Attachments,
        Self::FontDiagnostics,
    ];
    fn name(self) -> &'static str {
        match self {
            Self::Roots => "roots",
            Self::Diagnostics => "diagnostics",
            Self::Dependencies => "dependencies",
            Self::RuntimeDependencies => "runtime_dependencies",
            Self::AssetDiagnostics => "asset_diagnostics",
            Self::Attachments => "attachments",
            Self::FontDiagnostics => "font_diagnostics",
        }
    }
    fn pointer(self) -> &'static str {
        match self {
            Self::Roots => "/roots",
            Self::Diagnostics => "/diagnostics",
            Self::Dependencies => "/dependencies",
            Self::RuntimeDependencies => "/runtime_dependencies",
            Self::AssetDiagnostics => "/asset_diagnostics",
            Self::Attachments => "/attachments/images",
            Self::FontDiagnostics => "/configuration/font_diagnostics",
        }
    }
}

pub(super) fn validate(args: &DocumentArgs) -> Result<()> {
    if !(1..=500).contains(&args.limit) || args.cursor.is_some() && args.collection.is_none() {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Metadata limit must be 1..500; cursor requires collection",
        ));
    }
    Ok(())
}

pub(super) fn prepare(
    mut value: Value,
    args: &DocumentArgs,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<Value> {
    validate(args)?;
    let identity = crate::document::canonical_hash(&json!({
        "revision":value["revision"],"dependency_revision":value["dependency_revision"],
        "runtime_dependency_revision":value["runtime_dependency_revision"],"collection":args.collection
    }));
    let mut collections = serde_json::Map::new();
    for collection in Collection::ALL {
        crate::cancel::check(Some(cancel))?;
        let selected = args.collection == Some(collection);
        let entries = value
            .pointer_mut(collection.pointer())
            .and_then(Value::as_array_mut);
        let entries = match entries {
            Some(entries) => std::mem::take(entries),
            None if selected => {
                return Err(Error::new(
                    "INVALID_ARGUMENT",
                    "This metadata collection is unavailable; read the overview for available collections and dependency diagnostics",
                ));
            }
            None => continue,
        };
        let total = entries.len();
        let offset = if selected {
            crate::query::page_offset(&identity, args.cursor.as_deref(), args.limit, total)?
        } else {
            0
        };
        let count = if selected {
            args.limit
        } else {
            PREVIEW_ENTRIES
        };
        let budget = if selected { PAGE_BYTES } else { PREVIEW_BYTES };
        let mut bytes = 2;
        let mut page = Vec::new();
        for entry in entries.into_iter().skip(offset).take(count) {
            let Some(remaining) = budget.checked_sub(bytes + usize::from(!page.is_empty())) else {
                break;
            };
            match crate::query::checked_serialized_size(
                &entry,
                remaining,
                Some(cancel),
                "Metadata entry exceeds the page byte budget",
            ) {
                Ok(size) => {
                    bytes += size + usize::from(!page.is_empty());
                    page.push(entry);
                }
                Err(error) if error.code == "INVALID_ARGUMENT" => {
                    if selected && page.is_empty() {
                        return Err(Error::new(
                            "INVALID_ARGUMENT",
                            "One metadata entry exceeds 3 MiB; use CLI inspect for its complete contents",
                        ));
                    }
                    break;
                }
                Err(error) => return Err(error),
            }
        }
        let end = offset + page.len();
        collections.insert(collection.name().into(), json!({
            "total":total,"offset":offset,"returned":page.len(),"truncated":offset > 0 || end < total
        }));
        if let Some(target) = value.pointer_mut(collection.pointer()) {
            *target = Value::Array(page);
        }
        if selected {
            value["collection"] = json!(collection);
            value["total_matches"] = json!(total);
            value["next_cursor"] = json!((end < total).then(|| format!("{identity}:{end}")));
        }
    }
    value["collections"] = Value::Object(collections);
    let mut omitted = Vec::new();
    for field in [
        "configuration",
        "guidance",
        "attachments",
        "dependency_error",
        "resolution_error",
    ] {
        if field == "configuration" && args.include_configuration {
            continue;
        }
        if let Some(object) = value.get_mut(field).and_then(Value::as_object_mut) {
            let mut remaining = PREVIEW_BYTES;
            for (key, entry) in std::mem::take(object) {
                // Collection arrays already have independent page/preview budgets.
                if Collection::ALL
                    .iter()
                    .any(|collection| collection.pointer() == format!("/{field}/{key}"))
                {
                    object.insert(key, entry);
                    continue;
                }
                let size = crate::query::checked_serialized_size(
                    &(&key, &entry),
                    remaining,
                    Some(cancel),
                    "Metadata detail exceeds the overview byte budget",
                );
                match size {
                    Ok(bytes) => {
                        remaining -= bytes;
                        object.insert(key, entry);
                    }
                    Err(error) if error.code == "INVALID_ARGUMENT" => {
                        omitted.push(format!("{field}.{key}"))
                    }
                    Err(error) => return Err(error),
                }
            }
        }
    }
    if !omitted.is_empty() {
        value["omitted_fields"] = json!(omitted);
    }
    crate::query::check_response_size(
        &value,
        MAX_TOOL_BYTES - 16384,
        Some(cancel),
        "Document metadata exceeds the response limit; omit include_configuration, reduce limit, or use CLI inspect for full metadata",
    )?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(value: Value) -> DocumentArgs {
        serde_json::from_value(value).unwrap()
    }
    fn overview(roots: Value) -> Value {
        json!({"revision":"root","dependency_revision":"dependencies","runtime_dependency_revision":"runtime",
            "roots":roots,"diagnostics":[],"configuration":{},"guidance":{},"capabilities":{}})
    }

    #[test]
    fn oversized_overviews_keep_identity_totals_and_exact_byte_boundaries() {
        let cancel = tokio_util::sync::CancellationToken::new();
        let row = json!({"id":"first","name":""});
        let mut exact = row.clone();
        exact["name"] = json!("x".repeat(PREVIEW_BYTES - 2 - row.to_string().len()));
        let mut input = overview(json!([exact,{"id":"second"}]));
        input["configuration"] = json!({"path":"config.toml","fonts_complete":false,"font_lock":{"huge":"x".repeat(PREVIEW_BYTES)}});
        input["resolution_error"] =
            json!({"code":"INVALID_DOCUMENT","message":"x".repeat(PREVIEW_BYTES)});
        let result = prepare(input, &args(json!({})), &cancel).unwrap();
        assert_eq!(result["revision"], "root");
        assert_eq!(result["dependency_revision"], "dependencies");
        assert_eq!(result["roots"].to_string().len(), PREVIEW_BYTES);
        assert_eq!(
            result["collections"]["roots"],
            json!({"total":2,"offset":0,"returned":1,"truncated":true})
        );
        assert_eq!(result["configuration"]["fonts_complete"], false);
        assert_eq!(result["resolution_error"]["code"], "INVALID_DOCUMENT");
        assert!(
            result["omitted_fields"]
                .as_array()
                .unwrap()
                .contains(&json!("configuration.font_lock"))
        );
        assert!(
            result["omitted_fields"]
                .as_array()
                .unwrap()
                .contains(&json!("resolution_error.message"))
        );
        let giant = overview(json!([{"id":"giant","name":"x".repeat(MAX_TOOL_BYTES)}]));
        let result = prepare(giant.clone(), &args(json!({})), &cancel).unwrap();
        assert!(result["roots"].as_array().unwrap().is_empty());
        assert_eq!(result["collections"]["roots"]["total"], 1);
        assert!(
            prepare(giant, &args(json!({"collection":"roots"})), &cancel)
                .unwrap_err()
                .message
                .contains("CLI inspect")
        );
    }

    #[test]
    fn byte_bounded_pages_return_every_entry_and_reject_stale_or_other_collection_cursors() {
        let cancel = tokio_util::sync::CancellationToken::new();
        let roots = (0..9)
            .map(|index| json!({"id":index.to_string(),"name":"x".repeat(512*1024)}))
            .collect::<Vec<_>>();
        let input = overview(json!(roots));
        let mut cursor = Value::Null;
        let mut found = Vec::new();
        loop {
            let result = prepare(
                input.clone(),
                &args(json!({"collection":"roots","limit":500,"cursor":cursor})),
                &cancel,
            )
            .unwrap();
            assert!(result.to_string().len() < MAX_TOOL_BYTES);
            assert_eq!(result["total_matches"], 9);
            found.extend(
                result["roots"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|row| row["id"].as_str().unwrap().to_owned()),
            );
            cursor = result["next_cursor"].clone();
            if cursor.is_null() {
                break;
            }
            let mut stale = input.clone();
            stale["dependency_revision"] = json!("changed");
            assert_eq!(
                prepare(
                    stale,
                    &args(json!({"collection":"roots","cursor":cursor})),
                    &cancel
                )
                .unwrap_err()
                .code,
                "REVISION_CONFLICT"
            );
            assert_eq!(
                prepare(
                    input.clone(),
                    &args(json!({"collection":"diagnostics","cursor":cursor})),
                    &cancel
                )
                .unwrap_err()
                .code,
                "REVISION_CONFLICT"
            );
        }
        assert_eq!(
            found,
            (0..9).map(|index| index.to_string()).collect::<Vec<_>>()
        );
        for invalid in [
            json!({"cursor":"anything"}),
            json!({"limit":0}),
            json!({"limit":501}),
            json!({"collection":"dependencies"}),
        ] {
            assert_eq!(
                prepare(input.clone(), &args(invalid), &cancel)
                    .unwrap_err()
                    .code,
                "INVALID_ARGUMENT"
            );
        }
        cancel.cancel();
        assert_eq!(
            prepare(input, &args(json!({})), &cancel).unwrap_err().code,
            "CANCELLED"
        );
    }

    #[test]
    fn nested_collection_pages_are_not_removed_by_the_overview_object_budget() {
        let cancel = tokio_util::sync::CancellationToken::new();
        for (collection, pointer) in [
            ("attachments", "/attachments/images"),
            ("font_diagnostics", "/configuration/font_diagnostics"),
        ] {
            let mut input = overview(json!([]));
            input["attachments"] = json!({"conversation":"session","images":[]});
            input["configuration"]["font_diagnostics"] = json!([]);
            *input.pointer_mut(pointer).unwrap() =
                json!([{"id":"large","description":"x".repeat(PREVIEW_BYTES * 2)}]);
            let result = prepare(input, &args(json!({"collection":collection})), &cancel).unwrap();
            assert_eq!(
                result.pointer(pointer).unwrap().as_array().unwrap().len(),
                1
            );
            assert_eq!(result["collections"][collection]["returned"], 1);
            assert_eq!(result["collections"][collection]["truncated"], false);
            assert!(result.get("omitted_fields").is_none());
        }
    }
}
