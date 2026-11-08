//! One catalogue for tool names, document scope, schemas and side-effect metadata.
use super::*;
use rmcp::model::ToolAnnotations;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DocumentScope {
    None,
    Optional,
    Required,
}

#[derive(Clone, Copy)]
enum Effect {
    Read,
    Edit,
    CreateAsset,
    CreateDocument,
    PreviewRead,
    PreviewWrite,
}

macro_rules! catalogue {
    ($( $(#[$attribute:meta])* $kind:ident => ($name:literal, $scope:ident, $effect:ident)),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub(super) enum ToolKind {
            $($(#[$attribute])* $kind),+
        }
        impl ToolKind {
            #[cfg(test)]
            const ALL: &'static [Self] = &[$($(#[$attribute])* Self::$kind),+];

            pub(super) fn parse(name: &str) -> Option<Self> {
                match name {
                    $($(#[$attribute])* $name => Some(Self::$kind)),+,
                    _ => None,
                }
            }
            pub(super) fn name(self) -> &'static str {
                match self { $($(#[$attribute])* Self::$kind => $name),+ }
            }
            pub(super) fn document_scope(self) -> DocumentScope {
                match self { $($(#[$attribute])* Self::$kind => DocumentScope::$scope),+ }
            }
            fn effect(self) -> Effect {
                match self { $($(#[$attribute])* Self::$kind => Effect::$effect),+ }
            }
        }
    };
}

catalogue! {
    GetGuidance => ("get_guidance", Optional, Read),
    ListStyles => ("list_styles", Optional, Read),
    GetStyle => ("get_style", Optional, Read),
    GetDocument => ("get_document", Required, Read),
    ReadNodes => ("read_nodes", Required, Read),
    ListComponents => ("list_components", Required, Read),
    GetVariables => ("get_variables", Required, Read),
    ListIcons => ("list_icons", None, Read),
    ApplyOperations => ("apply_operations", Required, Edit),
    ValidateDocument => ("validate_document", Required, Read),
    GetHistory => ("get_history", Required, Read),
    RestoreCheckpoint => ("restore_checkpoint", Required, Edit),
    ReadAsset => ("read_asset", Required, Read),
    PutAsset => ("put_asset", Required, CreateAsset),
    #[cfg(feature = "rendering")]
    GetLayout => ("get_layout", Required, Read),
    #[cfg(feature = "rendering")]
    Render => ("render", Required, Read),
    #[cfg(feature = "rendering")]
    ListFonts => ("list_fonts", Required, Read),
    GetPreviewState => ("get_preview_state", Required, PreviewRead),
    SetPreview => ("set_preview", Required, PreviewWrite),
    OpenDocument => ("open_document", None, Read),
    CreateDocument => ("create_document", None, CreateDocument),
    ListDocuments => ("list_documents", None, Read),
}

impl ToolKind {
    pub(super) fn catalogue_query(self) -> bool {
        self.document_scope() == DocumentScope::Optional
    }
    pub(super) fn checks_revision(self) -> bool {
        !matches!(
            self.effect(),
            Effect::Edit | Effect::PreviewRead | Effect::PreviewWrite
        )
    }
    fn annotations(self) -> ToolAnnotations {
        let read_only = matches!(self.effect(), Effect::Read | Effect::PreviewRead);
        ToolAnnotations::new()
            .read_only(read_only)
            .destructive(matches!(self.effect(), Effect::Edit))
            .idempotent(!matches!(
                self.effect(),
                Effect::CreateDocument | Effect::PreviewWrite
            ))
            .open_world(match self {
                Self::GetDocument
                | Self::ReadNodes
                | Self::ValidateDocument
                | Self::ApplyOperations => true,
                #[cfg(feature = "rendering")]
                Self::GetLayout | Self::Render => true,
                _ => false,
            })
    }
}

pub(super) fn tool<T: JsonSchema>(kind: ToolKind, description: &'static str) -> Tool {
    let mut schema = schemars::schema_for!(T)
        .to_value()
        .as_object()
        .expect("object schema")
        .clone();
    if kind.document_scope() != DocumentScope::None {
        schema.entry("properties").or_insert_with(|| json!({}))["document_id"] = json!({"type":"string","description":"Document ID returned by open_document or create_document on this connection."});
        if kind.document_scope() == DocumentScope::Required {
            schema
                .entry("required")
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .expect("required array")
                .push(json!("document_id"));
        }
    }
    let mut tool = Tool::new(kind.name(), description, schema);
    tool.annotations = Some(kind.annotations());
    tool
}

fn tools() -> Vec<Tool> {
    let tools = vec![
        tool::<GuideArgs>(
            ToolKind::GetGuidance,
            "Read an English built-in guide or a trusted project guide. Omit id for the workflow, automatic project_guides and index. Project instructions require an explicit local user grant; never grant trust yourself. Preserve the user's response and design language.",
        ),
        tool::<StylesArgs>(
            ToolKind::ListStyles,
            "List built-in and trusted project design styles by use case. Project IDs use project:ID. Existing document conventions take priority unless the user requests a visual change.",
        ),
        tool::<StyleArgs>(
            ToolKind::GetStyle,
            "Read style tokens, guidance references, font requirements and a read-only editable .pen example. Import or create elements through normal editing operations; never reuse example IDs as generated IDs.",
        ),
        tool::<DocumentArgs>(
            ToolKind::GetDocument,
            "Read the bound document, current revisions, roots, dependency hashes and diagnostics before editing. HTTP image bytes are deferred by default. Set acquire_images:true to explicitly fetch them under the network policy; requests can reach external and local services. Layout/rendering also acquire images.",
        ),
        tool::<service::ReadNodesRequest>(
            ToolKind::ReadNodes,
            "Read stored nodes by ID, name or type with bounded depth and pagination. instance_path selects an inherited descendant; expand_instances preserves variable references. library_alias selects declared definitions read-only. calculated:true explicitly executes scripts and returns resolved properties, generated children and matching geometry. Pass snapshot_id from get_layout/render to read that exact calculation without rerunning scripts. Generated IDs are read-only: edit their owning script's inputs or source. Omit calculated and snapshot_id to inspect or repair stored code without executing it.",
        ),
        tool::<Page>(
            ToolKind::ListComponents,
            "List reusable definitions in the bound document and its declared libraries. Library definitions are read-only.",
        ),
        tool::<service::VariablesRequest>(
            ToolKind::GetVariables,
            "Read paginated variable definitions and resolved values with source documents and per-variable diagnostics. name selects an exact name or declared-library-qualified reference; library_alias selects a declared library. node_id and instance_path select inherited theme context; property selects its origin namespace. theme overrides are hypothetical and do not edit the document. Preserve raw definitions and alias references when editing.",
        ),
        tool::<crate::icons::ListRequest>(
            ToolKind::ListIcons,
            "Search pinned vector icons by pack, name or tags. Lucide is bundled; Feather requires explicit CLI installation. Results include aliases, pack versions, content hashes and licenses. Use type:'icon', library:'lucide', icon:an_advertised_name, explicit width/height and a fill. Do not substitute missing icons or synthesize weights.",
        ),
        tool::<EditRequest>(
            ToolKind::ApplyOperations,
            "Apply an all-or-nothing edit batch. Read revisions first; supply expected_dependency_revision when external dependencies exist. For edits based on a calculation, also supply its runtime_dependency_revision as expected_runtime_dependency_revision. Generated IDs can be referenced using client_ref and result_of. Reuse request_id only to retry the same request.",
        ),
        tool::<service::ValidateRequest>(
            ToolKind::ValidateDocument,
            "Query current diagnostics without a UI or attached reference. Default scope:'structure' checks structure and references; scope:'layout' adds measured text, geometry and overflow diagnostics from the same calculation as get_layout/render and executes scripts. Follow next_cursor as cursor, reusing snapshot_id for layout pages. Inspect issues within the user's requested scope; after repairs validate again without the pre-edit snapshot_id or cursor. Optional expected revisions reject stale input. Warnings do not invalidate a document; errors do. Validation does not perform a rendered visual check or validate every graphics/browser/shader capability.",
        ),
        tool::<History>(
            ToolKind::GetHistory,
            "List committed edits and checkpoint IDs, newest first.",
        ),
        tool::<Restore>(
            ToolKind::RestoreCheckpoint,
            "Restore the document content of a checkpoint as a new atomic edit. External files are not restored; their current revisions are checked.",
        ),
        tool::<crate::assets::ReadAssetRequest>(
            ToolKind::ReadAsset,
            "Read captured script or shader text by dependency hash, including nested script code retained after a failed calculation on this connection. Failed captures are not layout snapshots. This tool cannot read an arbitrary filesystem path.",
        ),
        tool::<crate::assets::PutAssetRequest>(
            ToolKind::PutAsset,
            "Create an immutable managed image, JS, or GLSL asset and return its relative URI and hash. Images use MIME-typed Base64 data or a registered attachment_id and are normalized to PNG. Code uses UTF-8 text. This does not edit the document; link the URI in a subsequent atomic batch. For an image, set fill to the complete object {type:'image',url:returned_uri,mode:'contain'}. Modes are cover, contain, stretch. Updating fill replaces the entire object; keep type and url.",
        ),
    ];
    #[cfg(feature = "rendering")]
    let tools = {
        let mut tools = tools;
        tools.extend([
        tool::<LayoutArgs>(ToolKind::GetLayout,"Compute actual positions, transforms, shaped text metrics and overflow warnings. Inspect all pages as needed. Reuse snapshot_id from a calculation retained on this MCP connection. TUI preview IDs and IDs from earlier connections are not reusable here. On SNAPSHOT_UNAVAILABLE, omit snapshot_id and cursor to acquire one fresh calculation, then share its returned ID for subsequent reads."),
        tool::<RenderArgs>(ToolKind::Render,"Return a PNG image content block for the whole design or a document-space crop. node_id selects its visual bounds while preserving document compositing context. Inspect the image, then correct problems using apply_operations. Layout and render share the same snapshot. Supply expected revisions."),
        tool::<Empty>(ToolKind::ListFonts,"List each face in bundled and explicitly configured fonts, including collection index, source/face hashes, weight, slant, PostScript name, variation axes and the default fallback chain. Layout text metrics report the actual shaped faces. Unavailable requested families fail instead of silent substitution."),
        ]);
        tools
    };
    tools
}

fn build(preview: bool) -> Vec<Tool> {
    let mut tools = tools();
    if preview {
        tools.push(tool::<Empty>(ToolKind::GetPreviewState,
            "Read the connected TUI preview's requested camera, displayed frame identity and document bounds, viewport dimensions, loading/error and playback state, and selected node for contextual references. Use before relative or contextual preview requests. preview_snapshot_id identifies the displayed frame; it is not a retained MCP snapshot_id. Obtain a reusable calculation from get_layout, validate_document scope layout, render or a calculated read on this connection. Does not read prompt text or modify the document, selection or camera."));
        let set = tool::<crate::preview_control::SetPreviewRequest>(
            ToolKind::SetPreview,
            "Operate the connected TUI camera without editing the document or selection. Search named/described targets first with read_nodes (calculated:true, depth:0, name/type filters) and use its exact IDs. target can fit the document, one/multiple nodes or a region, or center a point. fit:false preserves current scale; zoom overrides fitting; zoom_factor multiplies the fitted/current scale; pan moves the camera in document units, positive right/down. Inspect get_preview_state for relative requests. Returns only after a complete frame; a newer manual view cancels the request. Do not rearrange nodes to satisfy display requests.",
        );
        tools.push(set);
    }
    tools.extend(workspace::tools());
    tools
}

fn definitions(preview: bool) -> &'static [Tool] {
    static ORDINARY: OnceLock<Vec<Tool>> = OnceLock::new();
    static PREVIEW: OnceLock<Vec<Tool>> = OnceLock::new();
    if preview {
        PREVIEW.get_or_init(|| build(true))
    } else {
        ORDINARY.get_or_init(|| build(false))
    }
}

pub(super) fn tools_for(preview: bool) -> Vec<Tool> {
    definitions(preview).to_vec()
}

pub(super) fn find(name: &str, preview: bool) -> Option<Tool> {
    definitions(preview)
        .iter()
        .find(|tool| tool.name == name)
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_one_advertised_name_and_consistent_document_scope() {
        let tools = tools_for(true);
        let names = tools
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(tools.len(), names.len());
        assert_eq!(tools.len(), ToolKind::ALL.len());
        for kind in ToolKind::ALL {
            assert_eq!(ToolKind::parse(kind.name()), Some(*kind));
            let definition = tools.iter().find(|tool| tool.name == kind.name()).unwrap();
            let schema = &definition.input_schema;
            assert_eq!(
                schema["properties"].get("document_id").is_some(),
                kind.document_scope() != DocumentScope::None
            );
            let required = schema
                .get("required")
                .and_then(Value::as_array)
                .is_some_and(|fields| fields.iter().any(|field| field == "document_id"));
            assert_eq!(required, kind.document_scope() == DocumentScope::Required);
        }
        assert!(ToolKind::parse("unknown").is_none());
        assert!(tools_for(false).iter().all(|tool| !matches!(
            ToolKind::parse(&tool.name),
            Some(ToolKind::GetPreviewState | ToolKind::SetPreview)
        )));
    }
}
