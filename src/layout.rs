use crate::{
    Error, Result,
    document::hash,
    geometry::{Affine, Rect},
    project::{NodeAddress, ResolvedNode, ResolvedScene},
    text::{Fonts, TextBlock, TextMetrics, TextSpec},
};
use serde::Serialize;
use serde_json::{Value, json};
use skia_safe::{Color, textlayout::TextAlign};
use std::{collections::BTreeMap, path::PathBuf};

const MAX_DIMENSION: f32 = 1_000_000.0;
const ENGINE: &str = concat!("lazypencil/", env!("CARGO_PKG_VERSION"), "/layout-7");

#[derive(Debug, Clone, Serialize)]
pub struct LayoutDiagnostic {
    pub code: &'static str,
    pub severity: &'static str,
    pub node_id: String,
    pub message: String,
    pub details: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct LayoutRecord {
    pub id: String,
    pub address: NodeAddress,
    pub source_path: PathBuf,
    pub source_id: String,
    pub parent_id: Option<String>,
    pub node_type: String,
    pub enabled: bool,
    pub rect: Rect,
    pub transform: Affine,
    pub visual_bounds: Rect,
    pub sizing: [String; 2],
    pub text: Option<TextMetrics>,
    pub diagnostics: Vec<LayoutDiagnostic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_by: Option<NodeAddress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<Value>,
}

pub struct LayoutItem {
    pub node: ResolvedNode,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub record: LayoutRecord,
}

pub struct LayoutSnapshot {
    pub id: String,
    pub revision: String,
    pub dependency_revision: String,
    pub dependencies: Vec<crate::project::Dependency>,
    pub items: Vec<LayoutItem>,
    pub roots: Vec<usize>,
    pub text_blocks: BTreeMap<usize, TextBlock>,
    pub bounds: Rect,
    pub runtime_dependencies: Vec<crate::project::Dependency>,
    pub script_executions: Vec<Value>,
}

impl LayoutSnapshot {
    pub fn metadata(&self) -> Value {
        json!({"snapshot_id":self.id,"revision":self.revision,
            "dependency_revision":self.dependency_revision,"dependencies":self.dependencies,
            "engine":ENGINE,"bounds":self.bounds,"node_count":self.items.len(),
            "runtime_dependencies":self.runtime_dependencies,"runtime_dependency_revision":hash(serde_json::to_string(&self.runtime_dependencies).expect("dependencies").as_bytes()),
            "script_executions":self.script_executions})
    }

    pub fn summary(&self) -> Value {
        let mut result = self.metadata();
        result["diagnostics"] = json!(
            self.items
                .iter()
                .flat_map(|i| &i.record.diagnostics)
                .collect::<Vec<_>>()
        );
        result
    }

    pub fn records(&self) -> Vec<&LayoutRecord> {
        self.items.iter().map(|i| &i.record).collect()
    }
}

#[derive(Clone, Copy, Debug)]
enum Sizing {
    Fixed(f32),
    Fit(Option<f32>),
    Fill(Option<f32>),
}

impl Sizing {
    fn name(self) -> &'static str {
        match self {
            Self::Fixed(_) => "fixed",
            Self::Fit(_) => "content",
            Self::Fill(_) => "parent",
        }
    }
    fn fallback(self) -> Option<f32> {
        match self {
            Self::Fixed(n) => Some(n),
            Self::Fit(n) | Self::Fill(n) => n,
        }
    }
}

pub fn compute(scene: ResolvedScene, fonts: &mut Fonts) -> Result<LayoutSnapshot> {
    compute_checked(scene, fonts, &|| Ok(()))
}

