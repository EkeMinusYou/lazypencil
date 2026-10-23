//! Pinned vector icon packs, explicit installation, and snapshot-owned resolution.

use crate::{
    Error, Result,
    document::hash,
    store::{atomic_replace, create_private_directory},
};
use fs2::FileExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

#[cfg(feature = "rendering")]
mod drawing;
mod optional;
#[cfg(feature = "rendering")]
pub use drawing::shape;

const MAX_DATA: usize = 4 * 1024 * 1024;
const MAX_ARCHIVE: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub id: String,
    pub version: String,
    pub source_url: String,
    pub source_sha256: String,
    pub source_integrity: String,
    pub data_sha256: String,
    pub license_sha256: String,
    pub license: String,
    pub icon_count: usize,
    pub alias_count: usize,
    pub bundled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copyright: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Data {
    schema: u32,
    id: String,
    version: String,
    view_box: [f32; 4],
    stroke_width: f32,
    icons: BTreeMap<String, Vec<Primitive>>,
    aliases: BTreeMap<String, String>,
    tags: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    font: Option<BTreeMap<String, u32>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Primitive(pub String, pub BTreeMap<String, String>);
#[derive(Debug)]
pub struct Pack {
    descriptor: Descriptor,
    data: Data,
    captured_data: Option<Arc<[u8]>>,
    font_bytes: Option<Arc<[u8]>>,
    #[cfg(feature = "rendering")]
    font_face: OnceLock<Result<skia_safe::Typeface>>,
    #[cfg(feature = "rendering")]
    font_variants: std::sync::Mutex<std::collections::VecDeque<(u32, skia_safe::Typeface)>>,
}
impl Pack {
    fn tags(&self, name: &str) -> Option<&Vec<String>> {
        self.data
            .tags
            .get(self.data.aliases.get(name).map_or(name, String::as_str))
    }
}
#[derive(Clone, Debug)]
pub struct ResolvedIcon {
    pub pack: Arc<Pack>,
    pub name: String,
    pub canonical: String,
    pub weight: Option<f32>,
}
impl ResolvedIcon {
    pub fn metadata(&self) -> Value {
        let mut value = json!({"pack":self.pack.descriptor.id,"version":self.pack.descriptor.version,"name":self.name,"canonical_name":self.canonical,"hash":self.pack.descriptor.data_sha256,"license":self.pack.descriptor.license,"view_box":self.pack.data.view_box});
        if let Some(weight) = self.weight {
            value["weight"] = json!(weight);
            value["axes"] = json!({"FILL":0,"GRAD":0,"opsz":24,"wght":weight});
            value["font_hash"] = json!(self.pack.descriptor.source_sha256);
        }
        value
    }
}

pub fn catalogue() -> &'static [Descriptor] {
    static CATALOGUE: OnceLock<Vec<Descriptor>> = OnceLock::new();
    CATALOGUE.get_or_init(|| {
        serde_json::from_str(include_str!("../../resources/icons/catalogue.json"))
            .expect("generated icon catalogue")
    })
}
fn descriptor(id: &str) -> Result<&'static Descriptor> {
    catalogue().iter().find(|pack| pack.id == id).ok_or_else(|| Error::new("UNSUPPORTED_FEATURE", format!("Icon pack {id:?} is not implemented")).details(json!({"library":id,"implemented_packs":catalogue().iter().map(|pack| &pack.id).collect::<Vec<_>>()})))
}
pub fn license(id: &str) -> Result<Value> {
    let descriptor = descriptor(id)?;
    let text = match id {
        "lucide" => include_str!("../../resources/icons/lucide/LICENSE"),
        "feather" => include_str!("../../resources/icons/feather/LICENSE"),
        _ => optional::license(id)?,
    };
    verify(text.as_bytes(), &descriptor.license_sha256)?;
    Ok(
        json!({"pack":id,"version":descriptor.version,"license":descriptor.license,"source":descriptor.source_url,"license_source":descriptor.license_source_url,"copyright":descriptor.copyright,"hash":descriptor.license_sha256,"text":text}),
    )
}
fn directory(data: &Path, pack: &Descriptor) -> PathBuf {
    data.join("icons").join(&pack.id).join(&pack.version)
}
fn setup_command(id: &str) -> String {
    let argument = if id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        id.to_string()
    } else {
        format!("'{}'", id.replace('\'', "'\\''"))
    };
    format!("lazypencil icons setup {argument}")
}
fn integrity(message: impl Into<String>) -> Error {
    Error::new("ICON_INTEGRITY", message)
}
fn verify(bytes: &[u8], expected: &str) -> Result<()> {
    if hash(bytes) != expected {
        return Err(integrity(
            "Icon pack contents differ from the pinned upstream data",
        ));
    }
    Ok(())
}
fn parse(bytes: &[u8], pack: &Descriptor) -> Result<Arc<Pack>> {
    verify(bytes, &pack.data_sha256)?;
    let data: Data = serde_json::from_slice(bytes)
        .map_err(|error| integrity(format!("Invalid icon pack: {error}")))?;
    if !matches!(data.schema, 1 | 2)
        || data.id != pack.id
        || data.version != pack.version
        || data.icons.len() != pack.icon_count
        || data.aliases.len() != pack.alias_count
        || data.font.is_some() != (pack.source_format.as_deref() == Some("ttf"))
        || data
            .font
            .as_ref()
            .is_some_and(|font| font.len() != data.icons.len() || font.keys().ne(data.icons.keys()))
    {
        return Err(integrity("Icon pack metadata differs from its pin"));
    }
    Ok(Arc::new(Pack {
        descriptor: pack.clone(),
        data,
        captured_data: None,
        font_bytes: None,
        #[cfg(feature = "rendering")]
        font_face: OnceLock::new(),
        #[cfg(feature = "rendering")]
        font_variants: Default::default(),
    }))
}
fn bundled() -> Result<Arc<Pack>> {
    static PACK: OnceLock<Result<Arc<Pack>>> = OnceLock::new();
    PACK.get_or_init(|| {
        parse(
            include_bytes!("../../resources/icons/lucide/icons.json"),
            descriptor("lucide")?,
        )
    })
    .clone()
}
fn read(path: &Path, max: usize) -> Result<Arc<[u8]>> {
    let mut bytes = vec![];
    crate::store::open_regular_file(path)?
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| Error::io(path, error))?;
    if bytes.len() > max {
        return Err(integrity("Icon input exceeds its size limit"));
    }
    Ok(bytes.into())
}
pub(crate) type CapturedFiles = Vec<(PathBuf, Arc<[u8]>)>;
fn installed(data: &Path, descriptor: &Descriptor) -> Result<(Arc<Pack>, CapturedFiles)> {
    let directory = directory(data, descriptor);
    if !directory.exists() {
        return Err(Error::new("ASSET_NOT_FOUND", format!("Icon pack {:?} is not installed; run {}",descriptor.id,setup_command(&descriptor.id))).details(json!({"pack":descriptor.id,"version":descriptor.version,"setup_command":setup_command(&descriptor.id)})));
    }
    type Cache = std::sync::Mutex<BTreeMap<String, std::sync::Weak<Pack>>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let cached = cache
        .lock()
        .map_err(|_| integrity("Icon cache is unavailable"))?
        .get(&descriptor.id)
        .and_then(std::sync::Weak::upgrade)
        .filter(|pack| pack.descriptor == *descriptor);
    let mut files = vec![];
    let mut required = vec![
        ("manifest.json", 16384),
        ("LICENSE", 16384),
        ("icons.json", MAX_DATA),
    ];
    if descriptor.source_format.as_deref() == Some("ttf") {
        required.push(("font.ttf", MAX_ARCHIVE));
    }
    for (name, max) in required {
        let path = crate::paths::canonical(directory.join(name))
            .map_err(|error| integrity(format!("Cannot read installed icon file: {error}")))?;
        // Hash current files before reusing immutable captured bytes. Large
        // fonts must not be duplicated just to validate another snapshot.
        let captured = cached.as_ref().and_then(|pack| match name {
            "icons.json" => pack.captured_data.as_ref(),
            "font.ttf" => pack.font_bytes.as_ref(),
            _ => None,
        });
        let bytes = if let Some(captured) = captured {
            let expected = if name == "font.ttf" {
                &descriptor.source_sha256
            } else {
                &descriptor.data_sha256
            };
            let current = crate::store::content::file_hash(&path, max, None)
                .map_err(|error| integrity(format!("Cannot read installed icon file: {error}")))?;
            if &current != expected {
                return Err(integrity(
                    "Icon pack contents differ from the pinned upstream data",
                ));
            }
            captured.clone()
        } else {
            read(&path, max)
                .map_err(|error| integrity(format!("Cannot read installed icon file: {error}")))?
        };
        files.push((path, bytes));
    }
    let manifest: Descriptor = serde_json::from_slice(&files[0].1)
        .map_err(|_| integrity("Invalid installed icon manifest"))?;
    if &manifest != descriptor {
        return Err(integrity("Installed icon manifest differs from its pin"));
    }
    verify(&files[1].1, &descriptor.license_sha256)?;
    verify(&files[2].1, &descriptor.data_sha256)?;
    let font = descriptor.source_format.as_deref() == Some("ttf");
    if font {
        verify(&files[3].1, &descriptor.source_sha256)?;
    }
    // Revalidate every file before sharing immutable data or native font caches.
    let pack = if let Some(pack) = cached {
        pack
    } else {
        let mut pack = parse(&files[2].1, descriptor)?;
        let owned = Arc::get_mut(&mut pack).expect("Newly parsed icon pack");
        owned.captured_data = Some(files[2].1.clone());
        if font {
            owned.font_bytes = Some(files[3].1.clone());
        }
        let mut cache = cache
            .lock()
            .map_err(|_| integrity("Icon cache is unavailable"))?;
        if let Some(existing) = cache
            .get(&descriptor.id)
            .and_then(std::sync::Weak::upgrade)
            .filter(|pack| pack.descriptor == *descriptor)
        {
            existing
        } else {
            cache.insert(descriptor.id.clone(), Arc::downgrade(&pack));
            pack
        }
    };
    files[2].1 = pack
        .captured_data
        .as_ref()
        .expect("Installed icon data")
        .clone();
    if font {
        files[3].1 = pack
            .font_bytes
            .as_ref()
            .expect("Installed icon font")
            .clone();
    }
    Ok((pack, files))
}
fn load(data: Option<&Path>, id: &str) -> Result<(Arc<Pack>, CapturedFiles)> {
    let descriptor = descriptor(id)?;
    if descriptor.bundled {
        return bundled().map(|pack| (pack, vec![]));
    }
    installed(
        data.ok_or_else(|| {
            Error::new(
                "ASSET_NOT_FOUND",
                format!(
                    "Icon pack {id:?} requires a configured data directory; run {}",
                    setup_command(id)
                ),
            )
        })?,
        descriptor,
    )
}

