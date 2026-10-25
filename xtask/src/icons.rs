use crate::util;
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use clap::Subcommand;
use flate2::read::GzDecoder;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha512};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const COMMIT: &str = "10ecc14790612ca8fd164e3f4ffe9c16fc50d5b7";
const PHOSPHOR_SHA: &str = "313332be6190b724da24107addd781799b48bf76b13963f24501112ffe1baadd";
const PHOSPHOR_SRI: &str = "sha512-v4ARvrip4qBCImOE5rmPUylOEK4iiED9ZyKjcvzuezqMaiRASCHKcRIuvvxL/twvLpkfnEODCOJp5dM4eZilxQ==";
const CODEPOINT_SHA: &str = "a7404d9e5a1b63f496cf05ace45d386075886b5a7f123fba9f29527cb6006fde";
const LICENSE_SHA: &str = "58d1e17ffe5109a7ae296caafcadfdbe6a7d176f0bc4ab01e12a689b0499d8bd";
const FONTS: &[(&str, &str)] = &[
    (
        "Outlined",
        "f7837ba8573d9a7a74931618ced8307bc81f96d49af296bf6c8052c772ed5282",
    ),
    (
        "Rounded",
        "7e74e36533ccee12c2b81e75bfcf3ccf123d3bb7e7e46d65d94445f23db0c97c",
    ),
    (
        "Sharp",
        "1fd72f99ec1f44ec461c74f99341c1dcfa8c140d7087a2902e747d4ee01e379b",
    ),
];

#[derive(Subcommand)]
pub enum Command {
    Base {
        archives: PathBuf,
        #[arg(long, default_value = "resources/icons")]
        output: PathBuf,
    },
    Optional {
        sources: PathBuf,
        #[arg(long, default_value = "resources/icons")]
        output: PathBuf,
    },
}
impl Command {
    pub fn run(self) -> Result<()> {
        match self {
            Self::Base { archives, output } => {
                for descriptor in base(&archives, &output)? {
                    util::print(&descriptor)?;
                }
                Ok(())
            }
            Self::Optional { sources, output } => {
                let mut catalogue: Vec<Value> = util::json(&output.join("catalogue.json"))?;
                let mut records = vec![phosphor(&sources, &output)?];
                for (style, _) in FONTS {
                    records.push(material(&sources, &output, style)?);
                }
                catalogue.retain(|d| !records.iter().any(|(v, _)| v["id"] == d["id"]));
                catalogue.extend(records.iter().map(|(d, _)| d.clone()));
                util::publish(
                    &output.join("catalogue.json"),
                    &util::encode(&catalogue)?,
                    true,
                )?;
                for (descriptor, provenance) in records {
                    let mut record = provenance;
                    record["descriptor"] = descriptor.clone();
                    util::publish(
                        &output
                            .join(descriptor["id"].as_str().unwrap())
                            .join("provenance.json"),
                        &util::encode(&record)?,
                        true,
                    )?;
                    util::print(&descriptor)?;
                }
                Ok(())
            }
        }
    }
}

fn checked(path: &Path, expected: &str) -> Result<Vec<u8>> {
    let bytes = util::read(path, 256 * 1024 * 1024)?;
    ensure!(
        util::digest(&bytes) == expected,
        "Source differs from its pin: {}",
        path.display()
    );
    Ok(bytes)
}
fn sri(bytes: &[u8]) -> String {
    format!("sha512-{}", STANDARD.encode(Sha512::digest(bytes)))
}

fn archive(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut result = BTreeMap::new();
    let mut total = 0u64;
    let mut archive = tar::Archive::new(GzDecoder::new(bytes));
    for entry in archive.entries()? {
        let mut entry = entry?;
        if entry.header().entry_type().is_dir() {
            continue;
        }
        let name = String::from_utf8(entry.path_bytes().into_owned())?;
        util::safe_path(&name)?;
        ensure!(
            entry.header().entry_type().is_file() && entry.link_name()?.is_none(),
            "Linked npm member"
        );
        total = total
            .checked_add(entry.size())
            .context("npm archive size overflow")?;
        ensure!(
            entry.size() <= 32 * 1024 * 1024
                && result.len() < 100_000
                && total <= 512 * 1024 * 1024,
            "npm archive exceeds inspection budget"
        );
        let mut body = Vec::new();
        entry.read_to_end(&mut body)?;
        ensure!(result.insert(name, body).is_none(), "Duplicate npm member");
    }
    Ok(result)
}