/// Cancel at native calculation boundaries without publishing a partial snapshot.
pub fn compute_with_cancellation(
    scene: ResolvedScene,
    fonts: &mut Fonts,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<LayoutSnapshot> {
    compute_checked(scene, fonts, &|| crate::cancel::check(cancel))
}

pub(crate) fn compute_checked(
    scene: ResolvedScene,
    fonts: &mut Fonts,
    check: &dyn Fn() -> Result<()>,
) -> Result<LayoutSnapshot> {
    check()?;
    let mut solver = Solver {
        items: Vec::new(),
        roots: Vec::new(),
        dimensions: Vec::new(),
        active: Vec::new(),
        blocks: BTreeMap::new(),
        fill_allocations: BTreeMap::new(),
        fonts,
        check,
    };
    for node in scene.roots {
        let root = solver.flatten(node, None, true)?;
        solver.roots.push(root);
    }
    for index in 0..solver.items.len() {
        solver.dimension(index, 0)?;
        solver.dimension(index, 1)?;
    }
    for root in solver.roots.clone() {
        solver.items[root].record.rect.x = solver.number(root, "x", 0.0)?;
        solver.items[root].record.rect.y = solver.number(root, "y", 0.0)?;
        solver.position(root, Affine::default())?;
    }
    for root in solver.roots.clone() {
        solver.visual_bounds(root)?;
    }
    let bounds = solver
        .roots
        .iter()
        .filter(|i| solver.items[**i].record.enabled)
        .map(|i| solver.items[*i].record.visual_bounds)
        .reduce(Rect::union)
        .unwrap_or_default();
    check()?;
    let id = snapshot_identity(&scene.revision, &scene.dependency_revision, &solver.items);
    check()?;
    Ok(LayoutSnapshot {
        id,
        revision: scene.revision,
        dependency_revision: scene.dependency_revision,
        dependencies: scene.dependencies,
        items: solver.items,
        roots: solver.roots,
        text_blocks: solver.blocks,
        bounds,
        runtime_dependencies: Vec::new(),
        script_executions: Vec::new(),
    })
}

/// Preserve the original JSON hash without cloning the entire scene
/// into another JSON object graph and a complete serialization buffer.
fn snapshot_identity(revision: &str, dependency_revision: &str, items: &[LayoutItem]) -> String {
    use serde::ser::SerializeSeq;
    use sha2::{Digest, Sha256};
    struct Rows<'a> {
        items: &'a [LayoutItem],
        nodes: bool,
    }
    impl Serialize for Rows<'_> {
        fn serialize<S: serde::Serializer>(
            &self,
            serializer: S,
        ) -> std::result::Result<S::Ok, S::Error> {
            let mut sequence = serializer.serialize_seq(Some(self.items.len()))?;
            for item in self.items {
                // Preserve Value's original number conversion and map order,
                // with only one item's temporary allocations alive at a time.
                let value = if self.nodes {
                    serde_json::to_value(&item.node)
                } else {
                    serde_json::to_value(&item.record)
                }
                .expect("Serializable layout identity");
                sequence.serialize_element(&value)?;
            }
            sequence.end()
        }
    }
    #[derive(Serialize)]
    struct Identity<'a> {
        // serde_json's preserve_order feature retains the original json! order.
        engine: &'static str,
        revision: &'a str,
        dependency_revision: &'a str,
        records: Rows<'a>,
        nodes: Rows<'a>,
    }
    struct Writer<'a>(&'a mut Sha256);
    impl std::io::Write for Writer<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let identity = Identity {
        dependency_revision,
        engine: ENGINE,
        nodes: Rows { items, nodes: true },
        records: Rows {
            items,
            nodes: false,
        },
        revision,
    };
    let mut digest = Sha256::new();
    {
        let mut writer = std::io::BufWriter::with_capacity(64 * 1024, Writer(&mut digest));
        serde_json::to_writer(&mut writer, &identity).expect("Serializable layout identity");
        std::io::Write::flush(&mut writer).expect("In-memory identity hash");
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

struct Solver<'a> {
    items: Vec<LayoutItem>,
    roots: Vec<usize>,
    dimensions: Vec<[Option<f32>; 2]>,
    active: Vec<(usize, usize)>,
    blocks: BTreeMap<usize, TextBlock>,
    fill_allocations: BTreeMap<(usize, usize), f32>,
    fonts: &'a mut Fonts,
    check: &'a dyn Fn() -> Result<()>,
}

