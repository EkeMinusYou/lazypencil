use crate::{Error, Result, document::hash};
use serde_json::{Value, json};
mod project;
pub use project::{DesignSettings, ProjectCatalog};
pub mod usage;

const VERSION: &str = "1.0.52";
struct Guide {
    id: &'static str,
    summary: &'static str,
    body: &'static str,
}
// Embedded bodies are compiler inputs. CI must distinguish them from repository
// prose even when both use Markdown; Cargo tracks these include_str! dependencies.
const GUIDES: &[Guide] = &[
    Guide {
        id: "start",
        summary: "Requirements, document inspection, composition, editing, and visual refinement.",
        body: include_str!("../resources/guidance/start.md"),
    },
    Guide {
        id: "operations",
        summary: "A tested batch that creates a token, reusable button, and component instance.",
        body: concat!(
            include_str!("../resources/guidance/operations.md"),
            "\n```json\n",
            include_str!("../resources/examples/button-operations.json"),
            "\n```\n"
        ),
    },
    Guide {
        id: "layout",
        summary: "Flow, spacing, sizing constraints, text growth, and overflow.",
        body: include_str!("../resources/guidance/layout.md"),
    },
    Guide {
        id: "typography",
        summary: "Readable type hierarchy, real copy, multilingual text, and font metrics.",
        body: include_str!("../resources/guidance/typography.md"),
    },
    Guide {
        id: "color",
        summary: "Color roles, emphasis, contrast, state cues, and themes.",
        body: include_str!("../resources/guidance/color.md"),
    },
    Guide {
        id: "components",
        summary: "Reuse, overrides, slots, generated IDs, and atomic editing.",
        body: include_str!("../resources/guidance/components.md"),
    },
    Guide {
        id: "scripts",
        summary: "Typed inputs, bounded synchronous JavaScript, generated nodes, and runtime dependency snapshots.",
        body: include_str!("../resources/guidance/scripts.md"),
    },
    Guide {
        id: "browser",
        summary: "Page acquisition, emulation, refresh, runtime setup, and truthful capture metadata.",
        body: include_str!("../resources/guidance/browser.md"),
    },
    Guide {
        id: "shaders",
        summary: "GLSL fills and effects, typed uniforms, object and background textures, and explicit time.",
        body: include_str!("../resources/guidance/shaders.md"),
    },
    Guide {
        id: "effects",
        summary: "Native mesh paints, blur and shadows, backdrop context, transparency, effect order, and composited crops.",
        body: include_str!("../resources/guidance/effects.md"),
    },
    Guide {
        id: "verification",
        summary: "Structural validation, measured layout, image inspection, and truthful reporting.",
        body: include_str!("../resources/guidance/verification.md"),
    },
    Guide {
        id: "icons",
        summary: "Pinned vector icon names, aliases, licensing, explicit pack installation, and native paints.",
        body: include_str!("../resources/guidance/icons.md"),
    },
    Guide {
        id: "app-screen",
        summary: "Application screens, controls, states, and primary actions.",
        body: include_str!("../resources/guidance/app-screen.md"),
    },
    Guide {
        id: "dashboard",
        summary: "Information hierarchy, useful metrics, aligned rows, and compact workspaces.",
        body: include_str!("../resources/guidance/dashboard.md"),
    },
    Guide {
        id: "content-page",
        summary: "Message, reading sequence, calls to action, and content-led composition.",
        body: include_str!("../resources/guidance/content-page.md"),
    },
];
const STYLES: &str = include_str!("../resources/styles/manifest.json");

pub fn get_guidance(id: Option<&str>) -> Result<Value> {
    let id = id.unwrap_or("start");
    let guide = GUIDES
        .iter()
        .find(|g| g.id == id)
        .ok_or_else(|| Error::new("INVALID_ARGUMENT", format!("Unknown guidance ID {id:?}")))?;
    let mut result = json!({"id":guide.id,"version":VERSION,"language":"en","license":"MIT", "summary":guide.summary,
        "body":guide.body,"schema_versions":["2.21"],"available_guides":GUIDES.iter().map(|g| json!({"id":g.id,"summary":g.summary,"version":VERSION})).collect::<Vec<_>>()});
    if guide.id == "operations" {
        result["example_operations"] =
            serde_json::from_str(include_str!("../resources/examples/button-operations.json"))
                .map_err(|e| {
                    Error::new(
                        "INTERNAL_ERROR",
                        format!("Invalid bundled operation example: {e}"),
                    )
                })?;
    }
    stamp(&mut result);
    Ok(result)
}