#[derive(Serialize)]
struct Data<'a> {
    schema: u32,
    id: &'a str,
    version: &'a str,
    view_box: [f64; 4],
    stroke_width: f64,
    icons: &'a BTreeMap<String, Value>,
    aliases: &'a BTreeMap<String, String>,
    tags: &'a BTreeMap<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    font: Option<&'a BTreeMap<String, u32>>,
}

fn shapes(element: roxmltree::Node<'_, '_>) -> Result<Value> {
    let mut result = Vec::new();
    for child in element.children().filter(|n| n.is_element()) {
        let kind = child.tag_name().name();
        ensure!(
            [
                "path", "line", "circle", "ellipse", "rect", "polygon", "polyline"
            ]
            .contains(&kind)
                && !child.children().any(|n| n.is_element()),
            "Unexpected SVG shape"
        );
        let attributes = child
            .attributes()
            .map(|a| (a.name().to_owned(), a.value().to_owned()))
            .collect::<BTreeMap<_, _>>();
        result.push(json!([kind, attributes]));
    }
    Ok(json!(result))
}

pub fn base(directory: &Path, destination: &Path) -> Result<Vec<Value>> {
    let sources = [
        (
            "lucide",
            "lucide-static",
            "1.57.0",
            "afc8602c5231ce64ee227286f93772d0438eddd3b947b645bc14bfd1e49da518",
            "sha512-nuf4YoaKt3Pq9scXVQ71SLr0Q1/UZiTcfBBivFsCVL6biAzPplTikVWXLetBmK3oFEJS6f2L1rzgEsoJN/Ce0Q==",
            "ISC AND MIT",
        ),
        (
            "feather",
            "feather-icons",
            "4.29.2",
            "28b6cad56600feee25c53051d65326621f2675d30d6f3d9bd3175f6117f84476",
            "sha512-0TaCFTnBTVCz6U+baY2UJNKne5ifGh7sMG4ZC2LoBWCZdIyPa+y6UiR4lEYGws1JOFWdee8KAsAIvu0VcXqiqA==",
            "MIT",
        ),
    ];
    let mut descriptors = Vec::new();
    for (pack, package, version, expected, integrity, license) in sources {
        let body = checked(
            &directory.join(format!("{package}-{version}.tgz")),
            expected,
        )?;
        ensure!(sri(&body) == integrity, "npm source integrity mismatch");
        let members = archive(&body)?;
        let legal = members
            .get("package/LICENSE")
            .context("Missing npm original license")?;
        let mut aliases = BTreeMap::new();
        let mut icons = BTreeMap::new();
        let tags;
        if pack == "lucide" {
            let values: Value = serde_json::from_slice(
                members
                    .get("package/icon-nodes.json")
                    .context("Missing Lucide nodes")?,
            )?;
            icons = values
                .as_object()
                .context("Invalid Lucide nodes")?
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let mut matching = BTreeMap::new();
            for (name, value) in icons.iter().rev() {
                matching.insert(serde_json::to_vec(value)?, name.clone());
            }
            for (name, bytes) in &members {
                if name.starts_with("package/icons/") && name.ends_with(".svg") {
                    let id = Path::new(name).file_stem().unwrap().to_str().unwrap();
                    let doc = roxmltree::Document::parse(std::str::from_utf8(bytes)?)?;
                    let element = doc.root_element();
                    for (key, value) in [
                        ("viewBox", "0 0 24 24"),
                        ("fill", "none"),
                        ("stroke", "currentColor"),
                        ("stroke-width", "2"),
                        ("stroke-linecap", "round"),
                        ("stroke-linejoin", "round"),
                    ] {
                        ensure!(
                            element.attribute(key) == Some(value),
                            "Unexpected Lucide SVG attribute"
                        );
                    }
                    let value = shapes(element)?;
                    if let Some(expected) = icons.get(id) {
                        ensure!(*expected == value, "Lucide geometry differs from nodes");
                    } else {
                        aliases.insert(
                            id.into(),
                            matching
                                .get(&serde_json::to_vec(&value)?)
                                .context("Unknown Lucide alias geometry")?
                                .clone(),
                        );
                    }
                }
            }
            tags = serde_json::from_slice::<BTreeMap<String, Value>>(
                members
                    .get("package/tags.json")
                    .context("Missing Lucide tags")?,
            )?;
        } else {
            let fragments: BTreeMap<String, String> = serde_json::from_slice(
                members
                    .get("package/dist/icons.json")
                    .context("Missing Feather fragments")?,
            )?;
            for (name, fragment) in fragments {
                let svg = format!("<svg>{fragment}</svg>");
                let doc = roxmltree::Document::parse(&svg)?;
                icons.insert(name, shapes(doc.root_element())?);
            }
            tags = BTreeMap::new();
        }
        let data = serde_json::to_vec(&Data {
            schema: 1,
            id: pack,
            version,
            view_box: [0.0, 0.0, 24.0, 24.0],
            stroke_width: 2.0,
            icons: &icons,
            aliases: &aliases,
            tags: &tags,
            font: None,
        })?;
        let folder = destination.join(pack);
        fs::create_dir_all(&folder)?;
        fs::write(folder.join("LICENSE"), legal)?;
        fs::write(
            if pack == "lucide" {
                folder.join("icons.json")
            } else {
                directory.join("feather-4.29.2.icons.json")
            },
            &data,
        )?;
        descriptors.push(json!({"id":pack,"version":version,"source_url":format!("https://registry.npmjs.org/{package}/-/{package}-{version}.tgz"),"source_sha256":expected,"source_integrity":integrity,"data_sha256":util::digest(&data),"license_sha256":util::digest(legal),"license":license,"icon_count":icons.len(),"alias_count":aliases.len(),"bundled":pack=="lucide"}));
    }
    fs::create_dir_all(destination)?;
    util::publish(
        &destination.join("catalogue.json"),
        &util::encode(&descriptors)?,
        true,
    )?;
    Ok(descriptors)
}