impl Solver<'_> {
    fn flatten(
        &mut self,
        mut node: ResolvedNode,
        parent: Option<usize>,
        inherited_enabled: bool,
    ) -> Result<usize> {
        (self.check)()?;
        let children = std::mem::take(&mut node.children);
        let index = self.items.len();
        let enabled =
            inherited_enabled && node.properties.get("enabled") != Some(&Value::Bool(false));
        self.items.push(LayoutItem {
            record: LayoutRecord {
                id: node.id(),
                address: node.address.clone(),
                source_path: node.source_path.clone(),
                source_id: node.source_id.clone(),
                parent_id: parent.map(|p| self.items[p].record.id.clone()),
                node_type: node.kind().into(),
                enabled,
                rect: Rect::default(),
                transform: Affine::default(),
                visual_bounds: Rect::default(),
                sizing: [String::new(), String::new()],
                text: None,
                diagnostics: Vec::new(),
                generated_by: node.generated_by.clone(),
                icon: node.icon_metadata.clone(),
            },
            node,
            parent,
            children: Vec::new(),
        });
        self.dimensions.push([None, None]);
        if enabled
            && let Some(error) = self.items[index]
                .node
                .icon
                .as_ref()
                .and_then(|icon| icon.as_ref().err())
                .cloned()
        {
            let diagnostic = LayoutDiagnostic {
                code: error.code,
                severity: "error",
                node_id: self.items[index].record.id.clone(),
                message: error.message,
                details: error.details.unwrap_or(Value::Null),
            };
            self.items[index].record.diagnostics.push(diagnostic);
        }
        for child in children {
            let child = self.flatten(child, Some(index), enabled)?;
            self.items[index].children.push(child);
        }
        Ok(index)
    }

    fn dimension(&mut self, index: usize, axis: usize) -> Result<f32> {
        (self.check)()?;
        if let Some(value) = self.dimensions[index][axis] {
            return Ok(value);
        }
        if !self.items[index].record.enabled {
            self.dimensions[index][axis] = Some(0.0);
            return Ok(0.0);
        }
        let sizing = self.sizing(index, axis)?;
        if self.active.contains(&(index, axis)) {
            return sizing
                .fallback()
                .ok_or_else(|| self.unresolved(index, axis));
        }
        self.active.push((index, axis));
        let kind = self.items[index].node.kind();
        let is_text = matches!(kind, "text" | "note" | "prompt" | "context");
        let growth = self.string(index, "textGrowth", "auto").to_string();
        let value = if is_text && (growth == "auto" || axis == 1 && growth == "fixed-width") {
            let width = if growth == "auto" {
                None
            } else {
                Some(self.dimension(index, 0)?)
            };
            self.measure_text(index, width)?;
            let metrics = &self.blocks[&index].metrics;
            if axis == 0 {
                metrics.width
            } else {
                metrics.height
            }
        } else {
            match sizing {
                Sizing::Fixed(n) => n,
                Sizing::Fill(fallback) => self.fill(index, axis, fallback)?,
                Sizing::Fit(fallback) => self.content(index, axis, fallback)?,
            }
        };
        self.active.pop();
        if !value.is_finite() || !(0.0..=MAX_DIMENSION).contains(&value) {
            return Err(Error::new(
                "LAYOUT_UNRESOLVED",
                format!(
                    "Dimension outside 0..{MAX_DIMENSION} at {}",
                    self.items[index].record.id
                ),
            ));
        }
        self.dimensions[index][axis] = Some(value);
        self.items[index].record.sizing[axis] =
            if is_text && (growth == "auto" || axis == 1 && growth == "fixed-width") {
                "content"
            } else {
                sizing.name()
            }
            .into();
        if axis == 0 {
            self.items[index].record.rect.width = value;
        } else {
            self.items[index].record.rect.height = value;
        }
        if is_text && growth == "fixed-width-height" && axis == 1 {
            let width = self.dimension(index, 0)?;
            self.measure_text(index, Some(width))?;
        }
        Ok(value)
    }

    fn sizing(&self, index: usize, axis: usize) -> Result<Sizing> {
        let key = if axis == 0 { "width" } else { "height" };
        let node = &self.items[index].node;
        if matches!(node.kind(), "text" | "note" | "prompt" | "context") {
            let growth = self.string(index, "textGrowth", "auto");
            if growth == "auto" || axis == 1 && growth == "fixed-width" {
                return Ok(Sizing::Fit(None));
            }
        }
        let Some(value) = node.properties.get(key) else {
            return Ok(
                if matches!(
                    node.kind(),
                    "frame" | "group" | "text" | "note" | "prompt" | "context"
                ) {
                    Sizing::Fit(None)
                } else {
                    Sizing::Fixed(0.0)
                },
            );
        };
        if let Some(n) = value.as_f64() {
            return Ok(Sizing::Fixed(n as f32));
        }
        let value = value
            .as_str()
            .ok_or_else(|| invalid("Expected a numeric or dynamic dimension"))?;
        for (prefix, fill) in [("fit_content", false), ("fill_container", true)] {
            if let Some(suffix) = value.strip_prefix(prefix) {
                let fallback = if suffix.is_empty() {
                    None
                } else {
                    Some(
                        suffix
                            .strip_prefix('(')
                            .and_then(|s| s.strip_suffix(')'))
                            .and_then(|s| s.parse::<f32>().ok())
                            .filter(|n| n.is_finite() && *n >= 0.0)
                            .ok_or_else(|| invalid("Invalid dynamic dimension fallback"))?,
                    )
                };
                return Ok(if fill {
                    Sizing::Fill(fallback)
                } else {
                    Sizing::Fit(fallback)
                });
            }
        }
        Err(invalid(format!("Unresolved dimension {value:?}")))
    }

    fn flow(&self, index: usize) -> Option<usize> {
        let default = if self.items[index].node.kind() == "frame" {
            "horizontal"
        } else {
            "none"
        };
        match self.string(index, "layout", default) {
            "horizontal" => Some(0),
            "vertical" => Some(1),
            _ => None,
        }
    }

    fn participates(&self, index: usize) -> bool {
        self.items[index].record.enabled
            && self.string(index, "layoutPosition", "auto") != "absolute"
    }

    fn insets(&self, index: usize) -> Result<[f32; 4]> {
        let value = self.items[index].node.properties.get("padding");
        let mut padding = match value {
            None => [0.0; 4],
            Some(Value::Number(n)) => [n.as_f64().unwrap_or(f64::NAN) as f32; 4],
            Some(Value::Array(values)) if matches!(values.len(), 2 | 4) => {
                let values = values
                    .iter()
                    .map(|v| {
                        v.as_f64()
                            .map(|n| n as f32)
                            .ok_or_else(|| invalid("Padding must contain numbers"))
                    })
                    .collect::<Result<Vec<_>>>()?;
                if values.len() == 2 {
                    [values[0], values[1], values[0], values[1]]
                } else {
                    values.try_into().expect("four values")
                }
            }
            _ => {
                return Err(invalid(
                    "Padding must be a number or an array of two or four numbers",
                ));
            }
        };
        if padding.iter().any(|n| !n.is_finite() || *n < 0.0) {
            return Err(invalid("Padding must be finite and nonnegative"));
        }
        if self.items[index].node.properties.get("layoutIncludeStroke") == Some(&Value::Bool(true))
        {
            let widths = stroke_widths(&self.items[index].node)?;
            let fraction = match self.string(index, "strokeAlignment", "center") {
                "inner" => 1.0,
                "outer" => 0.0,
                _ => 0.5,
            };
            for (padding, width) in padding.iter_mut().zip(widths) {
                *padding += width * fraction;
            }
        }
        Ok(padding)
    }

    fn fill(&mut self, index: usize, axis: usize, fallback: Option<f32>) -> Result<f32> {
        (self.check)()?;
        let Some(parent) = self.items[index].parent else {
            return Ok(fallback.unwrap_or(0.0));
        };
        let Some(flow) = self.flow(parent).filter(|_| self.participates(index)) else {
            return Ok(fallback.unwrap_or(0.0));
        };
        if self.active.contains(&(parent, axis))
            && let Some(fallback) = fallback
        {
            return Ok(fallback);
        }
        let size = self.dimension(parent, axis)?;
        let padding = self.insets(parent)?;
        let inner = (size
            - if axis == 0 {
                padding[3] + padding[1]
            } else {
                padding[0] + padding[2]
            })
        .max(0.0);
        if flow != axis {
            return Ok(inner);
        }
        if let Some(allocation) = self.fill_allocations.get(&(parent, axis)) {
            return Ok(*allocation);
        }
        let children = self.items[parent].children.clone();
        let mut fixed = 0.0;
        let mut fills = 0;
        let mut count = 0usize;
        for sibling in children {
            if !self.participates(sibling) {
                continue;
            }
            count += 1;
            if matches!(self.sizing(sibling, axis)?, Sizing::Fill(_)) {
                fills += 1;
            } else {
                fixed += self.dimension(sibling, axis)?;
            }
        }
        let gap = self.number(parent, "gap", 0.0)?;
        let allocation = ((inner - fixed - gap * (count.saturating_sub(1) as f32)).max(0.0))
            / (fills.max(1) as f32);
        self.fill_allocations.insert((parent, axis), allocation);
        Ok(allocation)
    }

    fn content(&mut self, index: usize, axis: usize, fallback: Option<f32>) -> Result<f32> {
        (self.check)()?;
        let kind = self.items[index].node.kind().to_string();
        if matches!(kind.as_str(), "text" | "note" | "prompt" | "context") {
            if axis == 0 {
                self.measure_text(index, None)?;
                return Ok(self.blocks[&index].metrics.width);
            }
            let width = self.dimension(index, 0)?;
            self.measure_text(index, Some(width))?;
            return Ok(self.blocks[&index].metrics.height);
        }
        let flow = self.flow(index);
        if flow.is_none() && !matches!(kind.as_str(), "group" | "script") {
            return Ok(fallback.unwrap_or(0.0));
        }
        let children = self.items[index].children.clone();
        let padding = self.insets(index)?;
        let mut sizes = Vec::new();
        let mut min = 0.0f32;
        let mut max = 0.0f32;
        for child in children {
            if !self.participates(child) && flow.is_some() || !self.items[child].record.enabled {
                continue;
            }
            let size = self.dimension(child, axis)?;
            sizes.push(size);
            if flow.is_none() {
                let position = self.number(child, if axis == 0 { "x" } else { "y" }, 0.0)?;
                min = min.min(position);
                max = max.max(position + size);
            }
        }
        if sizes.is_empty() {
            return Ok(fallback.unwrap_or(0.0));
        }
        let content = match flow {
            Some(flow) if flow == axis => {
                sizes.iter().sum::<f32>()
                    + self.number(index, "gap", 0.0)? * (sizes.len().saturating_sub(1) as f32)
            }
            Some(_) => sizes.into_iter().fold(0.0, f32::max),
            None => max - min,
        };
        Ok(content
            + if axis == 0 {
                padding[3] + padding[1]
            } else {
                padding[0] + padding[2]
            })
    }

    fn measure_text(&mut self, index: usize, width: Option<f32>) -> Result<()> {
        (self.check)()?;
        if self
            .blocks
            .get(&index)
            .is_some_and(|b| width.is_none_or(|w| w == b.metrics.width))
        {
            return Ok(());
        }
        let node = &self.items[index].node;
        let content = node
            .properties
            .get("content")
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| invalid("Text content must resolve to a string"))
            })
            .transpose()?
            .unwrap_or_default();
        let weight = match node.properties.get("fontWeight") {
            Some(Value::String(s)) => s
                .parse()
                .map_err(|_| invalid("Font weight must be an integer string"))?,
            None => 400,
            _ => return Err(invalid("Font weight must be a string")),
        };
        let spec = TextSpec {
            content,
            family: self
                .string(index, "fontFamily", self.fonts.default_family())
                .into(),
            size: self.number(index, "fontSize", 16.0)?,
            weight,
            italic: self.string(index, "fontStyle", "normal") == "italic",
            underline: boolean(node, "underline", false)?,
            strikethrough: boolean(node, "strikethrough", false)?,
            outline_required: stroke_widths(node)?.iter().any(|width| *width > 0.0),
            letter_spacing: self.number(index, "letterSpacing", 0.0)?,
            line_height: node
                .properties
                .get("lineHeight")
                .map(|_| self.number(index, "lineHeight", 1.0))
                .transpose()?,
            color: Color::WHITE,
            align: match self.string(index, "textAlign", "left") {
                "left" => TextAlign::Left,
                "center" => TextAlign::Center,
                "right" => TextAlign::Right,
                "justify" => TextAlign::Justify,
                _ => return Err(invalid("Unknown text alignment")),
            },
            locale: self.fonts.locale().into(),
        };
        let block = self.fonts.layout_checked(&spec, width, self.check)?;
        self.items[index].record.text = Some(block.metrics.clone());
        self.blocks.insert(index, block);
        Ok(())
    }

    fn position(&mut self, index: usize, parent_transform: Affine) -> Result<()> {
        (self.check)()?;
        let rect = self.items[index].record.rect;
        let rotation = self.number(index, "rotation", 0.0)?;
        let xflip = self.items[index].node.properties.get("flipX") == Some(&Value::Bool(true));
        let yflip = self.items[index].node.properties.get("flipY") == Some(&Value::Bool(true));
        let transform = parent_transform
            .then(Affine::translation(rect.x, rect.y))
            .then(Affine::rotation(rotation))
            .then(Affine::translation(
                if xflip { rect.width } else { 0.0 },
                if yflip { rect.height } else { 0.0 },
            ))
            .then(Affine::scale(
                if xflip { -1.0 } else { 1.0 },
                if yflip { -1.0 } else { 1.0 },
            ));
        self.items[index].record.transform = transform;
        let children = self.items[index].children.clone();
        if let Some(flow) = self.flow(index) {
            let padding = self.insets(index)?;
            let main_start = if flow == 0 { padding[3] } else { padding[0] };
            let main_end = if flow == 0 { padding[1] } else { padding[2] };
            let cross_start = if flow == 0 { padding[0] } else { padding[3] };
            let cross_end = if flow == 0 { padding[2] } else { padding[1] };
            let parent_main = if flow == 0 { rect.width } else { rect.height };
            let parent_cross = if flow == 0 { rect.height } else { rect.width };
            let flow_children = children
                .iter()
                .copied()
                .filter(|i| self.participates(*i))
                .collect::<Vec<_>>();
            let sizes = flow_children
                .iter()
                .map(|i| self.dimensions[*i][flow].expect("resolved"))
                .sum::<f32>();
            let gap = self.number(index, "gap", 0.0)?;
            let spare = parent_main
                - main_start
                - main_end
                - sizes
                - gap * (flow_children.len().saturating_sub(1) as f32);
            let (offset, extra_gap) = match self.string(index, "justifyContent", "start") {
                "start" => (0.0, 0.0),
                "center" => (spare / 2.0, 0.0),
                "end" => (spare, 0.0),
                "space_between" if flow_children.len() > 1 => {
                    (0.0, spare.max(0.0) / (flow_children.len() - 1) as f32)
                }
                "space_between" => (0.0, 0.0),
                "space_around" if !flow_children.is_empty() => {
                    let space = spare.max(0.0) / flow_children.len() as f32;
                    (space / 2.0, space)
                }
                "space_around" => (0.0, 0.0),
                _ => return Err(invalid("Unknown main-axis alignment")),
            };
            let mut main = main_start + offset;
            for child in flow_children {
                let cross_size = self.dimensions[child][1 - flow].expect("resolved");
                let spare = parent_cross - cross_start - cross_end - cross_size;
                let cross = cross_start
                    + match self.string(index, "alignItems", "start") {
                        "start" => 0.0,
                        "center" => spare / 2.0,
                        "end" => spare,
                        _ => return Err(invalid("Unknown cross-axis alignment")),
                    };
                self.items[child].record.rect.x = if flow == 0 { main } else { cross };
                self.items[child].record.rect.y = if flow == 0 { cross } else { main };
                main += self.dimensions[child][flow].expect("resolved") + gap + extra_gap;
            }
        }
        for child in children {
            if self.flow(index).is_none() || !self.participates(child) {
                self.items[child].record.rect.x = self.number(child, "x", 0.0)?;
                self.items[child].record.rect.y = self.number(child, "y", 0.0)?;
            }
            self.position(child, transform)?;
            let child_rect = self.items[child].record.rect;
            let overflow = json!({"left":(-child_rect.x).max(0.0),"top":(-child_rect.y).max(0.0),
                "right":(child_rect.right()-rect.width).max(0.0),"bottom":(child_rect.bottom()-rect.height).max(0.0),
                "clipped":self.items[index].node.properties.get("clip")==Some(&Value::Bool(true))});
            if self.items[child].record.enabled
                && (child_rect.x < 0.0
                    || child_rect.y < 0.0
                    || child_rect.right() > rect.width + 0.01
                    || child_rect.bottom() > rect.height + 0.01)
            {
                self.warn(
                    child,
                    "LAYOUT_OVERFLOW",
                    "The child extends beyond its parent",
                    overflow,
                );
            }
        }
        if let Some(metrics) = self.items[index].record.text.clone() {
            let vertical = self.string(index, "textAlignVertical", "top");
            let offset = match vertical {
                "top" => 0.0,
                "middle" => (rect.height - metrics.height) / 2.0,
                "bottom" => rect.height - metrics.height,
                _ => return Err(invalid("Unknown vertical text alignment")),
            };
            if metrics.height > rect.height + 0.01 || metrics.longest_line > rect.width + 0.01 {
                self.warn(index,"TEXT_OVERFLOW","The shaped text extends beyond its text box",json!({"text_width":metrics.longest_line,"text_height":metrics.height,"vertical_offset":offset,"box":rect}));
            }
        }
        Ok(())
    }

    fn visual_bounds(&mut self, index: usize) -> Result<Rect> {
        (self.check)()?;
        let rect = self.items[index].record.rect;
        let transform = self.items[index].record.transform;
        let widths = stroke_widths(&self.items[index].node)?;
        let outside = match self.string(index, "strokeAlignment", "center") {
            "inner" => 0.0,
            "outer" => 1.0,
            "center" => 0.5,
            _ => return Err(invalid("Unknown stroke alignment")),
        };
        let stroke = widths.into_iter().fold(0.0, f32::max) * outside;
        let mut local = Rect::new(
            -widths[3] * outside,
            -widths[0] * outside,
            rect.width + (widths[3] + widths[1]) * outside,
            rect.height + (widths[0] + widths[2]) * outside,
        );
        if let Some(text) = &self.items[index].record.text {
            let x = match self.string(index, "textAlign", "left") {
                "right" => rect.width - text.longest_line,
                "center" => (rect.width - text.longest_line) / 2.0,
                _ => 0.0,
            };
            let y = match self.string(index, "textAlignVertical", "top") {
                "bottom" => rect.height - text.height,
                "middle" => (rect.height - text.height) / 2.0,
                _ => 0.0,
            };
            local =
                local.union(Rect::new(x, y, text.longest_line, text.height).outset(stroke, stroke));
            if let Some(block) = self.blocks.get(&index) {
                let ink = block.ink_bounds;
                local = local.union(Rect::new(ink.x, ink.y + y, ink.width, ink.height));
            }
            if let Some(outline) = self
                .blocks
                .get(&index)
                .and_then(|block| block.outline.as_ref())
            {
                let stroked = crate::render::stroke_geometry(
                    &self.items[index].node,
                    outline,
                    skia_safe::Matrix::default(),
                )?;
                let ink = stroked.bounds();
                local = local.union(Rect::new(
                    ink.left(),
                    ink.top() + y,
                    ink.width(),
                    ink.height(),
                ));
            }
        } else if widths.iter().any(|width| *width > 0.0) {
            let node = &self.items[index].node;
            let shape =
                crate::render::shape(node, skia_safe::Rect::from_wh(rect.width, rect.height))?;
            let geometry =
                crate::render::stroke_geometry(node, &shape, skia_safe::Matrix::default())?;
            let ink = geometry.bounds();
            local = local.union(Rect::new(ink.left(), ink.top(), ink.width(), ink.height()));
        }
        let mut bounds = local.transformed(transform);
        for child in self.items[index].children.clone() {
            let child_bounds = self.visual_bounds(child)?;
            if self.items[child].record.enabled {
                bounds = bounds.union(
                    if self.items[index].node.properties.get("clip") == Some(&Value::Bool(true)) {
                        child_bounds.intersection(
                            Rect::new(0.0, 0.0, rect.width, rect.height).transformed(transform),
                        )
                    } else {
                        child_bounds
                    },
                );
            }
        }
        if let Some(effects) = self.items[index].node.properties.get("effect") {
            let effects = effects
                .as_array()
                .map(|v| v.iter().collect::<Vec<_>>())
                .unwrap_or_else(|| vec![effects]);
            for effect in effects {
                if effect["enabled"] == false {
                    continue;
                }
                let blur = match effect["type"].as_str() {
                    Some("blur") => effect.get("radius"),
                    Some("shadow")
                        if effect
                            .get("shadowType")
                            .and_then(Value::as_str)
                            .unwrap_or("outer")
                            == "outer" =>
                    {
                        effect.get("blur")
                    }
                    _ => continue,
                }
                .map(|v| {
                    v.as_f64()
                        .filter(|n| n.is_finite() && *n >= 0.0)
                        .map(|n| n as f32)
                        .ok_or_else(|| invalid("Effect blur must be a nonnegative number"))
                })
                .transpose()?
                .unwrap_or(0.0);
                let [a, b, c, d, _, _] = transform.0;
                let expanded = bounds.outset(
                    1.5 * blur * (a.abs() + c.abs()),
                    1.5 * blur * (b.abs() + d.abs()),
                );
                if effect["type"] == "shadow" {
                    let x = effect
                        .get("offset")
                        .and_then(|v| v.get("x"))
                        .map(|v| {
                            v.as_f64()
                                .map(|n| n as f32)
                                .ok_or_else(|| invalid("Shadow offsets must be numbers"))
                        })
                        .transpose()?
                        .unwrap_or(0.0);
                    let y = effect
                        .get("offset")
                        .and_then(|v| v.get("y"))
                        .map(|v| {
                            v.as_f64()
                                .map(|n| n as f32)
                                .ok_or_else(|| invalid("Shadow offsets must be numbers"))
                        })
                        .transpose()?
                        .unwrap_or(0.0);
                    let shadow = Rect::new(
                        expanded.x + a * x + c * y,
                        expanded.y + b * x + d * y,
                        expanded.width,
                        expanded.height,
                    );
                    bounds = bounds.union(shadow);
                } else {
                    bounds = expanded;
                }
            }
        }
        self.items[index].record.visual_bounds = bounds;
        Ok(bounds)
    }

    fn warn(&mut self, index: usize, code: &'static str, message: &str, details: Value) {
        let node_id = self.items[index].record.id.clone();
        self.items[index].record.diagnostics.push(LayoutDiagnostic {
            code,
            severity: "warning",
            node_id,
            message: message.into(),
            details,
        });
    }
    fn number(&self, index: usize, key: &str, default: f32) -> Result<f32> {
        match self.items[index].node.properties.get(key) {
            None => Ok(default),
            Some(v) => v
                .as_f64()
                .map(|n| n as f32)
                .filter(|n| n.is_finite() && n.abs() <= MAX_DIMENSION)
                .ok_or_else(|| {
                    invalid(format!(
                        "{}.{} must resolve to a finite number",
                        self.items[index].record.id, key
                    ))
                }),
        }
    }
    fn string<'a>(&'a self, index: usize, key: &str, default: &'a str) -> &'a str {
        self.items[index]
            .node
            .properties
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or(default)
    }
    fn unresolved(&self, index: usize, axis: usize) -> Error {
        Error::new("LAYOUT_UNRESOLVED", "A content/fill sizing cycle has no intrinsic size or explicit fallback")
            .details(json!({"node_id":self.items[index].record.id,"axis":if axis==0 { "width" } else { "height" },
                "cycle":self.active.iter().map(|(i,a)| json!({"node_id":self.items[*i].record.id,"axis":a})).collect::<Vec<_>>()}))
    }
}