#[derive(Default)]
pub(crate) struct Registry {
    packs: BTreeMap<String, Result<Arc<Pack>>>,
    missing: Vec<PathBuf>,
}
impl Registry {
    pub fn prepare(libraries: BTreeSet<String>, data: Option<&Path>) -> (Self, CapturedFiles) {
        let mut registry = Self::default();
        let mut files = vec![];
        for id in libraries {
            let pack = load(data, &id).map(|(pack, captured)| {
                files.extend(captured);
                pack
            });
            if pack
                .as_ref()
                .is_err_and(|error| error.code == "ASSET_NOT_FOUND")
                && let (Some(data), Ok(descriptor)) = (data, descriptor(&id))
            {
                registry.missing.push(directory(data, descriptor));
            }
            registry.packs.insert(id, pack);
        }
        (registry, files)
    }
    pub fn contains(&self, library: &str) -> bool {
        self.packs.contains_key(library)
    }
    pub fn extend(&mut self, other: Self) {
        self.packs.extend(other.packs);
        self.missing.extend(other.missing);
    }
    pub fn verify_unchanged(&self) -> Result<()> {
        if self.missing.iter().any(|path| path.exists()) {
            return Err(Error::new(
                "REVISION_CONFLICT",
                "An icon pack became available; recalculate the project snapshot",
            ));
        }
        Ok(())
    }
    pub fn resolve(
        &self,
        properties: &serde_json::Map<String, Value>,
    ) -> Result<Arc<ResolvedIcon>> {
        let library = properties
            .get("library")
            .map(|value| {
                value.as_str().ok_or_else(|| {
                    Error::new("INVALID_DOCUMENT", "Icon library must resolve to a string")
                })
            })
            .transpose()?
            .unwrap_or("lucide");
        let name = properties
            .get("icon")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| Error::new("INVALID_DOCUMENT", "An icon node requires an icon name"))?;
        let pack = match self.packs.get(library) {
            Some(result) => result.clone()?,
            None if library == "lucide" => bundled()?,
            _ => {
                return Err(Error::new(
                    "ASSET_NOT_FOUND",
                    format!("Icon pack {library:?} was not captured in this snapshot"),
                ));
            }
        };
        let mut selected_weight = None;
        if let Some(weight) = properties.get("weight") {
            if !weight
                .as_f64()
                .is_some_and(|weight| weight.is_finite() && (100.0..=700.0).contains(&weight))
            {
                return Err(Error::new(
                    "INVALID_DOCUMENT",
                    "Icon weight must resolve to a number within 100..700",
                ));
            }
            if pack.data.font.is_none() {
                return Err(Error::new(
                    "UNSUPPORTED_FEATURE",
                    format!(
                        "Icon pack {library:?} does not provide variable weights; omit weight instead of synthesizing a weight"
                    ),
                ));
            }
            selected_weight = weight.as_f64().map(|value| value as f32);
        }
        let canonical = pack
            .data
            .aliases
            .get(name)
            .map(String::as_str)
            .unwrap_or(name);
        if !pack.data.icons.contains_key(canonical) {
            return Err(Error::new("ASSET_NOT_FOUND",format!("Icon {name:?} is unavailable in {library} {}",pack.descriptor.version)).details(json!({"pack":library,"version":pack.descriptor.version,"name":name,"lookup":"lazypencil icons list --pack PACK --name QUERY"})));
        }
        Ok(Arc::new(ResolvedIcon {
            name: name.into(),
            canonical: canonical.into(),
            pack,
            weight: selected_weight
                .or_else(|| (library.starts_with("Material Symbols ")).then_some(400.0)),
        }))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListRequest {
    pub pack: Option<String>,
    pub name: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    pub cursor: Option<String>,
}
fn default_limit() -> usize {
    100
}
pub fn list(data: &Path, request: &ListRequest) -> Result<Value> {
    if !(1..=500).contains(&request.limit)
        || request.name.as_ref().is_some_and(|name| name.len() > 256)
    {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Icon listing needs a limit within 1..500 and a query up to 256 bytes",
        ));
    }
    let selected = request.pack.as_deref().unwrap_or("lucide");
    let descriptor = descriptor(selected)?;
    let loaded = load(Some(data), selected);
    let availability = loaded
        .as_ref()
        .map(|_| json!({"installed":true}))
        .unwrap_or_else(|error| json!({"installed":false,"diagnostic":error}));
    let packs = catalogue().iter().map(|pack| {
        let status = if pack.id == selected { availability.clone() } else { load(Some(data),&pack.id).map(|_|json!({"installed":true})).unwrap_or_else(|error|json!({"installed":false,"diagnostic":error})) };
        json!({"id":pack.id,"version":pack.version,"hash":pack.data_sha256,"license":pack.license,"source":pack.source_url,"bundled":pack.bundled,"icon_count":pack.icon_count,"alias_count":pack.alias_count,"status":status})
    }).collect::<Vec<_>>();
    let (pack, _) = match loaded {
        Ok(pack) => pack,
        Err(error) if error.code == "ASSET_NOT_FOUND" => {
            return Ok(
                json!({"pack":selected,"packs":packs,"icons":[],"next_cursor":null,"diagnostic":error}),
            );
        }
        Err(error) => return Err(error),
    };
    let query = request.name.as_deref().unwrap_or_default().to_lowercase();
    let fingerprint = hash(
        json!({"pack":selected,"hash":descriptor.data_sha256,"query":query})
            .to_string()
            .as_bytes(),
    );
    let offset = request
        .cursor
        .as_ref()
        .map(|cursor| {
            let (bound, offset) = cursor
                .split_once(':')
                .ok_or_else(|| Error::new("INVALID_ARGUMENT", "Invalid icon cursor"))?;
            if bound != fingerprint {
                return Err(Error::new(
                    "INVALID_ARGUMENT",
                    "Icon cursor belongs to another query or pack version",
                ));
            }
            offset
                .parse::<usize>()
                .map_err(|_| Error::new("INVALID_ARGUMENT", "Invalid icon cursor offset"))
        })
        .transpose()?
        .unwrap_or(0);
    let names = pack
        .data
        .icons
        .keys()
        .chain(pack.data.aliases.keys())
        .filter(|name| {
            name.to_lowercase().contains(&query)
                || pack
                    .tags(name)
                    .is_some_and(|tags| tags.iter().any(|tag| tag.to_lowercase().contains(&query)))
        })
        .collect::<BTreeSet<_>>();
    if offset > names.len() {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Icon cursor is past the result",
        ));
    }
    let values = names.iter().skip(offset).take(request.limit).map(|name| json!({"pack":selected,"name":name,"alias_of":pack.data.aliases.get(*name),"weights":[],"variable_weight":pack.data.font.is_some(),"weight_range":pack.data.font.as_ref().map(|_| [100,700]),"default_weight":pack.data.font.as_ref().map(|_|400),"view_box":pack.data.view_box,"tags":pack.tags(name),"version":descriptor.version,"hash":descriptor.data_sha256,"license":descriptor.license})).collect::<Vec<_>>();
    let next = offset + values.len();
    Ok(
        json!({"pack":selected,"packs":packs,"icons":values,"total":names.len(),"next_cursor":(next<names.len()).then(||format!("{fingerprint}:{next}"))}),
    )
}

