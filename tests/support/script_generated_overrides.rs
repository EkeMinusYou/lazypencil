use super::*;
use crate::script_copy::fixture;

#[path = "script_generated_overrides/cases.rs"]
mod cases;
use cases::{Case, Combined, Context, Inherited, Mode, NativeScope, Owner, Style};

const REPLACEMENT: &str = r##"/**
 * @schema 2.21
 * @input selected: ref = "badge"
 */
if ('stale' in pencil.input) throw new Error('Stale control leaked');
return [{type:'ref',ref:pencil.input.selected},
 {type:'rectangle',x:12,width:10,height:10,fill:'$tone'},
 {type:'ref',x:24,ref:'badge'}];
"##;

fn setup(version: &str, case: &Case<'_>, count: usize) -> (TempDir, Store, String) {
    let mode = case.mode;
    let style = case.style;
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    fs::create_dir(temp.path().join("library")).unwrap();
    fs::write(
        temp.path().join("library/base.js"),
        "/* @schema 2.21 */ return [{type:'rectangle',width:36,height:12,fill:'#0000ff'}];",
    )
    .unwrap();
    fs::write(temp.path().join("library/override.js"), REPLACEMENT).unwrap();
    // A tempting caller-side filename must never replace an unmanifested source.
    fs::write(
        temp.path().join("override.js"),
        "/* @schema 2.21 */ return [{type:'rectangle',width:36,height:12,fill:'#ff00ff'}];",
    )
    .unwrap();
    let mut changes = json!({"scriptUri":"override.js"});
    match mode {
        Mode::EmittedCode(variant) => {
            changes.as_object_mut().unwrap().remove("scriptUri");
            let mut metadata = json!({"type":"test","keep":"generated","literal":"$tone"});
            let stale = json!({"version":1,"script_uri":"old.js","library":"uncaptured"});
            match variant {
                Owner::Keep => (),
                Owner::Scope => metadata["lazypencil"]["script_scope"] = stale,
                Owner::Binding => metadata["lazypencil"]["script_bindings"] = stale,
                Owner::Carrier => {
                    metadata = json!({"type":"lazypencil","lazypencil":{"script_override":{"version":1,"metadata":metadata,"output":stale}}})
                }
            }
            changes["metadata"] = metadata;
        }
        Mode::InheritedState(variant) => {
            let raw = json!({"selected":"$selection","stale":"$missing"});
            let hash = lazypencil::document::hash(raw.to_string().as_bytes());
            let binding = json!({"version":1,"script_uri":"override.js","components":{"badge":"badge"},"variables":{"tone":"tone"}});
            let context = |stale: bool| {
                json!({"version":1,"values":if stale {json!({"selected":"different","stale":"$missing"})} else {raw.clone()},
                "library":if stale {"uncaptured"} else {""},"variables":{"selection":"selection"},"components":{"badge":"alternate"}})
            };
            let mut metadata = json!({"type":"test","keep":"generated","literal":"$tone","lazypencil":{"script_bindings":binding}});
            if matches!(variant, Inherited::Scope | Inherited::StaleScope) {
                metadata["lazypencil"]["script_scope"] = json!({"version":1,"script_uri":"old.js","library":"uncaptured",
                    "inputs":{"hash":if variant.stale() {"0".repeat(64)} else {hash},"library":""}});
            } else if matches!(variant, Inherited::Context | Inherited::StaleContext) {
                metadata["lazypencil"]["script_input_context"] = context(variant.stale());
            } else {
                let input = if variant == Inherited::CarrierOutput {
                    None
                } else {
                    Some(context(variant != Inherited::Carrier))
                };
                let output = (variant == Inherited::CarrierOutput).then(|| json!({"version":1,"script_uri":"override.js","components":{"badge":"badge"},"variables":{"tone":"tone"},"inputs_hash":hash,"input_components":{"badge":"badge"}}));
                metadata = json!({"type":"lazypencil","lazypencil":{"script_override":{
                    "version":1,"metadata":{"type":"test","keep":"generated","literal":"$tone"},"input":input,"output":output
                }}});
            }
            if variant == Inherited::UnderlayBinding {
                changes.as_object_mut().unwrap().remove("scriptUri");
                fs::write(temp.path().join("library/base.js"), REPLACEMENT).unwrap();
            }
            changes["metadata"] = metadata;
        }
        Mode::CodeOwner { variant, .. } => {
            changes.as_object_mut().unwrap().remove("scriptUri");
            changes["inputs"] = json!({"selected":"badge"});
            let mut metadata = json!({"type":"test","keep":"generated","literal":"$tone"});
            match variant {
                Owner::Scope => {
                    metadata["lazypencil"]["script_scope"] =
                        json!({"version":1,"script_uri":"old.js","library":"uncaptured"})
                }
                Owner::Binding => {
                    metadata["lazypencil"]["script_bindings"] = json!({"version":1,"script_uri":"old.js","library":"uncaptured","components":{"badge":"missing"}})
                }
                Owner::Carrier => {
                    metadata = json!({"type":"lazypencil","lazypencil":{"script_override":{
                        "version":1,"metadata":metadata,"output":{"version":1,"script_uri":"old.js","library":"uncaptured","components":{"badge":"missing"}}
                    }}})
                }
                Owner::Keep => (),
            }
            changes["metadata"] = metadata;
        }
        Mode::InputContext { variant, carrier } => {
            let variable = variant == Context::Variable;
            let inputs = if variable {
                json!({"selected":"$selection","stale":"$missing"})
            } else {
                json!({"selected":"badge"})
            };
            changes["inputs"] = inputs.clone();
            let mut context = json!({"version":1,"values":inputs,"variables":{},"components":{"badge":"alternate"}});
            if variable {
                context["values"]["selected"] = json!("$choice");
                context["variables"] = json!({"selection":"choice","choice":"tone"});
            }
            if variant == Context::Library {
                context["library"] = json!("base");
                context["components"] = json!({"badge":"badge"});
            }
            if variant == Context::Stale {
                context["values"]["selected"] = json!("different");
                context["library"] = json!("uncaptured");
                context["variables"] = json!({"missing":"missing"});
                context["components"] = json!({"missing":"missing"});
            }
            let hash = lazypencil::document::hash(inputs.to_string().as_bytes());
            let mut namespace = json!({
                "script_input_context":context,
                "script_scope":{"version":1,"script_uri":"override.js","library":"base","inputs":{"hash":hash,"library":"base"}},
                "script_bindings":{"version":1,"script_uri":"override.js","components":{"badge":"badge"},"variables":{"tone":"tone"},"inputs_hash":hash,"input_components":{"badge":"badge"}}
            });
            if variant == Context::Scope {
                namespace.as_object_mut().unwrap().remove("script_bindings");
            }
            if variant == Context::Only {
                namespace.as_object_mut().unwrap().remove("script_scope");
                namespace.as_object_mut().unwrap().remove("script_bindings");
            }
            changes["metadata"] =
                json!({"type":"test","keep":"generated","literal":"$tone","lazypencil":namespace});
            if carrier {
                if variant == Context::MetadataOutput {
                    changes.as_object_mut().unwrap().remove("scriptUri");
                    fs::write(temp.path().join("library/base.js"), REPLACEMENT).unwrap();
                }
                if variant == Context::Nested {
                    changes["metadata"]["lazypencil"]["script_override"] =
                        json!({"version":999,"opaque":"keep"});
                }
                let metadata = changes["metadata"].take();
                let output = [Context::Output, Context::Both, Context::StaleOutput, Context::MetadataOutput].contains(&variant).then(|| json!({
                    "version":1,"script_uri":if variant == Context::StaleOutput {"old.js"} else if variant == Context::MetadataOutput {"base.js"} else {"override.js"},
                    "library":if variant == Context::StaleOutput {"uncaptured"} else {"base"},
                    "components":{"badge":"badge"},"variables":{"tone":"tone"}
                }));
                let input = [Context::Input, Context::Both, Context::StaleInput].contains(&variant).then(|| json!({
                    "version":1,"library":if variant == Context::StaleInput {"uncaptured"} else {"base"},
                    "values":{"selected":if variant == Context::StaleInput {"different"} else {"badge"}},
                    "variables":{},"components":{"badge":"badge"}
                }));
                changes["metadata"] = json!({"type":"lazypencil","lazypencil":{"script_override":{
                    "version":1,"metadata":metadata,"output":output,"input":input
                }}});
            }
        }
        Mode::Combined { variant, .. } => {
            let inherited = variant.inherited();
            let inputs = if inherited {
                json!({"selected":"$selection","stale":"$missing"})
            } else {
                let inputs = json!({"selected":"badge"});
                changes["inputs"] = inputs.clone();
                inputs
            };
            let hash = lazypencil::document::hash(inputs.to_string().as_bytes());
            let stale_inputs = variant == Combined::StaleInputs;
            let stale_binding = variant == Combined::StaleBindingOutput;
            let stale_scope = variant == Combined::StaleScopeOutput;
            changes["metadata"] = json!({"type":"test","keep":"generated","literal":"$tone","lazypencil":{
                "script_scope":{"version":1,"script_uri":if stale_scope {"old.js"} else {"override.js"},"library":if stale_scope {"uncaptured"} else {"base"},
                    "inputs":{"hash":if stale_inputs {"a".repeat(64)} else {hash.clone()},"library":if variant == Combined::InheritedScopeInput {""} else {"base"}}},
                "script_bindings":{"version":1,"script_uri":if stale_binding {"old.js"} else {"override.js"},"library":if stale_binding {"uncaptured"} else {""},
                    "components":{"badge":"badge"},"variables":{"tone":"tone"},
                    "inputs_hash":if stale_inputs || [Combined::ScopeInput,Combined::StaleScopeOutput,Combined::InheritedScopeInput].contains(&variant) {"b".repeat(64)} else {hash},
                    "input_components":{"badge":"alternate"}}
            }});
        }
        Mode::NativeInput {
            inherited,
            stale,
            scope,
            ..
        } => {
            let selected = if inherited != stale {
                "$selection"
            } else {
                "$native_selection"
            };
            let guarded = if inherited {
                json!({"selected":selected,"stale":"$missing"})
            } else {
                changes["inputs"] = json!({"selected":"$selection"});
                json!({"selected":selected})
            };
            let hash = lazypencil::document::hash(guarded.to_string().as_bytes());
            let namespace = if matches!(scope, NativeScope::Scope | NativeScope::OnlyScope) {
                json!({"script_scope":{"version":1,"script_uri":if scope == NativeScope::OnlyScope {"old.js"} else {"override.js"},"library":if scope == NativeScope::OnlyScope {"uncaptured"} else {""},"inputs":{"hash":hash,"library":if inherited {""} else {"base"}}}})
            } else if scope == NativeScope::Default {
                json!({})
            } else {
                json!({"script_bindings":{"version":1,"script_uri":"override.js","components":{"badge":"badge"},"variables":{"tone":"tone"},"inputs_hash":hash,"input_components":if inherited {json!({"badge":"alternate"})} else {json!({"alternate":"badge"})}}})
            };
            changes["metadata"] =
                json!({"type":"test","keep":"generated","literal":"$tone","lazypencil":namespace});
        }
        Mode::Literal => changes["inputs"] = json!({"selected":"badge","stale":"$missing"}),
        Mode::Variable | Mode::ThemedVariable => {
            changes["inputs"] = json!({"selected":"$selection","stale":"$missing"})
        }
        Mode::BadVariable => changes["inputs"] = json!({"selected":"$missing"}),
        Mode::Null => changes["inputs"] = json!({"selected":null,"stale":"$missing"}),
        Mode::Metadata => {
            changes["metadata"] = json!({"type":"test","keep":"generated","literal":"$tone"})
        }
        Mode::ExplicitContext => {
            changes["metadata"] = json!({"type":"test","keep":"generated","literal":"$tone","lazypencil":{"script_scope":{"version":1,"script_uri":"override.js","library":"base"}}})
        }
        Mode::ExplicitInputContext => {
            let inputs = json!({"selected":"$selection","stale":"$missing"});
            changes["inputs"] = inputs.clone();
            let hash = lazypencil::document::hash(inputs.to_string().as_bytes());
            changes["metadata"] = json!({"type":"test","keep":"generated","literal":"$tone","lazypencil":{"script_scope":{"version":1,"script_uri":"override.js","library":"base","inputs":{"hash":hash,"library":""}}}});
        }
        Mode::UnknownContext => {
            changes["metadata"] = json!({"type":"test","lazypencil":{"script_scope":{"version":1,"script_uri":"override.js","library":"uncaptured"}}});
        }
        Mode::ExplicitBindings
        | Mode::BindingLibrary
        | Mode::BindingMapped
        | Mode::BindingCatalog
        | Mode::BindingInputs
        | Mode::BindingStaleInputs
        | Mode::BindingInheritedInputs
        | Mode::BindingMetadata
        | Mode::BindingUnknownTarget
        | Mode::BindingUnknownLibrary
        | Mode::BindingUnmanifestedAsset
        | Mode::BindingCombined
        | Mode::NativeParent { .. } => {
            let mut bindings = json!({"version":1,"script_uri":"override.js","components":{"badge":"badge"},"variables":{"tone":"tone"}});
            if [
                Mode::BindingLibrary,
                Mode::BindingInputs,
                Mode::BindingStaleInputs,
            ]
            .contains(&mode)
            {
                bindings["library"] = json!("base");
            }
            if mode == Mode::BindingMapped {
                bindings["components"]["badge"] = json!("alternate");
                bindings["variables"]["tone"] = json!("accent");
            }
            if [Mode::BindingInputs, Mode::BindingStaleInputs].contains(&mode) {
                let inputs = json!({"selected":"badge","stale":"$missing"});
                changes["inputs"] = inputs.clone();
                bindings["inputs_hash"] = json!(if mode == Mode::BindingInputs {
                    lazypencil::document::hash(inputs.to_string().as_bytes())
                } else {
                    "a".repeat(64)
                });
                bindings["input_components"] = json!({"badge":"badge"});
            }
            if mode == Mode::BindingInheritedInputs {
                let inputs = json!({"selected":"$selection","stale":"$missing"});
                bindings["inputs_hash"] =
                    json!(lazypencil::document::hash(inputs.to_string().as_bytes()));
                bindings["input_components"] = json!({"badge":"alternate"});
            }
            if mode == Mode::BindingMetadata {
                changes.as_object_mut().unwrap().remove("scriptUri");
                bindings["script_uri"] = json!("base.js");
                fs::write(temp.path().join("library/base.js"), REPLACEMENT).unwrap();
            }
            if mode == Mode::BindingCatalog {
                bindings["version"] = json!(2);
                bindings["components"] = json!({"component-slot":"badge"});
                bindings["variables"] = json!({"variable-slot":"tone"});
                bindings["assets"] = json!({});
                bindings["script_scopes"] = json!({"":{"components":{"badge":"component-slot"},"variables":{"tone":"variable-slot"},"themes":{},"assets":{},"libraries":{"":""},"browser_libraries":{}}});
            }
            if mode == Mode::BindingUnknownTarget {
                bindings["components"]["badge"] = json!("missing");
            }
            if mode == Mode::BindingUnknownLibrary {
                bindings["library"] = json!("uncaptured");
            }
            if mode == Mode::BindingUnmanifestedAsset {
                bindings["assets"] = json!({"script:alias.js":"unmanifested.js"});
            }
            changes["metadata"] = json!({"type":"test","keep":"generated","literal":"$tone","lazypencil":{"script_bindings":bindings}});
            if mode == Mode::BindingCombined {
                changes["metadata"]["lazypencil"]["script_scope"] =
                    json!({"version":1,"script_uri":"override.js","library":"base"});
            }
            if matches!(
                mode,
                Mode::NativeParent {
                    scope: NativeScope::Scope,
                    ..
                }
            ) {
                changes["metadata"]["lazypencil"] =
                    json!({"script_scope":{"version":1,"script_uri":"override.js","library":""}});
            } else if matches!(
                mode,
                Mode::NativeParent {
                    scope: NativeScope::Default,
                    ..
                }
            ) {
                changes["metadata"]
                    .as_object_mut()
                    .unwrap()
                    .remove("lazypencil");
            }
        }
        Mode::StaleScope | Mode::StaleInputScope => {
            let mut scope = json!({"version":1,"script_uri":"old.js","library":"uncaptured"});
            if mode == Mode::StaleInputScope {
                changes["inputs"] = json!({"selected":"badge"});
                scope["inputs"] = json!({"hash":"a".repeat(64),"library":"uncaptured"});
            }
            changes["metadata"] = json!({"type":"test","keep":"generated","literal":"$tone","lazypencil":{"script_scope":scope}});
        }
        Mode::ActiveInputScope | Mode::InheritedInputScope => {
            let inputs = if mode == Mode::ActiveInputScope {
                let inputs = json!({"selected":"badge"});
                changes["inputs"] = inputs.clone();
                inputs
            } else {
                json!({"selected":"$selection","stale":"$missing"})
            };
            let hash = lazypencil::document::hash(inputs.to_string().as_bytes());
            changes["metadata"] = json!({"type":"test","keep":"generated","literal":"$tone","lazypencil":{"script_scope":{"version":1,"script_uri":"old.js","library":"base","inputs":{"hash":hash,"library":""}}}});
        }
        Mode::StaleInheritedInputScope | Mode::InheritedUnknownInputScope => {
            if mode == Mode::StaleInheritedInputScope {
                fs::write(temp.path().join("library/base.js"), "/**\n * @schema 2.21\n * @input selected: ref = \"badge\"\n */\nreturn [{type:'rectangle',width:36,height:12,fill:'#0000ff'}];").unwrap();
            }
            let selected = if mode == Mode::StaleInheritedInputScope {
                "$base_selection"
            } else {
                "$selection"
            };
            let inputs = json!({"selected":selected,"stale":"$missing"});
            let hash = lazypencil::document::hash(inputs.to_string().as_bytes());
            changes["metadata"] = json!({"type":"test","keep":"generated","literal":"$tone","lazypencil":{"script_scope":{"version":1,"script_uri":"old.js","library":"base","inputs":{"hash":hash,"library":"uncaptured"}}}});
        }
        Mode::MetadataOutput
        | Mode::MetadataInput
        | Mode::MetadataBoth
        | Mode::MetadataUnknownSource => {
            changes.as_object_mut().unwrap().remove("scriptUri");
            fs::write(temp.path().join("library/base.js"), REPLACEMENT).unwrap();
            let mut scope = json!({"version":1,"script_uri":"base.js","library":""});
            if mode == Mode::MetadataInput {
                scope["script_uri"] = json!("old.js");
                scope["library"] = json!("uncaptured");
            }
            if [Mode::MetadataInput, Mode::MetadataBoth].contains(&mode) {
                let inputs = json!({"selected":"$selection","stale":"$missing"});
                scope["inputs"] = json!({"hash":lazypencil::document::hash(inputs.to_string().as_bytes()),"library":""});
            }
            if mode == Mode::MetadataUnknownSource {
                scope["library"] = json!("uncaptured");
            }
            changes["metadata"] = json!({"type":"test","keep":"generated","literal":"$tone","lazypencil":{"script_scope":scope}});
        }
        Mode::Inherit | Mode::SavedInputOwner => {}
    }
    let direct = |mut changes: Value| {
        changes["type"] = json!("ref");
        changes["ref"] = json!("base:worker");
        changes
    };
    let template = match style {
        Style::EmittedChildren | Style::EmittedReplacement => {
            let script = json!({"id":"emitted","type":"script","scriptUri":"override.js","width":36,"height":12,
                "inputs":{"selected":"badge"},"metadata":{"type":"test","keep":"generated","literal":"$tone",
                "lazypencil":{"script_scope":{"version":1,"script_uri":"override.js","library":"base"}}}});
            if style == Style::EmittedChildren {
                json!({"type":"ref","ref":"base:card","children":[script],"descendants":{"emitted":changes}})
            } else {
                json!({"type":"ref","ref":"base:card","descendants":{
                    "draw":{"id":"replacement","type":"frame","layout":"none","width":36,"height":12,"children":[script]},
                    "emitted":changes
                }})
            }
        }
        Style::Direct => direct(changes),
        Style::Descendant => json!({"type":"ref","ref":"base:card","descendants":{"draw":changes}}),
        Style::Replacement => {
            json!({"type":"ref","ref":"base:card","descendants":{"draw":direct(changes)}})
        }
        Style::Children => json!({"type":"ref","ref":"base:card","children":[direct(changes)]}),
        Style::Script => {
            changes["type"] = json!("script");
            changes["width"] = json!(36);
            changes["height"] = json!(12);
            changes
        }
    };
    let code = format!(
        "/* @schema 2.21 */\nconst template={template};return Array.from({{length:{count}}},(_,i)=>({{...template,id:'child-'+i}}));\n"
    );
    fs::write(temp.path().join("library/draw.js"), &code).unwrap();
    write(
        &temp.path().join("library/base.pen"),
        json!({"version":version,
        "variables":{"selection":{"type":"string","value":"badge"},"tone":{"type":"color","value":"#ff0000"}},
        "children":[
            {"id":"badge","name":"Base badge","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#ff0000"},
            {"id":"worker","name":"Base worker","type":"script","reusable":true,"width":36,"height":12,"scriptUri":"base.js","inputs":{"selected":"$selection","stale":"$missing"},"metadata":{"type":"test","keep":"base"}},
            {"id":"card","type":"frame","reusable":true,"layout":"none","width":36,"height":12,"children":[{"id":"draw","type":"ref","ref":"worker"}]}
        ]}),
    );
    write(
        &temp.path().join("library/ui.pen"),
        json!({"version":version,"imports":{"base":"base.pen"},
        "variables":{"selection":{"type":"string","value":"badge"},"tone":{"type":"color","value":"#00ff00"}},
        "children":[
            {"id":"badge","name":"UI badge","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#00ff00"},
            {"id":"generator","name":"Generator","type":"script","reusable":true,"scriptUri":"draw.js","width":36,"height":12}
        ]}),
    );
    write(
        &temp.path().join("main.pen"),
        json!({"version":version,"imports":{"ui":"library/ui.pen"},
        "variables":{"tone":{"type":"color","value":"#0000ff"},"selection":{"type":"string","value":"badge"},"missing":{"type":"string","value":"badge"}},
        "children":[
            {"id":"definitions","type":"frame","enabled":false,"children":[{"id":"badge","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#0000ff"}]},
            {"id":"use","type":"ref","ref":"ui:generator"}
        ]}),
    );
    if matches!(mode, Mode::CodeOwner { .. }) {
        fs::write(temp.path().join("library/base.js"), REPLACEMENT).unwrap();
        write(
            &temp.path().join("library/palette.pen"),
            json!({"version":version,
                "variables":{"tone":{"type":"color","value":"#00ffff"},"selection":{"type":"string","value":"badge"}},
                "children":[{"id":"badge","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#00ffff"}]
            }),
        );
        let path = temp.path().join("library/base.pen");
        let mut base: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        base["imports"]["palette"] = json!("palette.pen");
        base["children"][1]["metadata"]["lazypencil"] = if matches!(
            mode,
            Mode::CodeOwner { native: true, .. }
        ) {
            json!({"script_bindings":{"version":1,"script_uri":"base.js","library":"palette","components":{"badge":"badge"},"variables":{"tone":"tone"}}})
        } else {
            json!({"script_scope":{"version":1,"script_uri":"base.js","library":"palette"}})
        };
        write(&path, base);
    }
    if [Mode::BindingMapped, Mode::BindingInheritedInputs].contains(&mode)
        || matches!(
            mode,
            Mode::Combined {
                native_parent: false,
                ..
            }
        )
        || matches!(mode, Mode::InputContext { .. })
        || matches!(mode, Mode::InheritedState(_))
    {
        let path = temp.path().join("library/ui.pen");
        let mut ui: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        ui["variables"]["accent"] = json!({"type":"color","value":"#00ffff"});
        if matches!(
            mode,
            Mode::InputContext {
                variant: Context::Variable,
                carrier: false
            }
        ) {
            ui["variables"]["choice"] = json!({"type":"string","value":"badge"});
        }
        ui["children"].as_array_mut().unwrap().push(json!({"id":"alternate","name":"UI alternate","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#00ffff"}));
        write(&path, ui);
    }
    if mode == Mode::ThemedVariable {
        let path = temp.path().join("library/base.pen");
        let mut base: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        base["themes"] = json!({"mode":["light","dark"]});
        base["children"][2]["theme"] = json!({"mode":"dark"});
        write(&path, base);
        let path = temp.path().join("library/ui.pen");
        let mut ui: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        ui["themes"] = json!({"mode":["light","dark"]});
        ui["variables"]["selection"]["value"] = json!([
            {"value":"badge"},{"theme":{"mode":"dark"},"value":"alternate"}
        ]);
        ui["children"].as_array_mut().unwrap().push(json!({"id":"alternate","name":"UI alternate","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#00ffff"}));
        write(&path, ui);
        let path = temp.path().join("main.pen");
        let mut main: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        main["themes"] = json!({"copy_mode":["light","dark"]});
        write(&path, main);
    }
    if mode == Mode::SavedInputOwner {
        write(
            &temp.path().join("library/inputs.pen"),
            json!({"version":version,
                "variables":{"source_selection":{"type":"string","value":"badge"}},
                "children":[{"id":"badge","name":"Input badge","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#00ffff"}]
            }),
        );
        let path = temp.path().join("library/base.pen");
        let mut base: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        base["imports"] = json!({"inputs":"inputs.pen"});
        let inputs = json!({"selected":"$source_selection","stale":"$missing"});
        base["children"][1]["inputs"] = inputs.clone();
        base["children"][1]["metadata"]["lazypencil"]["script_scope"] = json!({"version":1,"script_uri":"base.js","library":"","inputs":{"hash":lazypencil::document::hash(inputs.to_string().as_bytes()),"library":"inputs"}});
        write(&path, base);
    }
    if matches!(
        mode,
        Mode::Combined {
            variant: Combined::InheritedNonlexical,
            native_parent: false
        }
    ) || matches!(mode, Mode::InheritedState(_))
    {
        write(
            &temp.path().join("library/inputs.pen"),
            json!({"version":version,
                "variables":{"selection":{"type":"string","value":"badge"}},
                "children":[{"id":"badge","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#00ffff"}]
            }),
        );
        let path = temp.path().join("library/base.pen");
        let mut base: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        base["imports"] = json!({"inputs":"inputs.pen"});
        base["children"][1]["metadata"]["lazypencil"]["script_input_context"] = json!({
            "version":1,"library":"inputs","values":{"selected":"$selection","stale":"$missing"},
            "variables":{"selection":"selection"},"components":{"badge":"badge"}
        });
        if mode.underlay() {
            write(
                &temp.path().join("library/underlay.pen"),
                json!({"version":version,
                    "variables":{"tone":{"type":"color","value":"#0000ff"}},
                    "children":[{"id":"badge","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#0000ff"}]
                }),
            );
            base["imports"]["underlay"] = json!("underlay.pen");
            let raw = json!({"selected":"$selection","stale":"$missing"});
            let hash = lazypencil::document::hash(raw.to_string().as_bytes());
            if mode == Mode::InheritedState(Inherited::UnderlayScope) {
                base["children"][1]["metadata"]["lazypencil"]["script_scope"] = json!({"version":1,"script_uri":"base.js","library":"","inputs":{"hash":hash,"library":"underlay"}});
            } else {
                base["children"][1]["metadata"]["lazypencil"]["script_bindings"] = json!({"version":1,"script_uri":"base.js","library":"underlay","variables":{"tone":"tone"},"components":{"badge":"badge"},"inputs_hash":hash,"input_components":{"badge":"badge"}});
            }
        }
        write(&path, base);
    }
    if let Some(library) = mode.native_parent_library() {
        let path = temp.path().join(if library {
            "library/base.pen"
        } else {
            "library/ui.pen"
        });
        let mut owner: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        owner["variables"]["accent"] = json!({"type":"color","value":"#00ffff"});
        if matches!(mode, Mode::NativeInput { .. }) {
            owner["variables"]["native_selection"] = json!({"type":"string","value":"alternate"});
            if !library {
                owner["variables"]["selection"]["value"] = json!("alternate");
            }
            let path = temp.path().join("library/base.pen");
            let mut base: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            base["variables"]["native_selection"] = json!({"type":"string","value":"missing"});
            if !library {
                base["children"].as_array_mut().unwrap().push(json!({"id":"alternate","name":"Base alternate","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#ff0000"}));
            }
            write(&path, base);
        }
        owner["children"].as_array_mut().unwrap().push(json!({"id":"alternate","name":"Native alternate","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#00ffff"}));
        if !library {
            owner["children"].as_array_mut().unwrap().extend([
                json!({"id":"worker-bridge","type":"ref","reusable":true,"ref":"base:worker"}),
                json!({"id":"card-bridge","type":"ref","reusable":true,"ref":"base:card"}),
            ]);
        }
        write(&path, owner);
        let path = temp.path().join("library/ui.pen");
        let mut ui: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let mut parent = json!({"version":1,"script_uri":"draw.js",
            "components":{"badge":"alternate","base:worker":if library {"worker"} else {"worker-bridge"},"base:card":if library {"card"} else {"card-bridge"}},
            "variables":{"tone":"accent"},
            "descendants":{"base:card":{"draw":"draw"}},
            "descendant_contexts":{"base:card":{"draw":{"parent":"","instance":true,"target_instance":true}}}
        });
        if matches!(mode, Mode::NativeInput { .. }) {
            parent["variables"]["selection"] = json!("native_selection");
        }
        if library {
            parent["library"] = json!("base");
            ui["imports"]["base"] = json!("base/base.pen");
        }
        let generator = ui["children"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|node| node["id"] == "generator")
            .unwrap();
        generator["metadata"] =
            json!({"type":"test","keep":"native-parent","lazypencil":{"script_bindings":parent}});
        write(&path, ui);
        if library {
            fs::create_dir(temp.path().join("library/base")).unwrap();
            fs::rename(
                temp.path().join("library/base.pen"),
                temp.path().join("library/base/base.pen"),
            )
            .unwrap();
            fs::rename(
                temp.path().join("library/base.js"),
                temp.path().join("library/base/base.js"),
            )
            .unwrap();
            fs::write(temp.path().join("library/base/override.js"), REPLACEMENT).unwrap();
            fs::write(
                temp.path().join("library/override.js"),
                "/* @schema 2.21 */ return [{type:'rectangle',width:36,height:12,fill:'#ff00ff'}];",
            )
            .unwrap();
        }
    }
    let store = Store::open(temp.path().join("main.pen"), Some(temp.path().join("data"))).unwrap();
    (temp, store, code)
}

