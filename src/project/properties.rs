//! Property names whose authored strings never denote variable references.

pub(crate) fn literal_variable_property(key: &str) -> bool {
    matches!(
        key,
        "theme"
            | "metadata"
            | "name"
            | "context"
            | "id"
            | "ref"
            | "scriptUri"
            | "url"
            | "geometry"
            | "href"
    )
}

pub(super) fn literal_property(key: &str) -> bool {
    // Resolution additionally handles structural fields outside ordinary values.
    literal_variable_property(key) || matches!(key, "type" | "descendants" | "slot")
}