/// Parse only the pinned package's data grammar; never evaluate package code.
pub fn catalogue_literal(source: &str) -> Result<Value> {
    let prefix = "const n = ";
    ensure!(
        source.matches(prefix).count() == 1,
        "Unexpected pinned catalogue declaration"
    );
    let text = source.split_once(prefix).unwrap().1;
    struct Reader<'a> {
        text: &'a [u8],
        cursor: usize,
        nodes: usize,
    }
    impl Reader<'_> {
        fn space(&mut self) {
            while self
                .text
                .get(self.cursor)
                .is_some_and(u8::is_ascii_whitespace)
            {
                self.cursor += 1;
            }
        }
        fn value(&mut self, depth: usize) -> Result<Value> {
            self.space();
            self.nodes += 1;
            ensure!(
                depth < 128 && self.nodes <= 100_000,
                "Catalogue exceeds depth/node budget"
            );
            let first = *self
                .text
                .get(self.cursor)
                .context("Truncated catalogue data")?;
            if first == b'[' || first == b'{' {
                self.cursor += 1;
                self.space();
                let object = first == b'{';
                let close = if object { b'}' } else { b']' };
                let mut values = Vec::new();
                let mut fields = serde_json::Map::new();
                while self.text.get(self.cursor) != Some(&close) {
                    if object {
                        let start = self.cursor;
                        ensure!(
                            self.text
                                .get(start)
                                .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_'),
                            "Unexpected catalogue property"
                        );
                        self.cursor += 1;
                        while self
                            .text
                            .get(self.cursor)
                            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
                        {
                            self.cursor += 1;
                        }
                        let key = std::str::from_utf8(&self.text[start..self.cursor])?.to_owned();
                        self.space();
                        ensure!(
                            self.text.get(self.cursor) == Some(&b':'),
                            "Missing catalogue property separator"
                        );
                        self.cursor += 1;
                        let value = self.value(depth + 1)?;
                        ensure!(
                            fields.insert(key, value).is_none(),
                            "Duplicate catalogue property"
                        );
                    } else {
                        values.push(self.value(depth + 1)?);
                    }
                    self.space();
                    if self.text.get(self.cursor) != Some(&b',') {
                        break;
                    }
                    self.cursor += 1;
                    self.space();
                }
                ensure!(
                    self.text.get(self.cursor) == Some(&close),
                    "Unclosed catalogue data"
                );
                self.cursor += 1;
                return Ok(if object {
                    Value::Object(fields)
                } else {
                    Value::Array(values)
                });
            }
            if first == b'"' {
                let start = self.cursor;
                self.cursor += 1;
                let mut escaped = false;
                while let Some(byte) = self.text.get(self.cursor).copied() {
                    self.cursor += 1;
                    if byte == b'"' && !escaped {
                        return Ok(serde_json::from_slice(&self.text[start..self.cursor])?);
                    }
                    if byte == b'\\' && !escaped {
                        escaped = true;
                    } else {
                        escaped = false;
                    }
                }
                anyhow::bail!("Unclosed catalogue string");
            }
            if first.is_ascii_digit() {
                let start = self.cursor;
                while self
                    .text
                    .get(self.cursor)
                    .is_some_and(|b| b.is_ascii_digit() || b".eE+-".contains(b))
                {
                    self.cursor += 1;
                }
                return Ok(serde_json::from_slice(&self.text[start..self.cursor])?);
            }
            let start = self.cursor;
            ensure!(
                [b'a', b'e'].contains(&first) && self.text.get(start + 1) == Some(&b'.'),
                "Executable expressions are not catalogue data"
            );
            self.cursor += 2;
            let begin = self.cursor;
            while self
                .text
                .get(self.cursor)
                .is_some_and(|b| b.is_ascii_uppercase() || *b == b'_')
            {
                self.cursor += 1;
            }
            ensure!(self.cursor > begin, "Invalid catalogue enum member");
            Ok(json!(std::str::from_utf8(&self.text[start..self.cursor])?))
        }
    }
    let mut reader = Reader {
        text: text.as_bytes(),
        cursor: 0,
        nodes: 0,
    };
    let result = reader.value(0)?;
    ensure!(
        std::str::from_utf8(&reader.text[reader.cursor..])?
            .trim_start()
            .starts_with(";\nexport {"),
        "Unexpected catalogue suffix"
    );
    Ok(result)
}