pub fn stroke_widths(node: &ResolvedNode) -> Result<[f32; 4]> {
    let Some(stroke) = node.properties.get("stroke") else {
        return Ok([0.0; 4]);
    };
    if stroke
        .as_array()
        .map(|fills| fills.iter().all(|fill| fill["enabled"] == false))
        .unwrap_or_else(|| stroke["enabled"] == false)
    {
        return Ok([0.0; 4]);
    }
    let value = node.properties.get("strokeWidth");
    let result = match value {
        None => [1.0; 4],
        Some(Value::Number(n)) => [n.as_f64().unwrap_or(f64::NAN) as f32; 4],
        Some(Value::Object(map)) => {
            let mut widths = [0.0; 4];
            for (i, key) in ["top", "right", "bottom", "left"].iter().enumerate() {
                widths[i] = map
                    .get(*key)
                    .map(|v| {
                        v.as_f64()
                            .map(|n| n as f32)
                            .ok_or_else(|| invalid("Stroke widths must be numbers"))
                    })
                    .transpose()?
                    .unwrap_or(0.0);
            }
            widths
        }
        _ => return Err(invalid("Invalid stroke width")),
    };
    if result.iter().any(|n| !n.is_finite() || *n < 0.0) {
        return Err(invalid("Stroke widths must be finite and nonnegative"));
    }
    // Native 2.19 uses the top entry as a uniform width on other shapes.
    // Resolve this once so painting, glyph outlines and visual bounds agree.
    Ok(
        if matches!(node.kind(), "rectangle" | "frame" | "browser") {
            result
        } else {
            [result[0]; 4]
        },
    )
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new("INVALID_DOCUMENT", message)
}