pub fn list_styles(use_case: Option<&str>, limit: usize, cursor: Option<&str>) -> Result<Value> {
    if !(1..=100).contains(&limit) {
        return Err(Error::new("INVALID_ARGUMENT", "Style limit must be 1..100"));
    }
    let catalog = catalog()?;
    let identity = hash(
        json!({"catalog":catalog,"use_case":use_case})
            .to_string()
            .as_bytes(),
    );
    let offset = if let Some(cursor) = cursor {
        let (catalog, offset) = cursor
            .rsplit_once(':')
            .ok_or_else(|| Error::new("INVALID_ARGUMENT", "Malformed style cursor"))?;
        if catalog != identity {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "This cursor belongs to a different style catalog or query",
            ));
        }
        offset
            .parse::<usize>()
            .map_err(|_| Error::new("INVALID_ARGUMENT", "Malformed style offset"))?
    } else {
        0
    };
    let styles = catalog["styles"]
        .as_array()
        .ok_or_else(|| Error::new("INTERNAL_ERROR", "Invalid bundled style catalog"))?;
    let styles = styles
        .iter()
        .filter(|style| {
            use_case.is_none_or(|q| {
                style["use_cases"].as_array().is_some_and(|uses| {
                    uses.iter()
                        .any(|v| v.as_str().is_some_and(|s| s.eq_ignore_ascii_case(q)))
                })
            })
        })
        .collect::<Vec<_>>();
    if offset > styles.len() {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Style cursor is out of range",
        ));
    }
    let total = styles.len();
    let styles=styles.into_iter().skip(offset).take(limit).map(|s| json!({"id":s["id"],"name":s["name"],"version":s["version"],
        "description":s["description"],"use_cases":s["use_cases"],"language":"en","required_fonts":["Noto Sans JP"]})).collect::<Vec<_>>();
    let next =
        (offset + styles.len() < total).then(|| format!("{identity}:{}", offset + styles.len()));
    let mut result = json!({"version":VERSION,"language":"en","styles":styles,"total_matches":total,"next_cursor":next});
    stamp(&mut result);
    Ok(result)
}

pub(crate) fn style_document(id: &str) -> Result<crate::document::Document> {
    let source = match id {
        "standard-ui" => include_str!("../examples/welcome.pen"),
        "dense-workspace" => include_str!("../examples/dense-workspace.pen"),
        "content-page" => include_str!("../examples/content-page.pen"),
        _ => return Err(Error::new("INVALID_ARGUMENT", "Unknown bundled style")),
    };
    crate::document::Document::parse(source.to_owned())
}

pub fn get_style(id: &str) -> Result<Value> {
    let catalog = catalog()?;
    let mut style = catalog["styles"]
        .as_array()
        .and_then(|styles| styles.iter().find(|s| s["id"] == id))
        .cloned()
        .ok_or_else(|| Error::new("INVALID_ARGUMENT", format!("Unknown style ID {id:?}")))?;
    let document = style_document(id)?;
    style["example"] = json!({"filename":style["example"],"format":"pen","document":document.value(),"hash":document.revision(),"read_only":true});
    style["example"]["components"] = json!(
        document
            .locations()
            .keys()
            .filter_map(|id| {
                let node = document.node(id).ok()?;
                (node["reusable"] == true).then(|| json!({"node_id":id,"name":node.get("name")}))
            })
            .collect::<Vec<_>>()
    );
    style["language"] = json!("en");
    style["license"] = json!("MIT");
    style["schema_versions"] = json!(["2.21"]);
    style["required_fonts"] =
        json!([{"family":"Noto Sans JP","source":"bundled","license":"OFL-1.1"}]);
    stamp(&mut style);
    Ok(style)
}

fn catalog() -> Result<Value> {
    serde_json::from_str(STYLES)
        .map_err(|e| Error::new("INTERNAL_ERROR", format!("Invalid bundled styles: {e}")))
}
fn stamp(value: &mut Value) {
    value["hash"] = json!(hash(value.to_string().as_bytes()));
}
