use crate::{
    Error, Result,
    document::Document,
    operations::{self, EditRequest},
    project::Project,
    store::{self, Store},
};
use serde_json::{Value, json};
use std::{cell::RefCell, path::PathBuf};

#[cfg(feature = "rendering")]
mod calculated_nodes;
mod guidance;
mod validation;
pub use crate::project::VariablesRequest;
#[cfg(feature = "rendering")]
pub use calculated_nodes::{read_calculated_nodes, validate_calculated_nodes_request};
pub use guidance::{project_design_start, project_guidance, project_style, project_styles};
#[cfg(feature = "rendering")]
pub use validation::validation_from_snapshot;
pub use validation::{ValidateRequest, ValidationScope, validate_document_with_options};

pub fn get_variables(
    store: &Store,
    fonts: &[PathBuf],
    request: &VariablesRequest,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<Value> {
    crate::cancel::check(cancel)?;
    request.validate()?;
    let project = project_with_overlay_and_cancellation(
        store,
        store.read()?,
        fonts,
        &Default::default(),
        cancel,
    )?;
    let mut result = project.variables(request)?;
    asset_status(&mut result, &project);
    if let Some(error) = project.resolution_error() {
        result["resolution_error"] = json!(error);
    }
    store::require_revision(&store.read()?, &project.root_document().revision())?;
    crate::cancel::check(cancel)?;
    crate::query::response(result)
}

pub fn apply_request(store: &Store, request: &EditRequest) -> Result<Value> {
    apply_request_with_fonts(store, request, &[])
}

pub fn apply_request_with_fonts(
    store: &Store,
    request: &EditRequest,
    font_files: &[PathBuf],
) -> Result<Value> {
    apply_request_with_cancellation(store, request, font_files, None)
}

pub fn apply_request_with_cancellation(
    store: &Store,
    request: &EditRequest,
    font_files: &[PathBuf],
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<Value> {
    crate::cancel::check(cancel)?;
    if request.expected_runtime_dependency_revision.is_some() {
        #[cfg(feature = "rendering")]
        {
            let mut runtime = crate::script::ScriptRuntime::current()?;
            runtime.set_cancellation(cancel.cloned());
            return apply_request_with_runtime(store, request, font_files, &mut runtime);
        }
        #[cfg(not(feature = "rendering"))]
        return Err(Error::new(
            "UNSUPPORTED_FEATURE",
            "Runtime dependency preconditions require the rendering feature",
        ));
    }
    apply_prepared_request(store, request, font_files, None, cancel)
}

fn edit_fingerprint(request: &EditRequest, font_files: &[PathBuf]) -> String {
    crate::document::hash(
        json!({"request":request.fingerprint(),"fonts":font_files})
            .to_string()
            .as_bytes(),
    )
}

#[cfg(feature = "rendering")]
pub fn apply_request_with_runtime(
    store: &Store,
    request: &EditRequest,
    font_files: &[PathBuf],
    runtime: &mut crate::script::ScriptRuntime,
) -> Result<Value> {
    let cancel = runtime.cancellation();
    crate::cancel::check(cancel.as_ref())?;
    let Some(expected) = request.expected_runtime_dependency_revision.as_deref() else {
        return apply_prepared_request(store, request, font_files, None, cancel.as_ref());
    };
    if !request.dry_run
        && let Some(result) = store.replay_result_with_cancellation(
            &request.request_id,
            &edit_fingerprint(request, font_files),
            cancel.as_ref(),
        )?
    {
        return Ok(result);
    }
    // Execute before acquiring any document or dependency save locks.
    let (project, _) = layout_snapshot_with_runtime(
        store,
        font_files,
        Some(&request.expected_revision),
        request.expected_dependency_revision.as_deref(),
        runtime,
    )?;
    project.require_dependencies(request.expected_dependency_revision.as_deref())?;
    project.require_runtime_dependencies(expected)?;
    apply_prepared_request(store, request, font_files, Some(&project), cancel.as_ref())
}

fn apply_prepared_request(
    store: &Store,
    request: &EditRequest,
    font_files: &[PathBuf],
    runtime_project: Option<&Project>,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<Value> {
    crate::cancel::check(cancel)?;
    if request.dry_run {
        let document = store.read()?;
        store::require_revision(&document, &request.expected_revision)?;
        let before_project = project_with_overlay_and_cancellation(
            store,
            document.clone(),
            font_files,
            &Default::default(),
            cancel,
        )?;
        before_project.require_dependencies(request.expected_dependency_revision.as_deref())?;
        let prepared = operations::prepare_in_project(
            &document,
            &request.operations,
            store.path(),
            store.data_dir(),
            cancel,
        )?;
        let after = prepared.document;
        let mut result = prepared.result;
        let after_project = project_with_overlay_and_cancellation(
            store,
            after.clone(),
            font_files,
            &prepared.overlay,
            cancel,
        )?;
        let scene = after_project.resolve_for_edit()?;
        before_project.verify_unchanged()?;
        after_project.require_no_new_missing(&before_project)?;
        for source in &prepared.sources {
            source.verify_unchanged()?;
        }
        for file in &prepared.files {
            crate::cancel::check(cancel)?;
            file.verify(false)?;
        }
        if let Some(project) = runtime_project {
            project.verify_unchanged()?;
            result["validated_runtime_dependency_revision"] =
                json!(project.runtime_dependency_revision());
        }
        result["dry_run"] = json!(true);
        result["revision"] = json!(document.revision());
        result["proposed_revision"] = json!(after.revision());
        result["request_id"] = json!(request.request_id);
        result["dependency_revision"] = json!(scene.dependency_revision);
        result["dependencies"] = json!(scene.dependencies);
        asset_status(&mut result, &after_project);
        require_edit_response_budget(&result)?;
        crate::cancel::check(cancel)?;
        return Ok(result);
    }
    let conditions = WriteConditions {
        allow_missing_assets: false,
        request_id: &request.request_id,
        revision: &request.expected_revision,
        dependency: request.expected_dependency_revision.as_deref(),
        runtime_project,
        cancel,
        fingerprint: edit_fingerprint(request, font_files),
    };
    save_project(store, &conditions, font_files, |document| {
        operations::prepare_in_project(
            document,
            &request.operations,
            store.path(),
            store.data_dir(),
            cancel,
        )
    })
}

pub struct RestoreRequest<'a> {
    pub request_id: &'a str,
    pub expected_revision: &'a str,
    pub expected_dependency_revision: Option<&'a str>,
    pub checkpoint: &'a str,
}

pub fn restore_checkpoint(
    store: &Store,
    request: &RestoreRequest<'_>,
    font_files: &[PathBuf],
) -> Result<Value> {
    restore_checkpoint_with_cancellation(store, request, font_files, None)
}

pub fn restore_checkpoint_with_cancellation(
    store: &Store,
    request: &RestoreRequest<'_>,
    font_files: &[PathBuf],
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<Value> {
    let conditions = WriteConditions {
        allow_missing_assets: true,
        request_id: request.request_id,
        revision: request.expected_revision,
        dependency: request.expected_dependency_revision,
        runtime_project: None,
        cancel,
        fingerprint: crate::document::hash(
            json!({"restore":request.checkpoint,"expected_revision":request.expected_revision,
            "expected_dependency_revision":request.expected_dependency_revision,"fonts":font_files})
            .to_string()
            .as_bytes(),
        ),
    };
    save_project(store, &conditions, font_files, |_| {
        Ok(operations::PreparedBatch::plain(
            store.checkpoint_document(request.checkpoint)?,
            json!({"restored_checkpoint":request.checkpoint}),
        ))
    })
}

struct WriteConditions<'a> {
    allow_missing_assets: bool,
    request_id: &'a str,
    revision: &'a str,
    dependency: Option<&'a str>,
    runtime_project: Option<&'a Project>,
    cancel: Option<&'a tokio_util::sync::CancellationToken>,
    fingerprint: String,
}

fn project_with_fonts(
    store: &Store,
    document: Document,
    font_files: &[PathBuf],
) -> Result<Project> {
    project_with_overlay(store, document, font_files, &Default::default())
}

fn project_with_overlay(
    store: &Store,
    document: Document,
    font_files: &[PathBuf],
    overlay: &crate::managed::Overlay,
) -> Result<Project> {
    project_with_overlay_and_cancellation(store, document, font_files, overlay, None)
}

fn project_with_overlay_and_cancellation(
    store: &Store,
    document: Document,
    font_files: &[PathBuf],
    overlay: &crate::managed::Overlay,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<Project> {
    let mut project = Project::from_document_with_overlay_and_cancellation(
        store.path(),
        document,
        &[],
        Some(store.data_dir()),
        overlay,
        cancel,
    )?;
    project.add_font_files(font_files)?;
    Ok(project)
}

fn save_project<F>(
    store: &Store,
    conditions: &WriteConditions<'_>,
    font_files: &[PathBuf],
    edit: F,
) -> Result<Value>
where
    F: FnOnce(&Document) -> Result<operations::PreparedBatch>,
{
    let dependency_locks = RefCell::new(Vec::new());
    let projects = RefCell::new(None);
    store.transact_guarded_with_cancellation(
        conditions.request_id,
        conditions.revision,
        &conditions.fingerprint,
        conditions.cancel,
        |document| {
            crate::cancel::check(conditions.cancel)?;
            let before_project = project_with_overlay_and_cancellation(
                store,
                document.clone(),
                font_files,
                &Default::default(),
                conditions.cancel,
            )?;
            before_project.require_dependencies(conditions.dependency)?;
            let prepared = edit(document)?;
            let after = prepared.document;
            let mut result = prepared.result;
            crate::cancel::check(conditions.cancel)?;
            let mut after_project = project_with_overlay_and_cancellation(
                store,
                after.clone(),
                font_files,
                &prepared.overlay,
                conditions.cancel,
            )?;
            let scene = after_project.resolve_for_edit()?;
            before_project.verify_unchanged()?;
            if !conditions.allow_missing_assets {
                after_project.require_no_new_missing(&before_project)?;
            }
            for source in &prepared.sources {
                source.verify_unchanged()?;
            }
            result["dependency_revision"] = json!(scene.dependency_revision);
            result["dependencies"] = json!(scene.dependencies);
            asset_status(&mut result, &after_project);
            if let Some(project) = conditions.runtime_project {
                result["validated_runtime_dependency_revision"] =
                    json!(project.runtime_dependency_revision());
            }
            require_edit_response_budget(&result)?;
            for file in &prepared.files {
                crate::cancel::check(conditions.cancel)?;
                file.publish(store, conditions.cancel)?;
            }
            after_project.publish_overlay()?;
            after_project.refresh_missing_assets()?;
            result["dependency_revision"] = json!(after_project.dependency_revision());
            result["dependencies"] = json!(after_project.dependencies());
            asset_status(&mut result, &after_project);
            require_edit_response_budget(&result)?;
            let paths = before_project
                .dependencies()
                .into_iter()
                .chain(after_project.dependencies())
                .chain(prepared.sources.iter().flat_map(Project::live_dependencies))
                .chain(
                    conditions
                        .runtime_project
                        .into_iter()
                        .flat_map(Project::runtime_dependencies),
                )
                .map(|dependency| dependency.path)
                .chain(before_project.configuration_lock_paths())
                .chain(after_project.configuration_lock_paths())
                .chain(
                    prepared
                        .sources
                        .iter()
                        .flat_map(Project::configuration_lock_paths),
                )
                .chain(
                    conditions
                        .runtime_project
                        .into_iter()
                        .flat_map(Project::configuration_lock_paths),
                )
                .collect::<Vec<_>>();
            *dependency_locks.borrow_mut() =
                store.lock_dependencies_with_cancellation(&paths, conditions.cancel)?;
            before_project.verify_unchanged()?;
            for source in &prepared.sources {
                source.verify_unchanged()?;
            }
            if let Some(project) = conditions.runtime_project {
                project.verify_unchanged()?;
                result["validated_runtime_dependency_revision"] =
                    json!(project.runtime_dependency_revision());
            }
            after_project.verify_unchanged()?;
            crate::cancel::check(conditions.cancel)?;
            *projects.borrow_mut() = Some((before_project, after_project, prepared.sources));
            Ok((after, result))
        },
        || {
            crate::cancel::check(conditions.cancel)?;
            if let Some((before, after, sources)) = projects.borrow().as_ref() {
                before.verify_unchanged()?;
                after.verify_unchanged()?;
                for source in sources {
                    source.verify_unchanged()?;
                }
            }
            if let Some(project) = conditions.runtime_project {
                project.verify_unchanged()?;
            }
            Ok(())
        },
    )
}

fn require_edit_response_budget(value: &Value) -> Result<()> {
    // Leave room for the store's request, revision, checkpoint, and replay fields.
    let limit = 4 * 1024 * 1024 - 16384;
    let bytes = value.to_string().len();
    if bytes > limit {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "The prepared edit report exceeds the shared response limit; use smaller batches",
        )
        .details(json!({"response_bytes":bytes,"limit_bytes":limit})));
    }
    Ok(())
}

pub fn document_metadata(store: &Store) -> Result<Value> {
    document_metadata_with_fonts(store, &[])
}

fn asset_status(result: &mut Value, project: &Project) {
    result["assets_complete"] = if project.resolution_error().is_some() {
        Value::Null
    } else {
        json!(project.assets_complete())
    };
    result["asset_diagnostics"] = json!(project.asset_diagnostics());
}

pub fn document_metadata_with_fonts(
    store: &Store,
    font_files: &[std::path::PathBuf],
) -> Result<Value> {
    let document = store.read()?;
    let mut result = document.summary();
    result["path"] = json!(store.path());
    result["capabilities"] = json!({
        "schema":document.version(),"supported_schemas":crate::document::SUPPORTED_DOCUMENT_VERSIONS,"default_schema":crate::document::DOCUMENT_VERSION,"rendering":cfg!(feature="rendering"),"dynamic_content":false,
        "asset_repair":{"declared_images_and_code":true,"declared_libraries":true,"configured_font_diagnostics":true,"max_missing_references":1024,"allow_new_missing":false,"restore_reports_missing":true},
        "instance_overrides":true,"managed_assets":true,"attachments":false,
        "variables":{"resolved_values":true,"sources":true,"node_theme_context":true,"property_namespace":true,"pagination":true},
        "node_reads":{"calculated":cfg!(feature="rendering"),"generated":cfg!(feature="rendering"),"snapshot_reuse":cfg!(feature="rendering")},
        "validation":{"structure":true,"layout":cfg!(feature="rendering"),"diagnostic_pagination":true,"rendered_visual_check":false},
        "script":{"execution":true,"rendering":cfg!(feature="rendering"),"schemas":crate::document::SUPPORTED_DOCUMENT_VERSIONS,"engine":crate::script::ENGINE},
        "browser":{"rendering":cfg!(feature="rendering"),"runtime_setup_required":true,"devices":["desktop","phone","tablet"],"file_url_rendering":cfg!(feature="rendering"),"file_url_requires_declared_resources":true,"resource_bundle":{"version":1,"declare":true,"pack":true,"preserve_original_urls":true}},
        "shader":{"fills":cfg!(feature="rendering"),"effects":cfg!(feature="rendering"),"software_webgl":true,"annotations":["resolution","time"],"variable_image_origins":true,"sampler_arrays":true,"max_encoded_sampler_bytes":67108864},
        "graphics":{"rounded_polygons":cfg!(feature="rendering"),"background_blur":cfg!(feature="rendering"),"node_preview":"document_crop","inner_shadow_blending":cfg!(feature="rendering"),"outer_shadow_blending":cfg!(feature="rendering"),"linear_color_blending":cfg!(feature="rendering"),"mesh_gradient":cfg!(feature="rendering")},
        "icons":{"node_type":"icon","packs":crate::icons::catalogue().iter().map(|pack| &pack.id).collect::<Vec<_>>(),"bundled":["lucide"],"rendering":cfg!(feature="rendering"),"synthetic_weights":false,"variable_weight_range":[100,700],"discovery_tool":"list_icons"},
        "fonts":{"rendering":cfg!(feature="rendering"),"formats":["ttf","otf","ttc","otc"],"face_metadata":true,"variation_control":["wght"],"fallback_policy":"project configuration or requested family, bundled default, explicit files","project_configuration":true},
        "detach_instance":{"local_stored":true,"external":true,"inherited":true,"preserve_nested_refs":true,"external_scripts":true,"script_scope_metadata_version":1,"shader_sampler_variables":true,"checked_semantics":true},
        "import_component":{"modes":["link","copy"],"sources":["local","declared_library","local_file","bundled_style"],"dry_run_writes":false,"copy_scripts":false,"copy_string_sampler_variables":true,"copy_absolute_string_sampler_paths":false,"sampler_copy_format":"imported_literal_variables","reuse_contained_definitions":false}
    });
    #[cfg(feature = "rendering")]
    {
        result["capabilities"]["graphics"]["blend_modes"] = json!(crate::render::BLEND_MODES);
    }
    match Project::from_document_with_data(store.path(), document, &[], Some(store.data_dir())) {
        Ok(mut project) => {
            project.add_font_files(font_files)?;
            result["configuration"] = project.configuration();
            result["dependency_revision"] = json!(project.dependency_revision());
            result["dependencies"] = json!(project.dependencies());
            asset_status(&mut result, &project);
            if let Some(error) = project.resolution_error() {
                result["resolution_error"] = json!(error);
            }
        }
        Err(error) => result["dependency_error"] = json!(error),
    }
    Ok(result)
}

#[cfg(test)]
mod import_write_tests {
    use super::*;
    #[test]
    fn an_asset_appearing_during_preparation_prevents_document_and_history_writes() {
        let temp = tempfile::TempDir::new().unwrap();
        let store =
            Store::init(temp.path().join("main.pen"), Some(temp.path().join("data"))).unwrap();
        let mut before = store.read().unwrap();
        before.insert_element("/children",0,&json!({"id":"image","type":"rectangle","width":20,"height":20,"fill":{"type":"image","url":"images/lost.png"}}).to_string()).unwrap();
        std::fs::write(store.path(), before.text()).unwrap();
        let project = Project::load(&store).unwrap();
        let operations = serde_json::from_value::<Vec<operations::Operation>>(
            json!([{"op":"update","target":"image","set":{"width":30}}]),
        )
        .unwrap();
        let conditions = WriteConditions {
            allow_missing_assets: false,
            request_id: "racing-repair",
            revision: &before.revision(),
            dependency: Some(&project.dependency_revision()),
            runtime_project: None,
            cancel: None,
            fingerprint: "racing-repair".into(),
        };
        let error = save_project(&store, &conditions, &[], |document| {
            let prepared = operations::prepare_in_project(
                document,
                &operations,
                store.path(),
                store.data_dir(),
                None,
            )?;
            std::fs::create_dir(temp.path().join("images")).unwrap();
            std::fs::write(temp.path().join("images/lost.png"), b"concurrently created").unwrap();
            Ok(prepared)
        })
        .unwrap_err();
        assert_eq!(error.code, "REVISION_CONFLICT");
        assert_eq!(store.read().unwrap().text(), before.text());
        assert!(store.history(10, None).unwrap().is_empty());
    }
    #[test]
    fn cancellation_after_import_preparation_prevents_files_history_and_document_writes() {
        let temp = tempfile::TempDir::new().unwrap();
        let store =
            Store::init(temp.path().join("main.pen"), Some(temp.path().join("data"))).unwrap();
        let before = store.read().unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        let operations = serde_json::from_value::<Vec<operations::Operation>>(json!([
            {"op":"import_component","mode":"link","source":{"style_id":"standard-ui","node_id":"button"}}
        ])).unwrap();
        let conditions = WriteConditions {
            allow_missing_assets: false,
            request_id: "cancelled-import",
            revision: &before.revision(),
            dependency: None,
            runtime_project: None,
            cancel: Some(&cancel),
            fingerprint: "cancelled-import".into(),
        };
        let error = save_project(&store, &conditions, &[], |document| {
            let prepared = operations::prepare_in_project(
                document,
                &operations,
                store.path(),
                store.data_dir(),
                Some(&cancel),
            )?;
            assert!(!prepared.files.is_empty());
            assert!(
                !prepared.document.value()["children"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
            cancel.cancel();
            Ok(prepared)
        })
        .unwrap_err();
        assert_eq!(error.code, "CANCELLED");
        assert_eq!(store.read().unwrap().text(), before.text());
        assert!(store.history(10, None).unwrap().is_empty());
        assert!(!temp.path().join("assets").exists());
        let request = EditRequest {
            request_id: "fresh-import".into(),
            expected_revision: before.revision(),
            expected_dependency_revision: None,
            expected_runtime_dependency_revision: None,
            operations,
            dry_run: false,
        };
        apply_request_with_cancellation(
            &store,
            &request,
            &[],
            Some(&tokio_util::sync::CancellationToken::new()),
        )
        .unwrap();
        assert_eq!(store.history(10, None).unwrap().len(), 1);
        assert!(temp.path().join("assets/lazypencil").is_dir());
    }

    #[test]
    fn an_unused_source_asset_changed_during_copy_preparation_prevents_publication() {
        let temp = tempfile::TempDir::new().unwrap();
        let store =
            Store::init(temp.path().join("main.pen"), Some(temp.path().join("data"))).unwrap();
        let image = temp.path().join("unused.png");
        std::fs::write(&image, b"\x89PNG\r\n\x1a\noriginal image snapshot").unwrap();
        std::fs::write(temp.path().join("ui.lib.pen"),json!({"version":"2.21","children":[{"id":"image","type":"rectangle","reusable":true,"width":10,"height":10,"fill":{"type":"image","url":"unused.png"}}]}).to_string()).unwrap();
        let mut before = store.read().unwrap();
        before
            .set_property("", "imports", &json!({"ui":"ui.lib.pen"}))
            .unwrap();
        std::fs::write(store.path(), before.text()).unwrap();
        let project = Project::load(&store).unwrap();
        assert!(project.dependencies().iter().all(|d| d.kind != "image"));
        let operations:Vec<operations::Operation>=serde_json::from_value(json!([
            {"op":"import_component","mode":"copy","source":{"library_alias":"ui","node_id":"image"}},
            {"op":"remove_import","alias":"ui"}
        ])).unwrap();
        let conditions = WriteConditions {
            allow_missing_assets: false,
            request_id: "racing-copy",
            revision: &before.revision(),
            dependency: Some(&project.dependency_revision()),
            runtime_project: None,
            cancel: None,
            fingerprint: "racing-copy".into(),
        };
        let error = save_project(&store, &conditions, &[], |document| {
            let prepared = operations::prepare_in_project(
                document,
                &operations,
                store.path(),
                store.data_dir(),
                None,
            )?;
            std::fs::write(&image, b"a concurrent external write").unwrap();
            Ok(prepared)
        })
        .unwrap_err();
        assert_eq!(error.code, "REVISION_CONFLICT");
        assert_eq!(store.read().unwrap().text(), before.text());
        assert!(store.history(10, None).unwrap().is_empty());
        assert!(!temp.path().join("assets").exists());
    }

    #[test]
    fn an_oversized_report_does_not_commit_a_change_the_mcp_client_cannot_receive() {
        let temp = tempfile::TempDir::new().unwrap();
        let store =
            Store::init(temp.path().join("main.pen"), Some(temp.path().join("data"))).unwrap();
        let before = store.read().unwrap();
        let conditions = WriteConditions {
            allow_missing_assets: false,
            request_id: "oversized-report",
            revision: &before.revision(),
            dependency: None,
            runtime_project: None,
            cancel: None,
            fingerprint: "oversized-report".into(),
        };
        let error = save_project(&store, &conditions, &[], |document| {
            let mut after = document.clone();
            after.set_property("", "metadata", &json!({"changed":true}))?;
            Ok(operations::PreparedBatch::plain(
                after,
                json!({"report":"x".repeat(4*1024*1024)}),
            ))
        })
        .unwrap_err();
        assert_eq!(error.code, "INVALID_ARGUMENT");
        assert_eq!(store.read().unwrap().text(), before.text());
        assert!(store.history(10, None).unwrap().is_empty());
    }
}

pub fn validate_document(store: &Store) -> Result<Value> {
    validate_document_with_options(store, &[], &ValidateRequest::default(), None)
}

#[cfg(feature = "rendering")]
pub fn layout_snapshot(
    store: &Store,
    font_files: &[std::path::PathBuf],
    expected_revision: Option<&str>,
    expected_dependency_revision: Option<&str>,
) -> Result<(Project, crate::layout::LayoutSnapshot)> {
    layout_snapshot_with_runtime(
        store,
        font_files,
        expected_revision,
        expected_dependency_revision,
        &mut crate::script::ScriptRuntime::current()?,
    )
}

#[cfg(feature = "rendering")]
pub fn layout_snapshot_with_runtime(
    store: &Store,
    font_files: &[std::path::PathBuf],
    expected_revision: Option<&str>,
    expected_dependency_revision: Option<&str>,
    runtime: &mut crate::script::ScriptRuntime,
) -> Result<(Project, crate::layout::LayoutSnapshot)> {
    layout_snapshot_observing_failure(
        store,
        font_files,
        expected_revision,
        expected_dependency_revision,
        runtime,
        |_, _, _| {},
    )
}

#[cfg(feature = "rendering")]
pub(crate) fn layout_snapshot_observing_failure(
    store: &Store,
    font_files: &[std::path::PathBuf],
    expected_revision: Option<&str>,
    expected_dependency_revision: Option<&str>,
    runtime: &mut crate::script::ScriptRuntime,
    failed: impl FnOnce(&Project, &[crate::project::Dependency], &Error),
) -> Result<(Project, crate::layout::LayoutSnapshot)> {
    let cancel = runtime.cancellation();
    let mut project = project_with_overlay_and_cancellation(
        store,
        store.read()?,
        font_files,
        &Default::default(),
        cancel.as_ref(),
    )?;
    if let Some(revision) = expected_revision {
        store::require_revision(project.root_document(), revision)?;
    }
    if expected_dependency_revision.is_some() {
        project.require_dependencies(expected_dependency_revision)?;
    }
    crate::cancel::check(cancel.as_ref())?;
    let mut fonts = project.load_fonts()?;
    crate::cancel::check(cancel.as_ref())?;
    let declared = project.dependencies();
    let snapshot = match crate::script::compute(&mut project, &mut fonts, runtime) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            failed(&project, &declared, &error);
            return Err(error);
        }
    };
    store::require_revision(&store.read()?, &snapshot.revision)?;
    project.verify_unchanged()?;
    crate::cancel::check(cancel.as_ref())?;
    // Retained snapshots must not inherit the lifetime of a finished request.
    project.set_cancellation(None);
    Ok((project, snapshot))
}

pub fn read_nodes(
    document: &Document,
    id: Option<&str>,
    name: Option<&str>,
    kind: Option<&str>,
    depth: usize,
    limit: usize,
    cursor: Option<&str>,
) -> Result<Value> {
    if depth > 16 || !(1..=500).contains(&limit) {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Node depth must be 0..16 and limit 1..500",
        ));
    }
    let revision = document.revision();
    let query_hash = crate::document::hash(
        json!({"id":id,"name":name,"type":kind,"depth":depth})
            .to_string()
            .as_bytes(),
    );
    let offset = if let Some(cursor) = cursor {
        let parts = cursor.split(':').collect::<Vec<_>>();
        if parts.len() != 3 {
            return Err(Error::new("INVALID_ARGUMENT", "Malformed cursor"));
        }
        let (expected, query, offset) = (parts[0], parts[1], parts[2]);
        if expected != revision {
            return Err(Error::new(
                "REVISION_CONFLICT",
                "This node cursor belongs to an older revision",
            ));
        }
        if query != query_hash {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "This cursor belongs to a different node query",
            ));
        }
        offset
            .parse::<usize>()
            .map_err(|_| Error::new("INVALID_ARGUMENT", "Malformed cursor offset"))?
    } else {
        0
    };
    let mut nodes = Vec::new();
    if let Some(id) = id {
        nodes.push(document.node(id)?);
    } else {
        collect_nodes(&document.value()["children"], &mut nodes);
    }
    nodes.retain(|node| {
        name.is_none_or(|q| {
            node["name"]
                .as_str()
                .is_some_and(|s| s.to_lowercase().contains(&q.to_lowercase()))
        }) && kind.is_none_or(|t| node["type"] == t)
    });
    if offset > nodes.len() {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Cursor offset is out of range",
        ));
    }
    let total = nodes.len();
    let nodes = nodes
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|node| truncate_node(node, depth))
        .collect::<Vec<_>>();
    let next = if offset + nodes.len() < total {
        Some(format!("{revision}:{query_hash}:{}", offset + nodes.len()))
    } else {
        None
    };
    let result =
        json!({"revision":revision,"nodes":nodes,"next_cursor":next,"total_matches":total});
    if result.to_string().len() > 4 * 1024 * 1024 {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Node response exceeds 4 MiB; reduce depth or limit",
        ));
    }
    Ok(result)
}