pub fn phosphor(directory: &Path, output: &Path) -> Result<(Value, Value)> {
    let bytes = checked(&directory.join("phosphor-core-2.1.1.tgz"), PHOSPHOR_SHA)?;
    ensure!(
        sri(&bytes) == PHOSPHOR_SRI,
        "Phosphor source integrity mismatch"
    );
    let members = archive(&bytes)?;
    let legal = members
        .get("package/LICENSE")
        .context("Missing Phosphor license")?;
    let source = members
        .get("package/dist/index.mjs")
        .context("Missing Phosphor catalogue")?;
    let entries = catalogue_literal(std::str::from_utf8(source)?)?;
    let mut aliases = BTreeMap::new();
    let mut tags = BTreeMap::new();
    let mut icons = BTreeMap::new();
    for entry in entries.as_array().context("Invalid Phosphor catalogue")? {
        let name = entry["name"].as_str().context("Missing Phosphor name")?;
        ensure!(
            tags.insert(name.to_owned(), entry["tags"].clone())
                .is_none(),
            "Duplicate Phosphor name"
        );
        for alias in std::iter::once(&entry["pascal_name"]).chain(
            entry["alias"]
                .as_object()
                .into_iter()
                .flat_map(|v| v.values()),
        ) {
            let alias = alias.as_str().context("Invalid Phosphor alias")?;
            if alias != name {
                ensure!(
                    aliases.insert(alias.to_owned(), name.to_owned()).is_none(),
                    "Duplicate Phosphor alias"
                );
            }
        }
    }
    for (name, bytes) in &members {
        if name.starts_with("package/assets/regular/") && name.ends_with(".svg") {
            let name = Path::new(name).file_stem().unwrap().to_str().unwrap();
            let doc = roxmltree::Document::parse(std::str::from_utf8(bytes)?)?;
            let root = doc.root_element();
            let attrs = root
                .attributes()
                .map(|a| (a.name(), a.value()))
                .collect::<BTreeMap<_, _>>();
            ensure!(
                attrs == [("viewBox", "0 0 256 256"), ("fill", "currentColor")].into(),
                "Unexpected Phosphor viewport/paint"
            );
            let children = root
                .children()
                .filter(|n| n.is_element())
                .collect::<Vec<_>>();
            ensure!(
                children.len() == 1
                    && children[0].tag_name().name() == "path"
                    && children[0].attributes().len() == 1
                    && children[0].attribute("d").is_some(),
                "Unexpected Phosphor path"
            );
            ensure!(icons.insert(name.into(),json!([["path",{"d":children[0].attribute("d").unwrap(),"fill":"currentColor"}]])).is_none(),"Duplicate Phosphor geometry");
        }
    }
    ensure!(
        icons.keys().eq(tags.keys()) && aliases.keys().all(|p| !icons.contains_key(p)),
        "Phosphor catalogue and geometry differ"
    );
    #[derive(Serialize)]
    struct Index<'a> {
        tags: &'a BTreeMap<String, Value>,
        aliases: &'a BTreeMap<String, String>,
    }
    let index = serde_json::to_vec(&Index {
        tags: &tags,
        aliases: &aliases,
    })?;
    let data = serde_json::to_vec(&Data {
        schema: 2,
        id: "phosphor",
        version: "2.1.1",
        view_box: [0.0, 0.0, 256.0, 256.0],
        stroke_width: 0.0,
        icons: &icons,
        aliases: &aliases,
        tags: &tags,
        font: None,
    })?;
    let folder = output.join("phosphor");
    fs::create_dir_all(&folder)?;
    fs::write(folder.join("LICENSE"), legal)?;
    fs::write(folder.join("index.json"), &index)?;
    fs::write(directory.join("phosphor.icons.json"), &data)?;
    Ok((
        json!({"id":"phosphor","version":"2.1.1","source_url":"https://registry.npmjs.org/@phosphor-icons/core/-/core-2.1.1.tgz","source_sha256":PHOSPHOR_SHA,"source_integrity":PHOSPHOR_SRI,"data_sha256":util::digest(&data),"license_sha256":util::digest(legal),"license":"MIT","icon_count":icons.len(),"alias_count":aliases.len(),"bundled":false,"source_format":"phosphor-tgz","index_sha256":util::digest(&index),"copyright":"Copyright (c) 2023 Phosphor Icons"}),
        json!({"catalogue_member":"package/dist/index.mjs","catalogue_sha256":util::digest(source),"normalization":"Original regular paths; canonical names, PascalCase and deprecated aliases, tags. Package JavaScript is not executed."}),
    ))
}

