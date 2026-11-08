//! Extensible success contracts shared by MCP schemas and real-transport tests.
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

type Object = Map<String, Value>;

macro_rules! outputs {
    ($($name:ident => [$($tool:literal),+] { $($fields:tt)* })+) => {
        $(#[allow(dead_code)]
        #[derive(Deserialize, JsonSchema)]
        struct $name { $($fields)* })+

        pub(super) fn schema(tool: &str) -> Object {
            match tool {
                $($($tool)|+ => schemars::schema_for!($name).to_value().as_object().unwrap().clone(),)+
                _ => panic!("Missing output contract for {tool}"),
            }
        }

        #[cfg(test)]
        pub(super) fn validate(tool: &str, value: Value) -> Result<(), serde_json::Error> {
            match tool {
                $($($tool)|+ => serde_json::from_value::<$name>(value).map(drop),)+
                _ => panic!("Missing output contract for {tool}"),
            }
        }
    };
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
struct Diagnostic {
    code: String,
    severity: String,
    message: String,
    node_id: Option<String>,
    details: Option<Value>,
}

outputs! {
    Handle => ["open_document", "create_document"] {
        document_id: String,
        path: String,
        revision: String,
        version: String,
    }
    Closed => ["close_document"] {
        document_id: String,
        closed: bool,
    }
    Documents => ["list_documents"] {
        directory: String,
        documents: Vec<String>,
        total_matches: usize,
        /// False means discovery stopped at a resource or time limit.
        complete: bool,
        stop_reason: Option<String>,
        entries_scanned: usize,
        unreadable_directories: usize,
        next_cursor: Option<String>,
    }
    Metadata => ["get_document"] {
        document_id: String,
        path: String,
        version: String,
        revision: String,
        node_count: usize,
        writable: bool,
        roots: Vec<Object>,
        diagnostics: Vec<Diagnostic>,
        capabilities: Object,
        /// Compact by default; include_configuration returns full settings and origins.
        configuration: Option<Object>,
        guidance: Object,
        dependency_revision: Option<String>,
        dependencies: Option<Vec<Object>>,
        runtime_dependency_revision: Option<String>,
        runtime_dependencies: Option<Vec<Object>>,
        assets_complete: Option<bool>,
        asset_diagnostics: Option<Vec<Object>>,
        dependency_error: Option<Object>,
        resolution_error: Option<Object>,
        attachments: Option<Object>,
    }
    Guide => ["get_guidance"] {
        document_id: Option<String>,
        id: String,
        version: String,
        language: String,
        summary: String,
        body: String,
        hash: String,
        available_guides: Option<Vec<Object>>,
        project_guides: Option<Vec<Object>>,
    }
    Styles => ["list_styles"] {
        document_id: Option<String>,
        version: String,
        language: String,
        styles: Vec<Object>,
        total_matches: usize,
        next_cursor: Option<String>,
    }
    Style => ["get_style"] {
        document_id: Option<String>,
        id: String,
        name: String,
        version: String,
        language: String,
        description: String,
        hash: String,
        tokens: Object,
        example: Object,
    }
    Nodes => ["read_nodes"] {
        document_id: String,
        revision: String,
        dependency_revision: Option<String>,
        nodes: Vec<Object>,
        total_matches: usize,
        next_cursor: Option<String>,
        snapshot_id: Option<String>,
        view: String,
    }
    Components => ["list_components"] {
        document_id: String,
        revision: String,
        dependency_revision: String,
        components: Vec<Object>,
        total_matches: usize,
        next_cursor: Option<String>,
    }
    Variables => ["get_variables"] {
        document_id: String,
        revision: String,
        dependency_revision: String,
        variables: Object,
        resolved_values: Object,
        sources: Object,
        diagnostics: Vec<Diagnostic>,
        resolution_complete: bool,
        total_matches: usize,
        next_cursor: Option<String>,
    }
    Icons => ["list_icons"] {
        pack: Option<String>,
        packs: Vec<Object>,
        icons: Vec<Object>,
        total: Option<usize>,
        next_cursor: Option<String>,
        diagnostic: Option<Object>,
    }
    Edited => ["apply_operations"] {
        document_id: String,
        revision: String,
        dependency_revision: String,
        request_id: String,
        changed_ids: Vec<String>,
        bindings: BTreeMap<String, String>,
        generated_ids: Object,
        diagnostics: Vec<Diagnostic>,
        checkpoint_id: Option<String>,
        replayed: Option<bool>,
        dry_run: Option<bool>,
        proposed_revision: Option<String>,
        validated_runtime_dependency_revision: Option<String>,
    }
    Validation => ["validate_document"] {
        document_id: String,
        revision: String,
        dependency_revision: String,
        /// Warnings do not make the document invalid.
        valid: bool,
        complete: bool,
        validated_scopes: Vec<String>,
        diagnostics: Vec<Diagnostic>,
        total_diagnostics: usize,
        next_cursor: Option<String>,
        snapshot_id: Option<String>,
        error_count: Option<usize>,
        warning_count: Option<usize>,
    }
    History => ["get_history"] {
        document_id: String,
        revision: String,
        entries: Vec<Object>,
        next_before_sequence: Option<u64>,
    }
    Restored => ["restore_checkpoint"] {
        document_id: String,
        revision: String,
        dependency_revision: String,
        request_id: String,
        restored_checkpoint: String,
        checkpoint_id: String,
        replayed: bool,
    }
    Code => ["read_asset"] {
        document_id: String,
        revision: String,
        dependency_revision: String,
        hash: String,
        kind: String,
        content: String,
        offset: usize,
        total_bytes: usize,
        next_offset: Option<usize>,
    }
    Asset => ["put_asset"] {
        document_id: String,
        kind: String,
        uri: String,
        hash: String,
        mime_type: String,
        bytes: usize,
        reused: bool,
        width: Option<u32>,
        height: Option<u32>,
    }
    Layout => ["get_layout"] {
        document_id: String,
        snapshot_id: String,
        revision: String,
        dependency_revision: String,
        runtime_dependency_revision: String,
        node_count: usize,
        nodes: Vec<Object>,
        total_matches: usize,
        next_cursor: Option<String>,
        diagnostics: Vec<Diagnostic>,
        total_diagnostics: usize,
        error_count: usize,
        warning_count: usize,
    }
    Render => ["render"] {
        document_id: String,
        snapshot_id: String,
        revision: String,
        dependency_revision: String,
        runtime_dependency_revision: String,
        width: u32,
        height: u32,
        region: Object,
        png_hash: String,
        diagnostics: Vec<Diagnostic>,
        total_diagnostics: usize,
    }
    Fonts => ["list_fonts"] {
        document_id: String,
        revision: String,
        dependency_revision: String,
        fonts: Vec<Object>,
        fallback_families: Vec<String>,
        default_family: String,
        locale: String,
    }
    Preview => ["get_preview_state"] {
        document_id: String,
        selected_node: Option<Object>,
        preview: Object,
    }
    Displayed => ["set_preview"] {
        document_id: String,
        status: String,
        revision: Option<String>,
        dependency_revision: Option<String>,
        node_ids: Option<Vec<String>>,
        bounds: Option<Object>,
        center: Option<Vec<f64>>,
        zoom: Option<f64>,
        view: Option<Object>,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_contracts_reject_missing_required_fields_and_wrong_types() {
        assert!(
            validate(
                "close_document",
                serde_json::json!({"document_id":"id","closed":true})
            )
            .is_ok()
        );
        assert!(validate("close_document", serde_json::json!({"document_id":"id"})).is_err());
        assert!(
            validate(
                "close_document",
                serde_json::json!({"document_id":"id","closed":"true"})
            )
            .is_err()
        );
        assert_eq!(schema("close_document")["type"], "object");
    }
}