#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadNodesRequest {
    /// Execute scripts and inspect the resolved tree used by layout and rendering.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub calculated: bool,
    /// Reuse this MCP connection's exact retained calculation without rerunning scripts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub instance_path: Vec<String>,
    #[serde(default)]
    pub library_alias: Option<String>,
    #[serde(default)]
    pub expand_instances: bool,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default = "node_depth")]
    pub depth: usize,
    #[serde(default = "node_limit")]
    pub limit: usize,
    #[serde(default)]
    pub cursor: Option<String>,
}

fn node_depth() -> usize {
    1
}
fn node_limit() -> usize {
    100
}

pub fn read_store_nodes(
    store: &Store,
    fonts: &[PathBuf],
    request: &ReadNodesRequest,
) -> Result<Value> {
    read_store_nodes_with_cancellation(store, fonts, request, None)
}

pub fn read_store_nodes_with_cancellation(
    store: &Store,
    fonts: &[PathBuf],
    request: &ReadNodesRequest,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<Value> {
    crate::cancel::check(cancel)?;
    if request.calculated || request.snapshot_id.is_some() {
        #[cfg(not(feature = "rendering"))]
        return Err(Error::new(
            "UNSUPPORTED_FEATURE",
            "Calculated node reads require the rendering feature",
        ));
        #[cfg(feature = "rendering")]
        {
            validate_calculated_nodes_request(request)?;
            if request.snapshot_id.is_some() {
                return Err(Error::new(
                    "SNAPSHOT_UNAVAILABLE",
                    "Calculation IDs are retained only by their MCP connection; use calculated:true to compute a fresh CLI result",
                ));
            }
            let mut runtime = crate::script::ScriptRuntime::current()?;
            runtime.set_cancellation(cancel.cloned());
            let (project, snapshot) =
                layout_snapshot_with_runtime(store, fonts, None, None, &mut runtime)?;
            let result = read_calculated_nodes(&project, &snapshot, request, cancel)?;
            store::require_revision(&store.read()?, &snapshot.revision)?;
            return Ok(result);
        }
    }
    let document = store.read()?;
    let result = if request.library_alias.is_none()
        && !request.expand_instances
        && request.instance_path.is_empty()
    {
        let mut result = read_nodes(
            &document,
            request.node_id.as_deref(),
            request.name.as_deref(),
            request.kind.as_deref(),
            request.depth,
            request.limit,
            request.cursor.as_deref(),
        )?;
        let metadata = document_metadata_with_fonts(store, fonts)?;
        result["dependency_revision"] = metadata["dependency_revision"].clone();
        if let Some(error) = metadata.get("dependency_error") {
            result["dependency_error"] = error.clone();
        }
        for key in ["assets_complete", "asset_diagnostics", "resolution_error"] {
            if let Some(value) = metadata.get(key) {
                result[key] = value.clone();
            }
        }
        result["view"] = json!("stored");
        result
    } else {
        let project = project_with_fonts(store, document.clone(), fonts)?;
        let mut result = read_project_nodes(&project, request)?;
        asset_status(&mut result, &project);
        result
    };
    store::require_revision(&store.read()?, &document.revision())?;
    Ok(result)
}

/// Read one immutable project snapshot, including inherited and library nodes.
/// Expanded properties retain variable references; layout supplies computed values.
pub fn read_project_nodes(project: &Project, request: &ReadNodesRequest) -> Result<Value> {
    if request.calculated || request.snapshot_id.is_some() {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Calculated reads require a layout snapshot; use the store or MCP read_nodes entry point",
        ));
    }
    if request.depth > 16 || !(1..=500).contains(&request.limit) {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Node depth must be 0..16 and limit 1..500",
        ));
    }
    if request.instance_path.len() > 128
        || request
            .instance_path
            .iter()
            .any(|p| p.is_empty() || p.contains('/'))
        || (!request.instance_path.is_empty() && request.node_id.is_none())
    {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "An instance path requires a node ID and at most 128 nonempty IDs without '/'",
        ));
    }
    let mut query = serde_json::to_value(request).expect("serializable query");
    query.as_object_mut().expect("object").remove("cursor");
    query.as_object_mut().expect("object").remove("limit");
    let identity = crate::document::hash(json!({
        "revision":project.root_document().revision(),"dependency_revision":project.dependency_revision(),"query":query
    }).to_string().as_bytes());
    let cursor = request
        .cursor
        .as_deref()
        .map(|cursor| {
            let (expected, offset) = cursor
                .split_once('.')
                .ok_or_else(|| Error::new("INVALID_ARGUMENT", "Malformed node cursor"))?;
            if expected != identity {
                return Err(Error::new(
                    "REVISION_CONFLICT",
                    "The node query, document, or a dependency changed; restart this query",
                ));
            }
            Ok(offset)
        })
        .transpose()?;
    let expanded = request.expand_instances || !request.instance_path.is_empty();
    let mut result = if expanded {
        let roots = if let Some(id) = &request.node_id {
            vec![project.expanded_library_node(
                request.library_alias.as_deref(),
                &crate::project::NodeAddress {
                    node_id: id.clone(),
                    instance_path: request.instance_path.clone(),
                },
            )?]
        } else {
            project.expanded_library_roots(request.library_alias.as_deref())?
        };
        let mut nodes = Vec::new();
        for root in &roots {
            if request.node_id.is_some() {
                nodes.push(root);
            } else {
                collect_expanded(root, &mut nodes);
            }
        }
        nodes.retain(|node| {
            request
                .kind
                .as_deref()
                .is_none_or(|kind| node.kind() == kind)
                && request.name.as_deref().is_none_or(|name| {
                    node.properties
                        .get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|value| value.to_lowercase().contains(&name.to_lowercase()))
                })
        });
        let offset = cursor
            .unwrap_or("0")
            .parse::<usize>()
            .map_err(|_| Error::new("INVALID_ARGUMENT", "Malformed node cursor offset"))?;
        if offset > nodes.len() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Cursor offset is out of range",
            ));
        }
        let total = nodes.len();
        let page = nodes
            .into_iter()
            .skip(offset)
            .take(request.limit)
            .map(|node| truncate_expanded(node, request.depth))
            .collect::<Vec<_>>();
        let next = (offset + page.len() < total).then(|| (offset + page.len()).to_string());
        json!({"nodes":page,"next_cursor":next,"total_matches":total,"view":"expanded"})
    } else {
        read_nodes(
            project.library_document(request.library_alias.as_deref())?,
            request.node_id.as_deref(),
            request.name.as_deref(),
            request.kind.as_deref(),
            request.depth,
            request.limit,
            cursor,
        )?
    };
    if let Some(next) = result["next_cursor"].as_str() {
        result["next_cursor"] = json!(format!("{identity}.{next}"));
    }
    result["revision"] = json!(project.root_document().revision());
    result["dependency_revision"] = json!(project.dependency_revision());
    result["library_alias"] = json!(request.library_alias);
    if !expanded {
        result["view"] = json!("stored");
    }
    if result.to_string().len() > 4 * 1024 * 1024 {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Node response exceeds 4 MiB; reduce depth or limit",
        ));
    }
    project.verify_unchanged()?;
    Ok(result)
}