pub fn material(directory: &Path, output: &Path, style: &str) -> Result<(Value, Value)> {
    let expected = FONTS
        .iter()
        .find(|(s, _)| *s == style)
        .context("Unknown Material style")?
        .1;
    let id = format!("Material Symbols {style}");
    let filename = format!("MaterialSymbols{style}[FILL,GRAD,opsz,wght]");
    let font = checked(&directory.join(format!("{filename}.ttf")), expected)?;
    let index = checked(
        &directory.join(format!("{filename}.codepoints")),
        CODEPOINT_SHA,
    )?;
    let legal = checked(&directory.join("Material-LICENSE"), LICENSE_SHA)?;
    let mut names = BTreeMap::new();
    for line in std::str::from_utf8(&index)?.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        ensure!(fields.len() == 2, "Invalid Material codepoint entry");
        let name = fields[0];
        let number = u32::from_str_radix(fields[1], 16)?;
        ensure!(
            !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                && number > 0
                && number <= 0x10ffff
                && !(0xd800..=0xdfff).contains(&number)
                && names.insert(name.into(), number).is_none(),
            "Invalid Material codepoint entry"
        );
    }
    let icons = names
        .keys()
        .map(|k: &String| (k.clone(), json!([])))
        .collect::<BTreeMap<_, _>>();
    let data = serde_json::to_vec(&Data {
        schema: 2,
        id: &id,
        version: COMMIT,
        view_box: [0.0, -960.0, 960.0, 960.0],
        stroke_width: 0.0,
        icons: &icons,
        aliases: &BTreeMap::new(),
        tags: &BTreeMap::new(),
        font: Some(&names),
    })?;
    let folder = output.join(&id);
    fs::create_dir_all(&folder)?;
    fs::write(folder.join("LICENSE"), &legal)?;
    fs::write(folder.join("codepoints.txt"), &index)?;
    fs::write(
        directory.join(format!("material-{}.icons.json", style.to_lowercase())),
        &data,
    )?;
    let encode = |s: &str| {
        s.bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect::<String>()
    };
    let prefix =
        format!("https://raw.githubusercontent.com/google/material-design-icons/{COMMIT}/");
    Ok((
        json!({"id":id,"version":COMMIT,"source_url":format!("{prefix}variablefont/{}",encode(&format!("{filename}.ttf"))),"source_sha256":util::digest(&font),"source_integrity":sri(&font),"data_sha256":util::digest(data),"license_sha256":util::digest(legal),"license":"Apache-2.0","icon_count":names.len(),"alias_count":0,"bundled":false,"source_format":"ttf","index_sha256":util::digest(index),"index_source_url":format!("{prefix}variablefont/{}",encode(&format!("{filename}.codepoints"))),"license_source_url":format!("{prefix}LICENSE"),"copyright":"Copyright 2026 Google LLC.  All Rights Reserved."}),
        json!({"commit":COMMIT,"font_bytes":font.len(),"font_name_copyright_id":0,"normalization":"Codepoints unchanged. Font downloaded/installed unchanged. Glyph outlines use FILL=0, GRAD=0, opsz=24 and wght=100..700; default 400."}),
    ))
}
