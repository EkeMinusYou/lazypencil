//! Normalize hash-verified upstream data without evaluating package code.

use super::*;

pub(super) fn license(id: &str) -> Result<&'static str> {
    Ok(match id {
        "phosphor" => include_str!("../../resources/icons/phosphor/LICENSE"),
        "Material Symbols Outlined" => {
            include_str!("../../resources/icons/Material Symbols Outlined/LICENSE")
        }
        "Material Symbols Rounded" => {
            include_str!("../../resources/icons/Material Symbols Rounded/LICENSE")
        }
        "Material Symbols Sharp" => {
            include_str!("../../resources/icons/Material Symbols Sharp/LICENSE")
        }
        _ => return Err(integrity("Icon pack lacks an embedded license")),
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PhosphorIndex {
    tags: BTreeMap<String, Vec<String>>,
    aliases: BTreeMap<String, String>,
}

pub(super) fn phosphor(source: &[u8], descriptor: &Descriptor) -> Result<(Vec<u8>, Vec<u8>)> {
    let index_bytes = include_bytes!("../../resources/icons/phosphor/index.json");
    verify(
        index_bytes,
        descriptor
            .index_sha256
            .as_deref()
            .ok_or_else(|| integrity("Missing icon index pin"))?,
    )?;
    let index: PhosphorIndex =
        serde_json::from_slice(index_bytes).map_err(|e| integrity(e.to_string()))?;
    let reader = flate2::read::GzDecoder::new(source).take(16 * 1024 * 1024);
    let mut archive = tar::Archive::new(reader);
    let mut icons = BTreeMap::new();
    let mut license_bytes = None;
    let mut count = 0;
    for entry in archive.entries().map_err(|e| integrity(e.to_string()))? {
        crate::cancel::check(Some(&crate::cancel::token()))?;
        count += 1;
        if count > 10000 {
            return Err(integrity("Too many icon archive members"));
        }
        let mut entry = entry.map_err(|e| integrity(e.to_string()))?;
        let path = entry
            .path()
            .map_err(|e| integrity(e.to_string()))?
            .into_owned();
        let name = path
            .to_str()
            .ok_or_else(|| integrity("Invalid icon member path"))?;
        let svg = name
            .strip_prefix("package/assets/regular/")
            .and_then(|name| name.strip_suffix(".svg"));
        if name != "package/LICENSE" && svg.is_none() {
            continue;
        }
        if !entry.header().entry_type().is_file() || entry.size() > 64 * 1024 {
            return Err(integrity("Invalid pinned Phosphor member"));
        }
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|e| integrity(e.to_string()))?;
        let Some(name) = svg else {
            if license_bytes.replace(bytes).is_some() {
                return Err(integrity("Duplicate icon license"));
            }
            continue;
        };
        if !index.tags.contains_key(name) {
            return Err(integrity("Unexpected Phosphor name"));
        }
        let source =
            std::str::from_utf8(&bytes).map_err(|_| integrity("Icon SVG must be UTF-8"))?;
        let xml = roxmltree::Document::parse(source).map_err(|e| integrity(e.to_string()))?;
        let root = xml.root_element();
        if root.tag_name().name() != "svg"
            || root.attribute("viewBox") != Some("0 0 256 256")
            || root.attribute("fill") != Some("currentColor")
        {
            return Err(integrity("Unexpected Phosphor SVG viewport/paint"));
        }
        let paths = root
            .children()
            .filter(|n| n.is_element())
            .collect::<Vec<_>>();
        if paths.len() != 1
            || paths[0].tag_name().name() != "path"
            || paths[0].attributes().len() != 1
            || paths[0].children().any(|n| n.is_element())
        {
            return Err(integrity("Unexpected Phosphor path"));
        }
        let d = paths[0]
            .attribute("d")
            .ok_or_else(|| integrity("Missing icon path"))?;
        let shapes = vec![Primitive(
            "path".into(),
            BTreeMap::from([
                ("d".into(), d.into()),
                ("fill".into(), "currentColor".into()),
            ]),
        )];
        if icons.insert(name.to_string(), shapes).is_some() {
            return Err(integrity("Duplicate Phosphor icon"));
        }
    }
    if icons.len() != index.tags.len() {
        return Err(integrity("Phosphor catalogue and SVGs differ"));
    }
    let license = license_bytes.ok_or_else(|| integrity("Missing Phosphor license"))?;
    verify(&license, &descriptor.license_sha256)?;
    let data = Data {
        schema: 2,
        id: descriptor.id.clone(),
        version: descriptor.version.clone(),
        view_box: [0.0, 0.0, 256.0, 256.0],
        stroke_width: 0.0,
        icons,
        aliases: index.aliases,
        tags: index.tags,
        font: None,
    };
    let bytes = serde_json::to_vec(&data).map_err(|e| integrity(e.to_string()))?;
    parse(&bytes, descriptor)?;
    Ok((bytes, license))
}

pub(super) fn material(descriptor: &Descriptor) -> Result<(Vec<u8>, Vec<u8>)> {
    let index: &[u8] = match descriptor.id.as_str() {
        "Material Symbols Outlined" => {
            include_bytes!("../../resources/icons/Material Symbols Outlined/codepoints.txt")
        }
        "Material Symbols Rounded" => {
            include_bytes!("../../resources/icons/Material Symbols Rounded/codepoints.txt")
        }
        "Material Symbols Sharp" => {
            include_bytes!("../../resources/icons/Material Symbols Sharp/codepoints.txt")
        }
        _ => return Err(integrity("Unknown Material library")),
    };
    verify(
        index,
        descriptor
            .index_sha256
            .as_deref()
            .ok_or_else(|| integrity("Missing codepoint pin"))?,
    )?;
    let mut names = BTreeMap::new();
    for line in std::str::from_utf8(index)
        .map_err(|_| integrity("Invalid codepoint text"))?
        .lines()
    {
        let fields = line.split_ascii_whitespace().collect::<Vec<_>>();
        if fields.len() != 2 {
            return Err(integrity("Invalid codepoint entry"));
        }
        let codepoint = u32::from_str_radix(fields[1], 16)
            .ok()
            .filter(|value| *value != 0 && char::from_u32(*value).is_some())
            .ok_or_else(|| integrity("Invalid Unicode codepoint"))?;
        if names.insert(fields[0].to_string(), codepoint).is_some() {
            return Err(integrity("Duplicate codepoint name"));
        }
    }
    let data = Data {
        schema: 2,
        id: descriptor.id.clone(),
        version: descriptor.version.clone(),
        view_box: [0.0, -960.0, 960.0, 960.0],
        stroke_width: 0.0,
        icons: names
            .keys()
            .map(|name| (name.clone(), Vec::new()))
            .collect(),
        aliases: BTreeMap::new(),
        tags: BTreeMap::new(),
        font: Some(names),
    };
    let bytes = serde_json::to_vec(&data).map_err(|e| integrity(e.to_string()))?;
    parse(&bytes, descriptor)?;
    let license = license(&descriptor.id)?.as_bytes().to_vec();
    verify(&license, &descriptor.license_sha256)?;
    Ok((bytes, license))
}
