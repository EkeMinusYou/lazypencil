use super::{ENGINE, ScriptOutput, ScriptRequest, ScriptRuntime, header::Header};
use crate::{
    Error, Result,
    document::hash,
    geometry::Rect,
    layout::{self, LayoutSnapshot},
    project::{Project, ResolvedNode},
    text::Fonts,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

struct Cached {
    input_key: String,
    width: f64,
    height: f64,
    output: ScriptOutput,
    metadata: Value,
}

/// Resolve generated children and their dimensions to a bounded fixed point.
pub fn compute(
    project: &mut Project,
    fonts: &mut Fonts,
    runtime: &mut ScriptRuntime,
) -> Result<LayoutSnapshot> {
    let settings = project.script_settings();
    runtime.configure(settings)?;
    let cancel = runtime.cancellation();
    crate::cancel::check(cancel.as_ref())?;
    let mut scene = project.resolve()?;
    crate::cancel::check(cancel.as_ref())?;
    if !scene.roots.iter().any(has_enabled_script) {
        return layout::compute_with_cancellation(scene, fonts, cancel.as_ref());
    }
    for node in &scene.roots {
        validate_initial_scripts(node)?;
    }
    let mut cached = BTreeMap::new();
    let mut runs = 0;
    let deadline = Instant::now() + Duration::from_millis(settings.layout_timeout_ms);
    let check = || {
        crate::cancel::check(cancel.as_ref())?;
        require_budget(deadline)
    };
    for _ in 0..settings.max_passes {
        check()?;
        let mut count = 0;
        for node in &scene.roots {
            validate_tree(node, 0, &mut count)?;
        }
        let mut snapshot = layout::compute_checked(scene.clone(), fonts, &check)?;
        check()?;
        let sizes = snapshot
            .items
            .iter()
            .map(|item| (item.record.id.clone(), item.record.rect))
            .collect::<BTreeMap<_, _>>();
        let mut changed = false;
        for node in &mut scene.roots {
            expand(
                node,
                true,
                &sizes,
                project,
                runtime,
                &mut cached,
                &mut runs,
                &mut changed,
                deadline,
            )?;
        }
        if !changed {
            cached.retain(|id, _| {
                snapshot
                    .items
                    .iter()
                    .any(|item| item.record.enabled && item.record.id == *id)
            });
            snapshot.runtime_dependencies = project.runtime_dependencies();
            snapshot.script_executions = cached
                .values()
                .map(|cached| cached.metadata.clone())
                .collect();
            if !snapshot.script_executions.is_empty() {
                snapshot.id=hash(json!({"layout":snapshot.id,"script_engine":ENGINE,"executions":snapshot.script_executions,"runtime_dependencies":snapshot.runtime_dependencies}).to_string().as_bytes());
            }
            project.verify_unchanged()?;
            check()?;
            return Ok(snapshot);
        }
    }
    Err(Error::new(
        "LAYOUT_UNRESOLVED",
        format!(
            "Script-generated layout did not converge within {} passes",
            settings.max_passes
        ),
    ))
}

fn require_budget(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        return Err(Error::new(
            "RENDER_TIMEOUT",
            "Script layout exceeded its configured evaluation budget",
        ));
    }
    Ok(())
}

fn has_enabled_script(node: &ResolvedNode) -> bool {
    node.properties.get("enabled") != Some(&Value::Bool(false))
        && (node.kind() == "script" || node.children.iter().any(has_enabled_script))
}

fn validate_initial_scripts(node: &ResolvedNode) -> Result<()> {
    if node.properties.get("enabled") == Some(&Value::Bool(false)) {
        return Ok(());
    }
    if node.kind() == "script" && !node.children.is_empty() {
        return Err(Error::new(
            "INVALID_DOCUMENT",
            "Script children are derived and cannot be stored in the document",
        ));
    }
    for child in &node.children {
        validate_initial_scripts(child)?;
    }
    Ok(())
}