fn copy(store: &Store, include_source: bool, case: &Case<'_>) -> Value {
    let mode = case.mode;
    let mut variables = serde_json::Map::new();
    if matches!(mode, Mode::CodeOwner { .. }) {
        for name in ["selection", "tone"] {
            variables.insert(
                format!("ui:base:palette:{name}"),
                json!({"action":"rename","name":format!("palette_{name}")}),
            );
        }
    }
    if matches!(
        mode,
        Mode::Combined {
            variant: Combined::InheritedNonlexical,
            native_parent: false
        }
    ) || matches!(mode, Mode::InheritedState(_))
    {
        variables.insert(
            "ui:base:inputs:selection".into(),
            json!({"action":"rename","name":"input_selection"}),
        );
        if mode.underlay() {
            variables.insert(
                "ui:base:underlay:tone".into(),
                json!({"action":"rename","name":"underlay_tone"}),
            );
        }
    }
    for (scope, prefix) in [("ui", "ui_"), ("ui:base", "base_")] {
        for name in ["selection", "tone"] {
            variables.insert(
                format!("{scope}:{name}"),
                json!({"action":"rename","name":format!("{prefix}{name}")}),
            );
        }
        if matches!(mode, Mode::NativeInput { .. })
            && !(scope == "ui" && mode.native_parent_library() == Some(true))
        {
            variables.insert(
                format!("{scope}:native_selection"),
                json!({"action":"rename","name":format!("{prefix}native_selection")}),
            );
        }
    }
    let mut options = json!({"component_prefix":"Copy ","variables":variables});
    if store.read().unwrap().value()["themes"]
        .get("copy_mode")
        .is_some()
    {
        options["themes"] = json!({
            "ui:mode":{"action":"reuse","target":"copy_mode"},
            "ui:base:mode":{"action":"reuse","target":"copy_mode"}
        });
    }
    if include_source {
        options["script_assets"] = json!({"ui:generator":[{"kind":"script","uri":"override.js"}]});
    }
    fixture::apply(
        store,
        "copy",
        json!([
            {"op":"import_component","source":{"library_alias":"ui","node_id":"generator"},"mode":"copy","options":options},
            {"op":"delete","target":"use"},{"op":"remove_import","alias":"ui"}
        ]),
    )
}