pub fn setup(data: &Path, id: &str, archive: Option<&Path>) -> Result<Value> {
    let descriptor = descriptor(id)?;
    if descriptor.bundled {
        bundled()?;
        return Ok(json!({"pack":id,"version":descriptor.version,"bundled":true,"reused":true}));
    }
    let parent = data.join("icons").join(id);
    create_private_directory(&parent)?;
    let lock_path = parent.join("setup.lock");
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options
        .open(&lock_path)
        .map_err(|error| Error::io(&lock_path, error))?;
    let cancel = crate::cancel::token();
    let start = Instant::now();
    loop {
        crate::cancel::check(Some(&cancel))?;
        match lock.try_lock_exclusive() {
            Ok(()) => break,
            Err(error)
                if crate::store::lock_contended(&error)
                    && start.elapsed() < Duration::from_secs(60) =>
            {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(error) => return Err(Error::io(&lock_path, error)),
        }
    }
    if directory(data, descriptor).exists() {
        installed(data, descriptor)?;
        return Ok(
            json!({"pack":id,"version":descriptor.version,"reused":true,"path":directory(data,descriptor)}),
        );
    }
    let archive = if let Some(path) = archive {
        read(path, MAX_ARCHIVE)?.to_vec()
    } else {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .build()
            .into();
        let mut response = agent.get(&descriptor.source_url).call().map_err(|error| {
            Error::new(
                "ICON_UNAVAILABLE",
                format!("Cannot retrieve pinned icon archive: {error}"),
            )
        })?;
        let mut bytes = vec![];
        response
            .body_mut()
            .as_reader()
            .take(MAX_ARCHIVE as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| Error::new("IO_ERROR", error.to_string()))?;
        if bytes.len() > MAX_ARCHIVE {
            return Err(integrity("Icon archive exceeds 16 MiB"));
        }
        bytes
    };
    crate::cancel::check(Some(&cancel))?;
    verify(&archive, &descriptor.source_sha256)?;
    let (bytes, license) = match descriptor.source_format.as_deref() {
        None => convert_feather(&archive, descriptor)?,
        Some("phosphor-tgz") => optional::phosphor(&archive, descriptor)?,
        Some("ttf") => optional::material(descriptor)?,
        _ => return Err(integrity("Unknown pinned icon source format")),
    };
    crate::cancel::check(Some(&cancel))?;
    let staging = tempfile::tempdir_in(&parent).map_err(|error| Error::io(&parent, error))?;
    atomic_replace(&staging.path().join("icons.json"), &bytes)?;
    atomic_replace(&staging.path().join("LICENSE"), &license)?;
    if descriptor.source_format.as_deref() == Some("ttf") {
        atomic_replace(&staging.path().join("font.ttf"), &archive)?;
    }
    atomic_replace(
        &staging.path().join("manifest.json"),
        &serde_json::to_vec(descriptor).map_err(|error| integrity(error.to_string()))?,
    )?;
    crate::cancel::check(Some(&cancel))?;
    fs::rename(staging.path(), directory(data, descriptor))
        .map_err(|error| Error::io(&parent, error))?;
    crate::store::sync_directory(&parent)?;
    installed(data, descriptor)?;
    Ok(
        json!({"pack":id,"version":descriptor.version,"path":directory(data,descriptor),"hash":descriptor.data_sha256,"license":descriptor.license,"reused":false}),
    )
}

fn convert_feather(bytes: &[u8], descriptor: &Descriptor) -> Result<(Vec<u8>, Vec<u8>)> {
    let reader = flate2::read::GzDecoder::new(bytes).take(2 * 1024 * 1024);
    let mut archive = tar::Archive::new(reader);
    let mut content = None;
    let mut license = None;
    for entry in archive
        .entries()
        .map_err(|error| integrity(error.to_string()))?
        .take(1024)
    {
        let mut entry = entry.map_err(|error| integrity(error.to_string()))?;
        let name = entry
            .path()
            .map_err(|error| integrity(error.to_string()))?
            .into_owned();
        if name == Path::new("package/dist/icons.json") || name == Path::new("package/LICENSE") {
            if !entry.header().entry_type().is_file() || entry.size() > MAX_DATA as u64 {
                return Err(integrity("Invalid pinned icon archive entry"));
            }
            let mut bytes = vec![];
            entry
                .read_to_end(&mut bytes)
                .map_err(|error| integrity(error.to_string()))?;
            let target = if name == Path::new("package/LICENSE") {
                &mut license
            } else {
                &mut content
            };
            if target.replace(bytes).is_some() {
                return Err(integrity("Duplicate icon archive entry"));
            }
        }
    }
    let license = license.ok_or_else(|| integrity("Icon archive lacks its license"))?;
    verify(&license, &descriptor.license_sha256)?;
    let fragments: BTreeMap<String, String> = serde_json::from_slice(
        &content.ok_or_else(|| integrity("Icon archive lacks its geometry"))?,
    )
    .map_err(|error| integrity(error.to_string()))?;
    let mut icons = BTreeMap::new();
    for (name, fragment) in fragments {
        let source = format!("<svg>{fragment}</svg>");
        let xml =
            roxmltree::Document::parse(&source).map_err(|error| integrity(error.to_string()))?;
        let shapes = xml
            .root_element()
            .children()
            .filter(|node| node.is_element())
            .map(|node| {
                if !matches!(
                    node.tag_name().name(),
                    "path" | "line" | "circle" | "ellipse" | "rect" | "polygon" | "polyline"
                ) || node.children().any(|node| node.is_element())
                {
                    return Err(integrity("Unexpected primitive in pinned icon"));
                }
                Ok(Primitive(
                    node.tag_name().name().into(),
                    node.attributes()
                        .map(|attribute| (attribute.name().into(), attribute.value().into()))
                        .collect(),
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        icons.insert(name, shapes);
    }
    let data = Data {
        schema: 1,
        id: descriptor.id.clone(),
        version: descriptor.version.clone(),
        view_box: [0.0, 0.0, 24.0, 24.0],
        stroke_width: 2.0,
        icons,
        aliases: BTreeMap::new(),
        tags: BTreeMap::new(),
        font: None,
    };
    let bytes = serde_json::to_vec(&data).map_err(|error| integrity(error.to_string()))?;
    parse(&bytes, descriptor)?;
    Ok((bytes, license))
}