fn collect_expanded<'a>(
    node: &'a crate::project::ResolvedNode,
    result: &mut Vec<&'a crate::project::ResolvedNode>,
) {
    result.push(node);
    for child in &node.children {
        collect_expanded(child, result);
    }
}

fn truncate_expanded(node: &crate::project::ResolvedNode, depth: usize) -> Value {
    let mut value = json!({"address":node.address,"source_path":node.source_path,"source_id":node.source_id,
        "is_instance":node.is_instance,"properties":node.properties,"property_sources":node.property_sources});
    if depth == 0 {
        value["children_count"] = json!(node.children.len());
    } else {
        value["children"] = json!(
            node.children
                .iter()
                .map(|child| truncate_expanded(child, depth - 1))
                .collect::<Vec<_>>()
        );
    }
    value
}

fn collect_nodes<'a>(children: &'a Value, result: &mut Vec<&'a Value>) {
    for node in children.as_array().into_iter().flatten() {
        result.push(node);
        collect_nodes(&node["children"], result);
        if node["type"] == "ref" {
            collect_override_nodes(node, result);
        }
    }
}

fn collect_override_nodes<'a>(node: &'a Value, result: &mut Vec<&'a Value>) {
    for value in node
        .get("descendants")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(serde_json::Map::values)
    {
        if value.get("type").is_some() {
            result.push(value);
            collect_nodes(&value["children"], result);
            if value["type"] == "ref" {
                collect_override_nodes(value, result);
            }
        } else {
            collect_nodes(&value["children"], result);
            collect_override_nodes(value, result);
        }
    }
}