fn preview(store: &Store, first: Color, metadata: &str) -> Vec<u8> {
    preview_colours(store, [first, Color::GREEN, Color::GREEN], metadata)
}

fn preview_colours(store: &Store, colours: [Color; 3], metadata: &str) -> Vec<u8> {
    let bytes = fs::read(store.path()).unwrap();
    let (project, layout) = service::layout_snapshot_with_runtime(
        store,
        &[],
        None,
        None,
        &mut ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
    )
    .unwrap();
    assert_eq!(layout.script_executions.len(), 2);
    assert!(
        layout.summary()["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let child = layout
        .items
        .iter()
        .find(|item| {
            item.node.kind() == "script"
                && item.node.generated_by.is_some()
                && item
                    .node
                    .properties
                    .get("metadata")
                    .is_some_and(|m| m["keep"] == metadata)
        })
        .expect("Generated script metadata");
    assert_eq!(child.node.properties["metadata"]["keep"], metadata);
    if metadata == "generated" {
        assert_eq!(child.node.properties["metadata"]["literal"], "$tone");
        if let Some(opaque) =
            child.node.properties["metadata"].pointer("/lazypencil/script_override")
        {
            assert_eq!(opaque, &json!({"version":999,"opaque":"keep"}));
        }
    }
    let image = render::png(
        &project,
        &layout,
        &RenderOptions {
            region: Some(Rect {
                x: 0.,
                y: 0.,
                width: 36.,
                height: 12.,
            }),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!((image.width, image.height), (36, 12));
    for (x, color) in [5, 17, 29].into_iter().zip(colours) {
        assert_eq!(pixel(&image.png, x), color, "x={x}");
    }
    assert_eq!(fs::read(store.path()).unwrap(), bytes);
    image.png
}

fn separate_code_owner(store: &Store) {
    let document = store.read().unwrap();
    assert!(
        document.locations().keys().any(|id| {
            let node = document.node(id).unwrap();
            let bindings = &node["metadata"]["lazypencil"]["script_bindings"];
            let Some(code) = bindings["code_namespace"].as_str() else {
                return false;
            };
            let output = bindings["script_namespace"].as_str().unwrap_or("");
            if code == output {
                return false;
            }
            let code_variable = bindings["script_scopes"][code]["variables"]["tone"]
                .as_str()
                .unwrap();
            let output_variable = bindings["script_scopes"][output]["variables"]["tone"]
                .as_str()
                .unwrap();
            bindings["variables"][code_variable] != bindings["variables"][output_variable]
        }),
        "The copied code must retain a different owner from its explicit output scope"
    );
}

fn independent_case(
    version: &str,
    style: &str,
    mode: &str,
    first: Color,
) -> (TempDir, Store, Vec<u8>) {
    eprintln!("Generated source: schema={version}, style={style}, mode={mode}");
    let case = Case::parse(style, mode);
    let mode = case.mode;
    let (temp, store, code) = setup(version, &case, 1);
    let metadata = if [
        Mode::Metadata,
        Mode::StaleScope,
        Mode::StaleInputScope,
        Mode::ExplicitContext,
        Mode::ExplicitInputContext,
        Mode::ActiveInputScope,
        Mode::InheritedInputScope,
        Mode::StaleInheritedInputScope,
        Mode::MetadataOutput,
        Mode::MetadataInput,
        Mode::MetadataBoth,
    ]
    .contains(&mode)
        || mode == Mode::ExplicitBindings
        || mode.is_binding()
    {
        "generated"
    } else {
        "base"
    };
    let output = if mode == Mode::EmittedCode(Owner::Keep) {
        Color::RED
    } else if mode == Mode::InheritedState(Inherited::UnderlayBinding) {
        Color::BLUE
    } else if matches!(mode, Mode::CodeOwner { .. }) {
        if matches!(
            mode,
            Mode::CodeOwner {
                variant: Owner::Keep,
                ..
            }
        ) {
            Color::CYAN
        } else {
            Color::RED
        }
    } else if mode.native_parent_library() == Some(true)
        || [
            Mode::ExplicitContext,
            Mode::ExplicitInputContext,
            Mode::MetadataInput,
            Mode::BindingLibrary,
            Mode::BindingInputs,
            Mode::BindingStaleInputs,
            Mode::Combined {
                variant: Combined::StaleBindingOutput,
                native_parent: false,
            },
            Mode::InputContext {
                variant: Context::Scope,
                carrier: false,
            },
            Mode::InputContext {
                variant: Context::Output,
                carrier: true,
            },
            Mode::InputContext {
                variant: Context::Both,
                carrier: true,
            },
            Mode::InputContext {
                variant: Context::MetadataOutput,
                carrier: true,
            },
        ]
        .contains(&mode)
    {
        Color::RED
    } else if mode == Mode::BindingMapped {
        Color::CYAN
    } else {
        Color::GREEN
    };
    let render = |store: &Store, stage: &str| {
        let png = preview_colours(store, [first, output, output], metadata);
        if ([
            Mode::ExplicitContext,
            Mode::ExplicitInputContext,
            Mode::ActiveInputScope,
            Mode::InheritedInputScope,
            Mode::StaleInheritedInputScope,
            Mode::MetadataOutput,
            Mode::MetadataInput,
            Mode::MetadataBoth,
            Mode::SavedInputOwner,
        ]
        .contains(&mode)
            || mode == Mode::ExplicitBindings
            || mode.is_binding())
            && let Some(root) = std::env::var_os("LAZYPENCIL_TEST_TUI_ARTIFACTS")
        {
            let root = std::path::PathBuf::from(root).join("generated-script-scopes");
            fs::create_dir_all(&root).unwrap();
            let name = format!("{version}-{}-{}-{stage}", case.style_label, case.label);
            fs::write(root.join(format!("{name}.png")), &png).unwrap();
            fs::write(root.join(format!("{name}.json")), serde_json::to_vec_pretty(&json!({
                "source_commit":std::env::var("GITHUB_SHA").ok(),"png_sha256":lazypencil::document::hash(&png),
                "width":36,"height":12,"red_first":first==Color::RED,"red_output":output==Color::RED,
                "first_rgba":[first.r(),first.g(),first.b(),first.a()],
                "output_rgba":[output.r(),output.g(),output.b(),output.a()]
            })).unwrap()).unwrap();
        }
        png
    };
    let before = render(&store, "original");
    let response = copy(&store, true, &case);
    let component = response["generated_ids"]["0"]["component_id"]
        .as_str()
        .unwrap();
    let document = store.read().unwrap();
    if matches!(mode, Mode::CodeOwner { .. }) {
        separate_code_owner(&store);
    }
    let node = document.node(component).unwrap();
    assert_eq!(
        fs::read(temp.path().join(node["scriptUri"].as_str().unwrap())).unwrap(),
        code.as_bytes()
    );
    let replacement =
        node["metadata"]["lazypencil"]["script_bindings"]["assets"]["script:override.js"]
            .as_str()
            .unwrap();
    assert_eq!(
        fs::read(temp.path().join(replacement)).unwrap(),
        REPLACEMENT.as_bytes()
    );
    fs::remove_dir_all(temp.path().join("library")).unwrap();
    assert_eq!(
        render(&store, "copied"),
        before,
        "{version}/{style}/{}/copy",
        case.label
    );
    let (second, _) = fixture::recopy(&store, component, &temp.path().join("second"), "again_");
    if matches!(mode, Mode::CodeOwner { .. }) {
        separate_code_owner(&second);
    }
    fs::remove_file(store.path()).unwrap();
    fs::remove_dir_all(temp.path().join("assets")).unwrap();
    assert_eq!(
        render(&second, "recopied"),
        before,
        "{version}/{style}/{}/recopy",
        case.label
    );

    (temp, second, before)
}

#[test]
fn generated_replacements_keep_their_code_owner_under_later_overrides() {
    std::thread::scope(|threads| {
        for version in ["2.19", "2.21"] {
            threads.spawn(move || {
                for style in ["emitted-children", "emitted-replacement"] {
                    for variant in ["keep", "scope", "binding", "carrier"] {
                        let mode = format!("binding-emitted-code-{variant}");
                        let _fixture = independent_case(version, style, &mode, Color::GREEN);
                    }
                }
            });
        }
    });
}

#[test]
fn inherited_input_generations_restore_lexical_controls_and_underlying_scopes() {
    std::thread::scope(|threads| {
        for version in ["2.19", "2.21"] {
            threads.spawn(move || {
                for style in ["direct", "descendant", "replacement", "children"] {
                    for (variant, first) in [
                        ("scope", Color::GREEN),
                        ("stale-scope", Color::RED),
                        ("context", Color::CYAN),
                        ("stale-context", Color::RED),
                        ("carrier", Color::CYAN),
                        ("stale-carrier", Color::RED),
                        ("carrier-output", Color::CYAN),
                        ("underlay-scope", Color::BLUE),
                        ("underlay-binding", Color::BLUE),
                    ] {
                        let mode = format!("binding-inherited-state-{variant}");
                        let (_temp, store, _) = independent_case(version, style, &mode, first);
                        let document = store.read().unwrap();
                        assert!(document.locations().keys().any(|id| {
                            document
                                .node(id)
                                .unwrap()
                                .pointer("/metadata/lazypencil/script_input_context/generations")
                                .and_then(Value::as_object)
                                .is_some_and(|generations| generations.len() >= 2)
                        }));
                    }
                    let _fixture = independent_case(
                        version,
                        style,
                        "binding-combined-inherited-nonlexical",
                        Color::CYAN,
                    );
                }
                different_input_generations(version);
            });
        }
    });
}

fn generation_panels(store: &Store, version: &str, stage: &str) -> Vec<Vec<u8>> {
    let (project, layout) = service::layout_snapshot_with_runtime(
        store,
        &[],
        None,
        None,
        &mut ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
    )
    .unwrap();
    assert_eq!(layout.script_executions.len(), 4);
    assert!(
        layout.summary()["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    [("old", 0.0, Color::RED, Color::GREEN), ("new", 36.0, Color::BLUE, Color::BLUE)]
        .into_iter().map(|(name, x, first, output)| {
            let image = render::png(&project, &layout, &RenderOptions {
                region: Some(Rect{x,y:0.,width:36.,height:12.}), ..Default::default()
            }).unwrap();
            for (x, expected) in [(5, first), (17, output), (29, output)] {
                assert_eq!(pixel(&image.png, x), expected, "{version}/{stage}/{name}/{x}");
            }
            if let Some(root) = std::env::var_os("LAZYPENCIL_TEST_TUI_ARTIFACTS") {
                let root = std::path::PathBuf::from(root).join("generated-script-scopes");
                fs::create_dir_all(&root).unwrap();
                let name = format!("{version}-generation-{name}-{stage}");
                fs::write(root.join(format!("{name}.png")), &image.png).unwrap();
                fs::write(root.join(format!("{name}.json")), serde_json::to_vec_pretty(&json!({
                    "source_commit":std::env::var("GITHUB_SHA").ok(),"png_sha256":lazypencil::document::hash(&image.png),
                    "width":36,"height":12,"red_first":first==Color::RED,"red_output":output==Color::RED,
                    "first_rgba":[first.r(),first.g(),first.b(),first.a()],"output_rgba":[output.r(),output.g(),output.b(),output.a()]
                })).unwrap()).unwrap();
            }
            image.png
        }).collect()
}

fn different_input_generations(version: &str) {
    let case = Case::parse("direct", "binding-inherited-state-stale-carrier");
    let (temp, store, _) = setup(version, &case, 1);
    let response = copy(&store, true, &case);
    let old = response["generated_ids"]["0"]["component_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let document = store.read().unwrap();
    let bindings = &document.node(&old).unwrap()["metadata"]["lazypencil"]["script_bindings"];
    let worker_key = bindings["script_scopes"][""]["components"]["base:worker"]
        .as_str()
        .unwrap();
    let worker = bindings["components"][worker_key]
        .as_str()
        .unwrap()
        .to_owned();
    let replacement = bindings["assets"]["script:override.js"]
        .as_str()
        .unwrap()
        .to_owned();
    let template = json!({"type":"ref","ref":worker,"scriptUri":replacement,
        "metadata":{"type":"lazypencil","lazypencil":{"script_override":{
            "version":1,"metadata":{"type":"test","keep":"generated","literal":"$tone"},
            "input":{"version":1,"library":"uncaptured","values":{"selected":"different"},"variables":{},"components":{}}
        }}}
    });
    fs::write(
        temp.path().join("new-generation.js"),
        format!("/* @schema 2.21 */ return [{template}];"),
    )
    .unwrap();
    let inserted = fixture::apply(
        &store,
        "two-generations",
        json!([
            {"op":"update","target":old,"set":{"enabled":false}},
            {"op":"insert","parent":null,"client_ref":"fresh","node":{"id":"new-generator","name":"New generator","type":"script","reusable":true,"enabled":false,"scriptUri":"new-generation.js","width":36,"height":12}},
            {"op":"insert","parent":null,"node":{"id":"generations","type":"frame","reusable":true,"layout":"none","width":72,"height":12,"children":[
                {"id":"old-use","type":"ref","ref":old,"enabled":true},
                {"id":"new-use","type":"ref","ref":{"result_of":"fresh"},"enabled":true,"x":36}
            ]}}
        ]),
    );
    let fresh = inserted["generated_ids"]["1"]["id"].as_str().unwrap();
    let source = inserted["generated_ids"]["2"]["id"].as_str().unwrap();
    let original = generation_panels(&store, version, "original");
    let second = temp.path().join("second-generations");
    fs::create_dir(&second).unwrap();
    write(
        &second.join("main.pen"),
        json!({"version":version,"imports":{"source":store.path().to_str().unwrap()},
        "variables":{"tone":{"type":"color","value":"#ff00ff"}},"children":[
            {"id":"local","type":"frame","enabled":false,"children":[{"id":"badge","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#ff00ff"}]}
        ]}),
    );
    let second_store = Store::open(second.join("main.pen"), Some(second.join("data"))).unwrap();
    let imported = fixture::apply(
        &second_store,
        "copy-generations",
        json!([
            {"op":"import_component","source":{"library_alias":"source","node_id":source},"mode":"copy","options":{
                "component_prefix":"Generation ","variable_prefix":"again_",
                "script_assets":{(format!("source:{fresh}")):[{"kind":"script","uri":replacement}]}
            }},{"op":"remove_import","alias":"source"}
        ]),
    );
    let group = imported["generated_ids"]["0"]["component_id"]
        .as_str()
        .unwrap();
    fs::remove_file(store.path()).unwrap();
    fs::remove_file(temp.path().join("new-generation.js")).unwrap();
    fs::remove_dir_all(temp.path().join("library")).unwrap();
    fs::remove_dir_all(temp.path().join("assets")).unwrap();
    assert_eq!(
        generation_panels(&second_store, version, "copied"),
        original
    );
    let (third, _) = fixture::recopy(
        &second_store,
        group,
        &temp.path().join("third-generations"),
        "third_",
    );
    fs::remove_file(second_store.path()).unwrap();
    fs::remove_dir_all(second.join("assets")).unwrap();
    assert_eq!(generation_panels(&third, version, "recopied"), original);
}

#[test]
fn inherited_code_ownership_survives_inactive_output_contexts_and_recopy() {
    std::thread::scope(|threads| {
        for version in ["2.19", "2.21"] {
            threads.spawn(move || {
                for style in ["direct", "descendant", "replacement", "children"] {
                    for native in [false, true] {
                        for variant in ["keep", "scope", "binding", "carrier"] {
                            let mode = format!(
                                "binding-code-owner-{variant}{}",
                                if native { "-native" } else { "" }
                            );
                            let _fixture = independent_case(version, style, &mode, Color::GREEN);
                        }
                    }
                }
            });
        }
    });
}

#[test]
fn generated_script_override_carriers_keep_metadata_output_and_input_independent() {
    std::thread::scope(|threads| {
        for version in ["2.19", "2.21"] {
            threads.spawn(move || {
                for style in ["direct", "descendant", "replacement", "children"] {
                    for (variant, first) in [
                        ("output", Color::CYAN),
                        ("input", Color::RED),
                        ("both", Color::RED),
                        ("stale-output", Color::CYAN),
                        ("stale-input", Color::GREEN),
                        ("nested", Color::CYAN),
                        ("metadata-output", Color::CYAN),
                    ] {
                        let mode = format!("binding-carrier-{variant}");
                        let _fixture = independent_case(version, style, &mode, first);
                    }
                }
            });
        }
    });
}

#[test]
fn generated_explicit_input_contexts_compose_with_scopes_and_bindings() {
    std::thread::scope(|threads| {
        for version in ["2.19", "2.21"] {
            threads.spawn(move || {
                for style in ["script", "direct", "descendant", "replacement", "children"] {
                    for (variant, first) in [
                        ("combined", Color::CYAN),
                        ("library", Color::RED),
                        ("variable", Color::CYAN),
                        ("stale", Color::GREEN),
                        ("scope", Color::CYAN),
                        ("only", Color::CYAN),
                    ] {
                        let mode = format!("binding-input-context-{variant}");
                        let _fixture = independent_case(version, style, &mode, first);
                    }
                }
            });
        }
    });
}

#[test]
fn generated_script_code_overrides_keep_saved_input_ownership_after_copy_and_recopy() {
    // Each schema owns independent temporary projects, assets and JS processes.
    // Keep both complete matrices while bounding their concurrency to two.
    std::thread::scope(|threads| {
        for version in ["2.19", "2.21"] {
            threads.spawn(move || {
                for style in ["direct", "descendant", "replacement"] {
                    for mode in ["inherit", "literal", "variable", "null", "metadata"] {
                        let first = if ["inherit", "metadata"].contains(&mode) {
                            Color::RED
                        } else {
                            Color::GREEN
                        };
                        let _fixture = independent_case(version, style, mode, first);
                    }
                }
            });
        }
    });
}

#[test]
fn generated_ref_inputs_follow_inherited_container_themes_after_copy_recopy_and_live_changes() {
    std::thread::scope(|threads| {
        for version in ["2.19", "2.21"] {
            threads.spawn(move || {
                for style in ["descendant", "replacement", "children"] {
                    let (_temp, store, original) = independent_case(version, style, "themed-variable", Color::CYAN);
                    let document = store.read().unwrap();
                    let cards = document.locations().keys().filter(|id| {
                        let node = document.node(id).unwrap();
                        node["type"] == "frame" && node["reusable"] == true
                    }).cloned().collect::<Vec<_>>();
                    assert_eq!(cards.len(), 1, "Expected one reusable theme container");
                    fixture::apply(&store, "container-light", json!([{"op":"update","target":cards[0],"set":{"theme":{"copy_mode":"light"}}}]));
                    assert_ne!(preview(&store, Color::GREEN, "base"), original);
                    fixture::apply(&store, "container-dark", json!([{"op":"update","target":cards[0],"set":{"theme":{"copy_mode":"dark"}}}]));
                    assert_eq!(preview(&store, Color::CYAN, "base"), original);
                }
            });
        }
    });
}

#[test]
fn inactive_generated_script_scopes_do_not_block_copy_or_recopy() {
    std::thread::scope(|threads| {
        for version in ["2.19", "2.21"] {
            threads.spawn(move || {
                for style in ["direct", "descendant", "replacement", "script"] {
                    for mode in ["stale-scope", "stale-input-scope"] {
                        let first = if style == "script" || mode == "stale-input-scope" {
                            Color::GREEN
                        } else {
                            Color::RED
                        };
                        let _fixture = independent_case(version, style, mode, first);
                    }
                }
            });
        }
    });
}

#[test]
fn generated_code_overrides_reject_unmanifested_sources_explicit_contexts_and_binding_growth() {
    for (include_source, mode, count, expected) in [
        (false, "inherit", 1, "ASSET_NOT_FOUND"),
        (true, "unknown-context", 1, "INVALID_DOCUMENT"),
        (true, "binding-unknown-target", 1, "INVALID_DOCUMENT"),
        (true, "binding-unknown-library", 1, "INVALID_DOCUMENT"),
        (true, "binding-unmanifested-asset", 1, "ASSET_NOT_FOUND"),
        (true, "inherited-unknown-input-scope", 1, "INVALID_DOCUMENT"),
        (true, "metadata-unknown-source", 1, "INVALID_DOCUMENT"),
        (true, "bad-variable", 1, "INVALID_DOCUMENT"),
        (true, "inherit", 600, "INVALID_DOCUMENT"),
    ] {
        let case = Case::parse("direct", mode);
        let (temp, store, _) = setup("2.21", &case, count);
        copy(&store, include_source, &case);
        fs::remove_dir_all(temp.path().join("library")).unwrap();
        let bytes = fs::read(store.path()).unwrap();
        let error = service::layout_snapshot_with_runtime(
            &store,
            &[],
            None,
            None,
            &mut ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
        )
        .err()
        .expect("Invalid generated source must fail");
        assert_eq!(
            error.code, expected,
            "{mode}/{include_source}/{count}: {error:?}"
        );
        if count == 600 {
            assert!(
                error.message.contains("generated output budget"),
                "{error:?}"
            );
        }
        assert_eq!(fs::read(store.path()).unwrap(), bytes);
    }
}

#[test]
fn generated_scope_and_binding_guards_compose_without_overwriting_each_other() {
    std::thread::scope(|threads| {
        for version in ["2.19", "2.21"] {
            threads.spawn(move || {
                for style in ["script", "direct", "descendant", "replacement", "children"] {
                    for (mode, first) in [
                        (
                            "binding-combined",
                            if style == "script" {
                                Color::GREEN
                            } else {
                                Color::RED
                            },
                        ),
                        ("binding-combined-scope-input", Color::RED),
                        ("binding-combined-binding-input", Color::CYAN),
                        ("binding-combined-stale-binding-output", Color::RED),
                        ("binding-combined-stale-scope-output", Color::RED),
                        ("binding-combined-stale-inputs", Color::GREEN),
                    ] {
                        let _fixture = independent_case(version, style, mode, first);
                    }
                }
                for style in ["direct", "descendant", "replacement", "children"] {
                    for (mode, first) in [
                        ("binding-combined-inherited-binding-input", Color::CYAN),
                        ("binding-combined-inherited-scope-input", Color::GREEN),
                    ] {
                        let _fixture = independent_case(version, style, mode, first);
                    }
                }
                for style in ["script", "direct"] {
                    for (mode, first) in [
                        ("binding-native-parent-combined-scope-input", Color::RED),
                        ("binding-native-parent-combined-binding-input", Color::CYAN),
                    ] {
                        let _fixture = independent_case(version, style, mode, first);
                    }
                }
            });
        }
    });
}

#[test]
fn generated_custom_bindings_compose_names_libraries_catalogs_and_saved_inputs() {
    std::thread::scope(|threads| {
        for version in ["2.19", "2.21"] {
            threads.spawn(move || {
                for style in ["direct", "descendant", "replacement", "children", "script"] {
                    for mode in [
                        "explicit-bindings",
                        "binding-library",
                        "binding-mapped",
                        "binding-catalog",
                        "binding-inputs",
                        "binding-stale-inputs",
                    ] {
                        let first = match mode {
                            "binding-inputs" => Color::RED,
                            "binding-stale-inputs" => Color::GREEN,
                            "binding-mapped" if style == "script" => Color::CYAN,
                            "binding-library" => Color::RED,
                            _ if style == "script" => Color::GREEN,
                            _ => Color::RED,
                        };
                        let _fixture = independent_case(version, style, mode, first);
                    }
                }
                for style in ["direct", "descendant", "replacement", "children"] {
                    let _fixture =
                        independent_case(version, style, "binding-inherited-inputs", Color::CYAN);
                    let _fixture = independent_case(version, style, "binding-metadata", Color::RED);
                }
            });
        }
    });
}

#[test]
fn native_binding_parents_keep_the_document_owner_of_nested_code_and_custom_contexts() {
    std::thread::scope(|threads| {
        for version in ["2.19", "2.21"] {
            threads.spawn(move || {
                for style in ["script", "direct", "descendant", "replacement", "children"] {
                    for mode in [
                        "binding-native-parent",
                        "binding-native-parent-scope",
                        "binding-native-parent-default",
                    ] {
                        let first = if style == "script" {
                            Color::GREEN
                        } else {
                            Color::RED
                        };
                        let _fixture = independent_case(version, style, mode, first);
                    }
                }
                for mode in [
                    "binding-native-parent-library",
                    "binding-native-parent-scope-library",
                    "binding-native-parent-default-library",
                ] {
                    let _fixture = independent_case(version, "script", mode, Color::RED);
                }
            });
        }
    });
}

#[test]
fn native_parent_input_guards_use_post_mapping_expressions_and_keep_inherited_controls_separate() {
    std::thread::scope(|threads| {
        for version in ["2.19", "2.21"] {
            threads.spawn(move || {
                for style in ["script", "direct", "descendant", "replacement", "children"] {
                    for (mode, first) in [
                        ("binding-native-parent-input-scope", Color::RED),
                        ("binding-native-parent-input-scope-stale", Color::CYAN),
                        ("binding-native-parent-input-only-scope", Color::RED),
                        ("binding-native-parent-input-only-scope-stale", Color::CYAN),
                        ("binding-native-parent-input-custom", Color::GREEN),
                        ("binding-native-parent-input-custom-stale", Color::CYAN),
                        ("binding-native-parent-input-default", Color::CYAN),
                    ] {
                        let _fixture = independent_case(version, style, mode, first);
                    }
                    if style != "script" {
                        for (mode, first) in [
                            ("binding-native-parent-input-inherited-scope", Color::GREEN),
                            (
                                "binding-native-parent-input-inherited-only-scope",
                                Color::GREEN,
                            ),
                            (
                                "binding-native-parent-input-inherited-only-scope-stale",
                                Color::RED,
                            ),
                            (
                                "binding-native-parent-input-inherited-scope-stale",
                                Color::RED,
                            ),
                            ("binding-native-parent-input-inherited-custom", Color::CYAN),
                            (
                                "binding-native-parent-input-inherited-custom-stale",
                                Color::RED,
                            ),
                        ] {
                            let _fixture = independent_case(version, style, mode, first);
                        }
                    }
                }
                for (mode, first) in [
                    ("binding-native-parent-input-custom-library", Color::RED),
                    (
                        "binding-native-parent-input-custom-stale-library",
                        Color::CYAN,
                    ),
                    ("binding-native-parent-input-default-library", Color::CYAN),
                ] {
                    let _fixture = independent_case(version, "script", mode, first);
                }
            });
        }
    });
}

#[test]
fn generated_inherited_scope_guards_keep_original_spelling_after_copy_and_recopy() {
    std::thread::scope(|threads| {
        for version in ["2.19", "2.21"] {
            threads.spawn(move || {
                for style in ["direct", "descendant", "replacement", "children"] {
                    for mode in ["inherited-input-scope", "metadata-output", "metadata-both"] {
                        let first = if mode == "metadata-output" {
                            Color::RED
                        } else {
                            Color::GREEN
                        };
                        let _fixture = independent_case(version, style, mode, first);
                    }
                }
                for (mode, first) in [
                    ("metadata-input", Color::GREEN),
                    ("stale-inherited-input-scope", Color::RED),
                    ("saved-input-owner", Color::CYAN),
                ] {
                    let _fixture = independent_case(version, "direct", mode, first);
                }
            });
        }
    });
}

#[test]
fn generated_explicit_scopes_keep_output_and_input_namespaces_through_copy_and_recopy() {
    std::thread::scope(|threads| {
        for version in ["2.19", "2.21"] {
            threads.spawn(move || {
                for style in ["direct", "descendant", "replacement", "children", "script"] {
                    for mode in [
                        "explicit-context",
                        "explicit-input-context",
                        "active-input-scope",
                    ] {
                        let first = if mode == "explicit-context" {
                            Color::RED
                        } else {
                            Color::GREEN
                        };
                        let _fixture = independent_case(version, style, mode, first);
                    }
                }
            });
        }
    });
}