fn validate_tree(node: &ResolvedNode, depth: usize, count: &mut usize) -> Result<()> {
    *count += 1;
    if depth > 256 || *count > crate::document::MAX_NODES {
        return Err(Error::new(
            "LAYOUT_UNRESOLVED",
            "Generated scene exceeds its node or depth limit",
        ));
    }
    for child in &node.children {
        validate_tree(child, depth + 1, count)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn expand(
    node: &mut ResolvedNode,
    inherited_enabled: bool,
    sizes: &BTreeMap<String, Rect>,
    project: &mut Project,
    runtime: &mut ScriptRuntime,
    cached: &mut BTreeMap<String, Cached>,
    runs: &mut usize,
    changed: &mut bool,
    deadline: Instant,
) -> Result<()> {
    require_budget(deadline)?;
    let enabled = inherited_enabled && node.properties.get("enabled") != Some(&Value::Bool(false));
    if !enabled {
        return Ok(());
    }
    if node.kind() == "script" {
        let id = node.id();
        let Some(rect) = sizes.get(&id) else {
            *changed = true;
            return Ok(());
        };
        let uri = node
            .properties
            .get("scriptUri")
            .and_then(Value::as_str)
            .filter(|uri| !uri.is_empty())
            .ok_or_else(|| {
                Error::new(
                    "SCRIPT_EXECUTION_FAILED",
                    "Script node is missing scriptUri",
                )
                .details(json!({"node":node.address}))
            })?;
        let path = project.resolve_file(node.property_source("scriptUri"), uri)?;
        let asset = project.asset(&path)?;
        let source_hash = asset.revision.clone();
        let context = |error| super::diagnostic::context(error, &node.address, &path, &source_hash);
        let code = std::str::from_utf8(&asset.bytes)
            .map_err(|_| {
                context(Error::new(
                    "SCRIPT_EXECUTION_FAILED",
                    "Script source must be UTF-8",
                ))
            })?
            .to_string();
        let header = Header::parse(&code).map_err(context)?;
        let mut inputs = node
            .properties
            .get("inputs")
            .map(|value| {
                value.as_object().cloned().ok_or_else(|| {
                    Error::new("SCRIPT_EXECUTION_FAILED", "Script inputs must be an object")
                })
            })
            .transpose()
            .map_err(context)?
            .unwrap_or_default();
        if !node.script_inputs_resolved {
            inputs = project
                .resolve_script_input_values(node, &header, &inputs)
                .map_err(context)?;
            if node.properties.contains_key("inputs") {
                node.properties
                    .insert("inputs".into(), Value::Object(inputs.clone()));
            }
            node.script_inputs_resolved = true;
        }
        let inputs = header.prepare(&inputs).map_err(context)?;
        let inputs = project
            .prepare_script_refs(node, &header, inputs)
            .map_err(context)?;
        let bindings_revision = node.script_bindings_revision();
        let input_key=hash(json!({"source":source_hash,"inputs":inputs,"origin":node.script_origin(),"input_origin":node.script_input_origin(),"bindings_revision":bindings_revision,"theme":node.theme,"limits":runtime.settings()}).to_string().as_bytes());
        let (width, height) = (rect.width as f64, rect.height as f64);
        let valid = cached.get(&id).is_some_and(|entry: &Cached| {
            entry.input_key == input_key
                && (!entry.output.reads_width || entry.width == width)
                && (!entry.output.reads_height || entry.height == height)
        });
        if !valid {
            *runs += 1;
            if *runs > runtime.settings().max_evaluations {
                return Err(Error::new(
                    "RENDER_TIMEOUT",
                    format!(
                        "A calculation may execute at most {} script evaluations",
                        runtime.settings().max_evaluations
                    ),
                ));
            }
            let output = runtime
                .execute_until(
                    &ScriptRequest {
                        code,
                        width,
                        height,
                        inputs: inputs.clone(),
                    },
                    Some(deadline),
                )
                .map_err(context)?;
            for (key, reads) in [
                ("width", output.reads_width),
                ("height", output.reads_height),
            ] {
                if reads && node.properties.get(key).and_then(Value::as_str) == Some("fit_content")
                {
                    return Err(Error::new("LAYOUT_UNRESOLVED",format!("Script reads its content-sized {key}; provide an explicit fallback to resolve this dependency"))
                        .details(json!({"node":node.address,"axis":key})));
                }
            }
            node.children = project.generated_children(node, &output.nodes)?;
            let metadata = json!({"node_id":id,"source_path":path,"source_hash":source_hash,"bindings_revision":bindings_revision,"width":width,"height":height,
                "inputs_hash":hash(serde_json::to_string(&inputs).expect("inputs").as_bytes()),"output_hash":hash(serde_json::to_string(&output.nodes).expect("nodes").as_bytes()),
                "output_root_count":output.nodes.len(),"engine":ENGINE,"limits":runtime.settings(),"reads_width":output.reads_width,"reads_height":output.reads_height});
            cached.insert(
                id,
                Cached {
                    input_key,
                    width,
                    height,
                    output,
                    metadata,
                },
            );
            *changed = true;
            return Ok(());
        }
    }
    for child in &mut node.children {
        expand(
            child, enabled, sizes, project, runtime, cached, runs, changed, deadline,
        )?;
    }
    Ok(())
}