fn boolean(node: &ResolvedNode, property: &str, default: bool) -> Result<bool> {
    node.properties
        .get(property)
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| invalid(format!("{property} must be a boolean")))
        })
        .unwrap_or(Ok(default))
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use crate::{document::Document, project::Project};
    use std::cell::Cell;

    #[test]
    fn native_cancellation_interrupts_partial_layout_and_preserves_recovery_identity() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("scene.pen");
        let rows = (0..40).map(|index| json!({"type":"frame","id":format!("row-{index}"),"layout":"horizontal","width":240,"height":"fit_content","gap":4,"children":[
            {"id":format!("label-{index}"),"type":"text","content":"日本語 native layout","fontSize":12,"fontFamily":"Noto Sans JP","width":"fill_container","textGrowth":"fixed-width","stroke":"#000000","strokeWidth":0.25},
            {"id":format!("mark-{index}"),"type":"rectangle","width":10,"height":10,"fill":"#336699"}
        ]})).collect::<Vec<_>>();
        let source = json!({"version":"2.21","children":[{"id":"canvas","type":"frame","width":240,"height":"fit_content","layout":"vertical","children":rows}]}).to_string();
        std::fs::write(&path, &source).unwrap();
        let project =
            Project::from_document(&path, Document::parse(source.clone()).unwrap(), &[]).unwrap();
        let mut fonts = Fonts::new(&[]).unwrap();
        let count = Cell::new(0usize);
        let baseline = compute_checked(project.resolve().unwrap(), &mut fonts, &|| {
            count.set(count.get() + 1);
            Ok(())
        })
        .unwrap();
        let original_identity = json!({"engine":ENGINE,"revision":baseline.revision,"dependency_revision":baseline.dependency_revision,
            "records":baseline.items.iter().map(|item| &item.record).collect::<Vec<_>>(),
            "nodes":baseline.items.iter().map(|item| &item.node).collect::<Vec<_>>()});
        assert_eq!(baseline.id, hash(original_identity.to_string().as_bytes()));
        let total = count.get();
        assert!(
            total > 400,
            "Expected substantial native calculation, got {total} checks"
        );
        let baseline_records = serde_json::to_value(baseline.records()).unwrap();
        for cutoff in [1, total / 4, total / 2, total - 1] {
            let cancel = tokio_util::sync::CancellationToken::new();
            let checks = Cell::new(0);
            let result = compute_checked(project.resolve().unwrap(), &mut fonts, &|| {
                checks.set(checks.get() + 1);
                if checks.get() == cutoff {
                    cancel.cancel();
                }
                crate::cancel::check(Some(&cancel))
            });
            assert_eq!(
                result
                    .err()
                    .expect("A partial calculation must not publish a snapshot")
                    .code,
                "CANCELLED"
            );
            assert_eq!(checks.get(), cutoff);
            let recovered =
                compute_with_cancellation(project.resolve().unwrap(), &mut fonts, None).unwrap();
            assert_eq!(recovered.id, baseline.id);
            assert_eq!(
                serde_json::to_value(recovered.records()).unwrap(),
                baseline_records
            );
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
    }

    #[test]
    fn native_cancellation_checked_layout_preserves_deadline_errors_and_priority() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("scene.pen");
        let source = json!({"version":"2.21","children":[{"id":"box","type":"rectangle","width":20,"height":20}]}).to_string();
        std::fs::write(&path, &source).unwrap();
        let project = Project::from_document(&path, Document::parse(source).unwrap(), &[]).unwrap();
        let mut fonts = Fonts::new(&[]).unwrap();
        let checks = Cell::new(0);
        let error = compute_checked(project.resolve().unwrap(), &mut fonts, &|| {
            checks.set(checks.get() + 1);
            if checks.get() == 4 {
                Err(Error::new(
                    "RENDER_TIMEOUT",
                    "Native layout exceeded its calculation budget",
                ))
            } else {
                Ok(())
            }
        })
        .err()
        .unwrap();
        assert_eq!(error.code, "RENDER_TIMEOUT");
        assert_eq!(checks.get(), 4);
        let cancel = tokio_util::sync::CancellationToken::new();
        cancel.cancel();
        let error =
            compute_with_cancellation(project.resolve().unwrap(), &mut fonts, Some(&cancel))
                .err()
                .unwrap();
        assert_eq!(error.code, "CANCELLED");
        compute(project.resolve().unwrap(), &mut fonts).unwrap();
    }
}
