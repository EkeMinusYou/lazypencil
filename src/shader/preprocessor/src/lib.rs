//! Isolated GLSL preprocessing for the browser's bounded Wasm memory.
//!
//! The native renderer must never run untrusted macro expansion in its own
//! address space. The browser instantiates this module with only env.memory;
//! stack exhaustion and allocation failure terminate that instance.

use glsl_lang_pp::{
    exts::{ExtensionSpec, Registry, names::ExtNameAtom},
    processor::{
        ProcessorState,
        event::{DirectiveKind, Event},
        nodes::{Define, DefineObject},
    },
};
use serde::Deserialize;
use std::cell::RefCell;

const MAX_INPUT: usize = 8 * 1024 * 1024;
const MAX_SOURCE: usize = 1024 * 1024;
const MAX_OUTPUT: usize = 4 * 1024 * 1024;
const MAX_EVENTS: usize = 262_144;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    source: String,
    defines: Vec<String>,
    coordinate_scale: [f64; 2],
}

fn has_version(mut source: &str) -> bool {
    loop {
        source = source.trim_start();
        if let Some(comment) = source.strip_prefix("//") {
            source = comment.split_once('\n').map_or("", |(_, tail)| tail);
        } else if let Some(comment) = source.strip_prefix("/*") {
            source = comment.split_once("*/").map_or("", |(_, tail)| tail);
        } else {
            return source.strip_prefix('#').is_some_and(|tail| {
                tail.trim_start_matches([' ', '\t', '\r'])
                    .strip_prefix("version")
                    .is_some_and(|tail| tail.starts_with(char::is_whitespace))
            });
        }
    }
}

fn append(output: &mut String, text: &str) -> Result<(), String> {
    if output.len().saturating_add(text.len()) > MAX_OUTPUT {
        return Err("Expanded shader exceeds 4 MiB".into());
    }
    output.push_str(text);
    Ok(())
}

fn preprocess(request: Request) -> Result<String, String> {
    if request.source.len() > MAX_SOURCE || request.defines.len() > 256 {
        return Err("Shader source or predefined macro budget exceeded".into());
    }
    if request
        .coordinate_scale
        .iter()
        .any(|value| !value.is_finite() || *value <= 0.0)
    {
        return Err("Coordinate scale must be positive and finite".into());
    }
    // Registry::default includes desktop extensions which the actual WebGL
    // context may not implement. Only caller-probed browser macros are defined.
    let mut registry = Registry::new();
    for name in &request.defines {
        if name.len() > 128
            || !name.starts_with("GL_")
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err("Invalid predefined GLSL macro".into());
        }
        if name != "GL_ES" && name != "GL_FRAGMENT_PRECISION_HIGH" {
            registry.add(ExtensionSpec::new(ExtNameAtom::from(name.as_str()), vec![]));
        }
    }
    let mut builder = ProcessorState::builder()
        .registry(&registry)
        .core_profile(false)
        .compatibility_profile(false)
        .es_profile(false);
    for name in &request.defines {
        builder = builder.definition(Define::object(
            name.as_str().into(),
            DefineObject::one(),
            true,
        ));
    }
    let source = if has_version(&request.source) {
        request.source
    } else {
        // The upstream preprocessor otherwise defaults to desktop GLSL 1.10.
        // Reset the line counter so __LINE__ keeps the original source position.
        format!("#version 100\n#line 1\n{}", request.source)
    };
    let [x, y] = request.coordinate_scale;
    let coordinate = format!("(vec4(gl_FragCoord.xy*vec2({x:.17e},{y:.17e}),gl_FragCoord.zw))");
    let mut output = String::new();
    for (index, event) in
        glsl_lang_pp::processor::str::process(&source, builder.finish()).enumerate()
    {
        if index >= MAX_EVENTS {
            return Err("Expanded shader token budget exceeded".into());
        }
        match event.map_err(|error| format!("{error:?}"))? {
            Event::Token {
                token,
                masked: false,
            } => {
                append(
                    &mut output,
                    if token.text() == "gl_FragCoord" {
                        &coordinate
                    } else {
                        token.text()
                    },
                )?;
                // Separators preserve token boundaries even when adjacent macro
                // expansions would otherwise form a new identifier or operator.
                append(&mut output, " ")?;
            }
            Event::Directive {
                directive,
                masked: false,
            } => {
                if !directive.errors().is_empty() {
                    return Err(format!("{:?}", directive.errors()));
                }
                match directive.kind() {
                    DirectiveKind::Version(_)
                    | DirectiveKind::Extension(_)
                    | DirectiveKind::Pragma(_) => {
                        append(&mut output, "\n")?;
                        append(&mut output, &directive.to_string())?;
                        append(&mut output, "\n")?;
                    }
                    DirectiveKind::Error(_)
                    | DirectiveKind::Include(_)
                    | DirectiveKind::Invalid(_) => {
                        return Err("Unsupported or invalid shader directive".into());
                    }
                    _ => {}
                }
            }
            Event::Error {
                error,
                masked: false,
            } => return Err(format!("{error:?}")),
            _ => {}
        }
    }
    Ok(output)
}

thread_local! {
    static INPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static OUTPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Return an input region owned by this instance. Zero signals an invalid size.
#[unsafe(no_mangle)]
pub extern "C" fn input_buffer(length: u32) -> u32 {
    let length = length as usize;
    if length == 0 || length > MAX_INPUT {
        return 0;
    }
    INPUT.with_borrow_mut(|input| {
        input.resize(length, 0);
        input.as_mut_ptr() as u32
    })
}

/// Return the output pointer, with its length available through output_length.
#[unsafe(no_mangle)]
pub extern "C" fn process_shader() -> u32 {
    let result = INPUT.with_borrow(|input| {
        serde_json::from_slice::<Request>(input)
            .map_err(|_| "Invalid shader preprocessor request".to_owned())
            .and_then(preprocess)
    });
    let value = match result {
        Ok(source) => serde_json::json!({"source":source}),
        Err(error) => serde_json::json!({"error":error.chars().take(4096).collect::<String>()}),
    };
    OUTPUT.with_borrow_mut(|output| {
        *output = serde_json::to_vec(&value).expect("String JSON serialization");
        output.as_ptr() as u32
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn output_length() -> u32 {
    OUTPUT.with_borrow(|output| output.len() as u32)
}