fn truncate_node(node: &Value, depth: usize) -> Value {
    truncate_payload(node, depth, false)
}

fn truncate_payload(node: &Value, depth: usize, is_override: bool) -> Value {
    let has_overrides = is_override || node["type"] == "ref";
    let mut result = Value::Object(
        node.as_object()
            .expect("indexed node")
            .iter()
            .filter(|(key, _)| {
                key.as_str() != "children" && (!has_overrides || key.as_str() != "descendants")
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    );
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        if depth == 0 {
            result
                .as_object_mut()
                .expect("indexed node")
                .remove("children");
            result["children_count"] = json!(children.len());
        } else {
            result["children"] = json!(
                children
                    .iter()
                    .map(|n| truncate_node(n, depth - 1))
                    .collect::<Vec<_>>()
            );
        }
    }
    if has_overrides && let Some(overrides) = node.get("descendants").and_then(Value::as_object) {
        if depth == 0 {
            result["descendant_overrides_count"] = json!(overrides.len());
        } else {
            result["descendants"] = Value::Object(
                overrides
                    .iter()
                    .map(|(key, value)| (key.clone(), truncate_payload(value, depth - 1, true)))
                    .collect(),
            );
        }
    }
    result
}

#[cfg(all(test, feature = "rendering"))]
mod write_tests {
    use super::*;

    #[test]
    fn a_runtime_asset_changed_during_edit_preparation_prevents_saving() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("scene.pen");
        let image = temp.path().join("generated.png");
        std::fs::write(&image, b"captured image").unwrap();
        std::fs::write(
            temp.path().join("draw.js"),
            "/** @schema 2.21 */ return [];",
        )
        .unwrap();
        std::fs::write(&path, r#"{"version":"2.21","children":[{"id":"script","type":"script","scriptUri":"draw.js","width":20,"height":20}]}"#).unwrap();
        let store = Store::open(&path, Some(temp.path().join("data"))).unwrap();
        let mut project = Project::load(&store).unwrap();
        let script = project.resolve().unwrap().roots.remove(0);
        project.generated_children(&script, &[json!({"type":"rectangle","width":20,"height":20,"fill":{"type":"image","url":"generated.png"}})]).unwrap();
        assert_eq!(project.runtime_dependencies().len(), 1);
        let before = store.read().unwrap();
        let conditions = WriteConditions {
            allow_missing_assets: false,
            request_id: "racing-runtime-asset",
            revision: &before.revision(),
            dependency: Some(&project.dependency_revision()),
            runtime_project: Some(&project),
            fingerprint: "test-guard".into(),
            cancel: None,
        };
        let error = save_project(&store, &conditions, &[], |document| {
            std::fs::write(&image, b"concurrent external edit").unwrap();
            let mut after = document.clone();
            after
                .set_property("/children/0", "name", &json!("Changed"))
                .unwrap();
            Ok(operations::PreparedBatch::plain(after, json!({})))
        })
        .unwrap_err();
        assert_eq!(error.code, "REVISION_CONFLICT");
        assert_eq!(store.read().unwrap().text(), before.text());
        assert!(store.history(10, None).unwrap().is_empty());
    }
}
