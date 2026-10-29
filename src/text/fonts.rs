use super::{DEFAULT_FONT_BYTES, DEFAULT_FONT_FAMILY};
use crate::{Error, Result, document::hash, project::font_files::FontFile};
use serde::{Deserialize, Serialize};
use skia_safe::{
    Data, FontMgr, FontStyle, Typeface,
    font_style::Slant,
    textlayout::{FontArguments, FontCollection, TypefaceFontProvider},
};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, OnceLock},
};

const MAX_FONT_BYTES: usize = 64 * 1024 * 1024;
const MAX_FACES: usize = 256;
const MAX_TOTAL_BYTES: usize = 128 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontAxis {
    pub tag: String,
    pub min: f32,
    pub default: f32,
    pub max: f32,
    pub value: f32,
    pub hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontInfo {
    pub family: String,
    pub source: String,
    pub hash: String,
    pub face_index: u32,
    pub face_hash: String,
    pub postscript_name: Option<String>,
    pub weight: i32,
    pub width: i32,
    pub slant: String,
    pub axes: Vec<FontAxis>,
    pub synthetic_bold: bool,
    pub synthetic_italic: bool,
}

impl FontInfo {
    fn instance(&self, face: &Typeface) -> Result<Self> {
        let mut info = self.clone();
        let style = face.font_style();
        info.weight = *style.weight();
        info.width = *style.width();
        info.slant = match style.slant() {
            Slant::Upright => "upright",
            Slant::Italic => "italic",
            _ => "oblique",
        }
        .into();
        info.postscript_name = face.post_script_name();
        info.synthetic_bold = face.is_synthetic_bold();
        info.synthetic_italic = face.is_synthetic_oblique();
        let positions = face.variation_design_position().unwrap_or_default();
        let axes = face.variation_design_parameters().unwrap_or_default();
        if axes.len() > 64 {
            return Err(invalid("A font has more than 64 variation axes"));
        }
        info.axes = axes
            .iter()
            .map(|axis| {
                let value = positions
                    .iter()
                    .find(|position| position.axis == axis.tag)
                    .map_or(axis.def, |position| position.value);
                FontAxis {
                    tag: String::from_utf8_lossy(&(*axis.tag).to_be_bytes()).into_owned(),
                    min: axis.min,
                    default: axis.def,
                    max: axis.max,
                    value,
                    hidden: axis.is_hidden(),
                }
            })
            .collect();
        if info.axes.iter().any(|axis| {
            ![axis.min, axis.default, axis.max, axis.value]
                .iter()
                .all(|value| value.is_finite())
                || axis.min > axis.default
                || axis.default > axis.max
        }) {
            return Err(invalid("A font has invalid variation axes"));
        }
        Ok(info)
    }
}

pub struct Fonts {
    provider: TypefaceFontProvider,
    pub(super) collection: FontCollection,
    system: Option<FontMgr>,
    #[cfg(target_os = "macos")]
    portable_color: Option<FontMgr>,
    families: BTreeMap<String, String>,
    explicit: BTreeSet<String>,
    registered: BTreeMap<u32, FontInfo>,
    system_requests: BTreeSet<(String, i32, i32, i32)>,
    fallbacks: Vec<String>,
    loaded_bytes: usize,
    default_family: String,
    locale: String,
    pending: Vec<PendingFace>,
    cancel: Option<tokio_util::sync::CancellationToken>,
    lock: Option<crate::font_lock::Manifest>,
}

struct PendingFace {
    source: FontFile,
    index: u32,
    names: Vec<String>,
}

pub(super) struct ResolvedFonts {
    pub families: Vec<String>,
    pub information: BTreeMap<u32, FontInfo>,
}

impl Fonts {
    pub fn new(files: &[PathBuf]) -> Result<Self> {
        if files.len() > MAX_FACES {
            return Err(invalid("Font inputs exceed 256 files"));
        }
        let sources = files
            .iter()
            .map(|path| FontFile::capture(&crate::paths::canonical(path)?, None))
            .collect::<Result<Vec<_>>>()?;
        Self::from_files(&sources, None)
    }

    pub fn from_sources(sources: &[(PathBuf, Arc<[u8]>)]) -> Result<Self> {
        if sources.len() > MAX_FACES
            || sources
                .iter()
                .try_fold(0usize, |total, (_, bytes)| total.checked_add(bytes.len()))
                .is_none_or(|total| total > MAX_TOTAL_BYTES)
        {
            return Err(invalid("Font inputs exceed 128 MiB or 256 files"));
        }
        let files = sources
            .iter()
            .map(|(path, bytes)| FontFile::from_bytes(path.clone(), bytes.clone()))
            .collect::<Vec<_>>();
        Self::from_files(&files, None)
    }

    pub(crate) fn from_files(
        sources: &[FontFile],
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<Self> {
        let total = sources
            .iter()
            .try_fold(0usize, |total, source| total.checked_add(source.length))
            .ok_or_else(|| invalid("Font input size overflow"))?;
        if total > MAX_TOTAL_BYTES || sources.len() > MAX_FACES {
            return Err(invalid("Font inputs exceed 128 MiB or 256 files"));
        }
        let mut fonts = Self {
            provider: TypefaceFontProvider::new(),
            collection: FontCollection::new(),
            system: None,
            #[cfg(target_os = "macos")]
            portable_color: None,
            families: BTreeMap::new(),
            explicit: BTreeSet::new(),
            registered: BTreeMap::new(),
            system_requests: BTreeSet::new(),
            fallbacks: vec![],
            loaded_bytes: 0,
            default_family: DEFAULT_FONT_FAMILY.into(),
            locale: "ja".into(),
            pending: Vec::new(),
            cancel: cancel.cloned(),
            lock: None,
        };
        let mut additional = Vec::new();
        let mut seen = BTreeSet::new();
        for source in sources {
            crate::cancel::check(cancel)?;
            if source.length > MAX_FONT_BYTES {
                return Err(invalid("A font file exceeds 64 MiB"));
            }
            for (index, names) in super::font_index::families(source)?.into_iter().enumerate() {
                if !seen.insert((source.revision.clone(), index)) {
                    continue;
                }
                if seen.len() > MAX_FACES {
                    return Err(invalid(
                        "Configured fonts exceed 256 faces including the default",
                    ));
                }
                let Some(names) = names else {
                    let bytes = source.read(cancel)?;
                    additional.extend(fonts.register_indices(
                        &bytes,
                        &source.path.display().to_string(),
                        None,
                        None,
                        Some(&[index as u32]),
                    )?);
                    continue;
                };
                let family = names[0].clone();
                for name in &names {
                    fonts.explicit.insert(name.to_lowercase());
                    fonts
                        .families
                        .entry(name.to_lowercase())
                        .or_insert_with(|| name.clone());
                }
                if !additional.contains(&family) {
                    additional.push(family);
                }
                fonts.pending.push(PendingFace {
                    source: source.clone(),
                    index: index as u32,
                    names,
                });
            }
            source.verify(cancel)?;
        }
        if !fonts
            .families
            .contains_key(&DEFAULT_FONT_FAMILY.to_lowercase())
        {
            if seen.len() >= MAX_FACES {
                return Err(invalid(
                    "Configured fonts exceed 256 faces including the default",
                ));
            }
            let face = fonts.bundled_face()?;
            fonts.register(
                DEFAULT_FONT_BYTES,
                "bundled",
                Some(DEFAULT_FONT_FAMILY),
                Some(&face),
            )?;
        }
        fonts.fallbacks.push(DEFAULT_FONT_FAMILY.into());
        fonts.fallbacks.extend(
            additional
                .into_iter()
                .filter(|family| family != DEFAULT_FONT_FAMILY),
        );
        fonts
            .collection
            .set_asset_font_manager(FontMgr::from(fonts.provider.clone()));
        fonts.collection.disable_font_fallback();
        Ok(fonts)
    }

    fn register(
        &mut self,
        bytes: &[u8],
        source: &str,
        alias: Option<&str>,
        shared_face: Option<&Typeface>,
    ) -> Result<Vec<String>> {
        self.register_indices(bytes, source, alias, shared_face, None)
    }

    fn register_indices(
        &mut self,
        bytes: &[u8],
        source: &str,
        alias: Option<&str>,
        shared_face: Option<&Typeface>,
        only: Option<&[u32]>,
    ) -> Result<Vec<String>> {
        if bytes.len() > MAX_FONT_BYTES {
            return Err(invalid("A font file exceeds 64 MiB"));
        }
        let count = super::container::count(bytes)?;
        if shared_face.is_some() && count != 1 {
            return Err(invalid("A shared bundled typeface needs exactly one face"));
        }
        // Only embedded, immutable program storage can retain its digest across
        // font collections. User-supplied bytes are hashed on every load.
        let digest = if std::ptr::eq(bytes, DEFAULT_FONT_BYTES) {
            static DIGEST: OnceLock<String> = OnceLock::new();
            DIGEST.get_or_init(|| hash(DEFAULT_FONT_BYTES)).clone()
        } else {
            hash(bytes)
        };
        let mut families = Vec::new();
        for index in 0..count {
            if only.is_some_and(|only| !only.contains(&index)) {
                continue;
            }
            if let Some(existing) = self
                .registered
                .values()
                .find(|info| info.hash == digest && info.face_index == index)
            {
                families.push(existing.family.clone());
                continue;
            }
            let bytes = super::container::extract(bytes, index)?;
            // A standalone font is borrowed without changing a byte. Collections
            // produce a new normalized face and require its independent digest.
            let face_hash = match &bytes {
                Cow::Borrowed(_) => digest.clone(),
                Cow::Owned(_) => hash(&bytes),
            };
            let size = bytes.len();
            let face = match shared_face {
                Some(face) => face.clone(),
                None => self
                    .file_face(&bytes)?
                    .ok_or_else(|| invalid(format!("Cannot load font {source}, face {index}")))?,
            };
            let family = alias
                .map(str::to_string)
                .unwrap_or_else(|| face.family_name());
            self.add(
                face,
                FontInfo {
                    family: family.clone(),
                    source: source.into(),
                    hash: digest.clone(),
                    face_index: index,
                    face_hash,
                    postscript_name: None,
                    weight: 0,
                    width: 0,
                    slant: String::new(),
                    axes: vec![],
                    synthetic_bold: false,
                    synthetic_italic: false,
                },
                size,
            )?;
            self.explicit.insert(family.to_lowercase());
            families.push(self.families[&family.to_lowercase()].clone());
        }
        Ok(families)
    }

    fn file_face(&mut self, bytes: &[u8]) -> Result<Option<Typeface>> {
        self.file_face_data(bytes, Data::new_copy(bytes))
    }

    fn bundled_face(&mut self) -> Result<Typeface> {
        static FACE: OnceLock<Result<Typeface>> = OnceLock::new();
        FACE.get_or_init(|| {
            // SAFETY: include_bytes! provides immutable program-lifetime storage.
            // The shared Data and Typeface may outlive every Fonts collection.
            let data = unsafe { Data::new_bytes(DEFAULT_FONT_BYTES) };
            self.file_face_data(DEFAULT_FONT_BYTES, data)?
                .ok_or_else(|| invalid("Cannot load bundled Noto Sans JP"))
        })
        .clone()
    }

    fn file_face_data(&mut self, bytes: &[u8], data: Data) -> Result<Option<Typeface>> {
        #[cfg(not(target_os = "macos"))]
        let _ = bytes;
        #[cfg(target_os = "macos")]
        if super::container::requires_freetype_color(bytes)? {
            if self.portable_color.is_none() {
                self.portable_color = Some(FontMgr::custom_empty().ok_or_else(|| {
                    Error::new(
                        "RENDER_ERROR",
                        "Skia's embedded FreeType font manager is unavailable",
                    )
                })?);
            }
            return Ok(self
                .portable_color
                .as_ref()
                .expect("initialized color manager")
                .new_from_data(data, Some(0)));
        }
        Ok(self.system().new_from_data(data, Some(0)))
    }

    fn system(&mut self) -> &FontMgr {
        // A cached bundled typeface does not need another OS font manager.
        // Keep each real system/file lookup fresh within its own Fonts context.
        self.system.get_or_insert_with(FontMgr::new)
    }

    fn add(&mut self, face: Typeface, info: FontInfo, size: usize) -> Result<()> {
        if self.registered.len() >= MAX_FACES {
            return Err(invalid("Configured fonts exceed 256 faces"));
        }
        let total = self
            .loaded_bytes
            .checked_add(size)
            .filter(|total| *total <= MAX_TOTAL_BYTES)
            .ok_or_else(|| invalid("Loaded font data exceeds 128 MiB"))?;
        if info.family.is_empty() || info.family.len() > 256 {
            return Err(invalid("Invalid font family name"));
        }
        let mut info = info.instance(&face)?;
        info.family = self
            .families
            .get(&info.family.to_lowercase())
            .cloned()
            .unwrap_or_else(|| info.family.clone());
        if let Some(lock) = &self.lock {
            lock.require_face(&info)?;
        }
        self.loaded_bytes = total;
        info.family = self
            .families
            .entry(info.family.to_lowercase())
            .or_insert_with(|| info.family.clone())
            .clone();
        self.provider
            .register_typeface(face.clone(), Some(info.family.as_str()));
        self.registered.insert(face.unique_id(), info);
        self.collection.clear_caches();
        Ok(())
    }

    fn ensure_style(&mut self, family: &str, style: FontStyle) -> Result<String> {
        let key = family.to_lowercase();
        while let Some(index) = self
            .pending
            .iter()
            .position(|face| face.names.iter().any(|name| name.to_lowercase() == key))
        {
            self.load_pending(index)?;
        }
        if self.explicit.contains(&key) {
            return Ok(self.families[&key].clone());
        }
        let request = (
            key.clone(),
            *style.weight(),
            *style.width(),
            style.slant() as i32,
        );
        if self.system_requests.contains(&request) {
            return Ok(self.families[&key].clone());
        }
        let actual = self
            .system()
            .family_names()
            .find(|name| name.to_lowercase() == key)
            .ok_or_else(|| {
                Error::new(
                    "ASSET_NOT_FOUND",
                    format!(
                        "Font family {family:?} is unavailable; add its font file or install it"
                    ),
                )
            })?;
        let face = self
            .system()
            .match_family_style(&actual, style)
            .ok_or_else(|| {
                Error::new(
                    "ASSET_NOT_FOUND",
                    format!("Cannot load font family {family:?}"),
                )
            })?;
        if self.registered.len() >= MAX_FACES {
            return Err(invalid("Font resolution exceeds 256 faces"));
        }
        let (bytes, index) = face.to_font_bytes().ok_or_else(|| {
            Error::new(
                "ASSET_NOT_FOUND",
                format!("Cannot capture the bytes of font family {family:?}"),
            )
        })?;
        if bytes.len() > MAX_FONT_BYTES {
            return Err(invalid("A system font exceeds 64 MiB"));
        }
        let normalized = super::container::extract(&bytes, index)?;
        let size = normalized.len();
        let frozen = self
            .file_face(&normalized)?
            .ok_or_else(|| invalid("Cannot load captured system font data"))?;
        self.add(
            frozen,
            FontInfo {
                family: actual.clone(),
                source: "system:font-data".into(),
                hash: hash(&bytes),
                face_index: index,
                face_hash: hash(&normalized),
                postscript_name: None,
                weight: 0,
                width: 0,
                slant: String::new(),
                axes: vec![],
                synthetic_bold: false,
                synthetic_italic: false,
            },
            size,
        )?;
        self.system_requests.insert(request);
        Ok(actual)
    }

    pub fn ensure_family(&mut self, family: &str) -> Result<FontInfo> {
        let family = self.ensure_style(family, FontStyle::normal())?;
        let face = self
            .provider
            .match_family_style(&family, FontStyle::normal())
            .ok_or_else(|| invalid("Registered family has no face"))?;
        Ok(self.registered[&face.unique_id()].clone())
    }

    pub fn fallback_families(&self) -> &[String] {
        &self.fallbacks
    }

    pub fn default_family(&self) -> &str {
        &self.default_family
    }
    pub fn locale(&self) -> &str {
        &self.locale
    }

    pub(crate) fn configure(&mut self, settings: &crate::config::FontSettings) -> Result<()> {
        if let Some(fallbacks) = &settings.fallbacks {
            self.set_fallback_families(fallbacks)?;
        }
        self.ensure_family(&settings.default_family)?;
        self.default_family = settings.default_family.clone();
        self.locale = settings.locale.clone();
        Ok(())
    }

    /// Replace the fallback chain for library callers; requested families remain strict.
    pub fn set_fallback_families(&mut self, families: &[String]) -> Result<()> {
        if families.len() > MAX_FACES
            || families
                .iter()
                .any(|family| family.is_empty() || family.len() > 256)
        {
            return Err(invalid(
                "A fallback chain needs at most 256 nonempty family names, each up to 256 bytes",
            ));
        }
        let mut names = Vec::new();
        for family in families {
            let family = if self.explicit.contains(&family.to_lowercase()) {
                self.families[&family.to_lowercase()].clone()
            } else {
                self.ensure_style(family, FontStyle::normal())?
            };
            if !names.contains(&family) {
                names.push(family);
            }
        }
        if self.lock.is_some() && self.fallbacks != names {
            return Err(Error::new(
                "FONT_LOCK_MISMATCH",
                "A locked font fallback chain cannot be changed; reload the project's configuration",
            ));
        }
        self.fallbacks = names;
        Ok(())
    }

    pub fn inventory(&mut self) -> Result<Vec<FontInfo>> {
        while !self.pending.is_empty() {
            self.load_pending(0)?;
        }
        let mut information = self.registered.values().cloned().collect::<Vec<_>>();
        information.sort_by(|a, b| {
            (
                &a.family,
                a.weight,
                a.width,
                &a.slant,
                &a.source,
                a.face_index,
            )
                .cmp(&(
                    &b.family,
                    b.weight,
                    b.width,
                    &b.slant,
                    &b.source,
                    b.face_index,
                ))
        });
        Ok(information)
    }

    pub(crate) fn enforce_lock(&mut self, lock: crate::font_lock::Manifest) -> Result<()> {
        lock.require_settings(self)?;
        for info in self.registered.values() {
            lock.require_face(info)?;
        }
        self.lock = Some(lock);
        Ok(())
    }

    fn load_pending(&mut self, index: usize) -> Result<()> {
        let pending = &self.pending[index];
        let bytes = pending.source.read(self.cancel.as_ref())?;
        let source = pending.source.path.display().to_string();
        let key = pending.names[0].to_lowercase();
        let selected = self
            .pending
            .iter()
            .enumerate()
            .filter(|(_, candidate)| {
                candidate.source.path == pending.source.path
                    && candidate.source.revision == pending.source.revision
                    && candidate
                        .names
                        .iter()
                        .any(|name| name.to_lowercase() == key)
            })
            .map(|(position, _)| position)
            .collect::<Vec<_>>();
        let indices = selected
            .iter()
            .map(|position| self.pending[*position].index)
            .collect::<Vec<_>>();
        let families = self.register_indices(&bytes, &source, None, None, Some(&indices))?;
        if families.len() != selected.len() {
            return Err(invalid("Missing indexed font face"));
        }
        for (position, family) in selected.iter().zip(families) {
            let names = &self.pending[*position].names;
            if !names
                .iter()
                .any(|name| name.to_lowercase() == family.to_lowercase())
            {
                return Err(invalid("Font family differs from its indexed names"));
            }
            for name in names {
                self.families.insert(name.to_lowercase(), family.clone());
            }
        }
        for position in selected.into_iter().rev() {
            self.pending.remove(position);
        }
        Ok(())
    }

    pub(super) fn resolve(
        &mut self,
        primary: &str,
        style: FontStyle,
        arguments: &FontArguments,
        fallback_count: usize,
    ) -> Result<ResolvedFonts> {
        let mut families = vec![self.ensure_style(primary, style)?];
        for family in self.fallbacks.clone().into_iter().take(fallback_count) {
            let family = self.ensure_style(&family, style)?;
            if !families.contains(&family) {
                families.push(family);
            }
        }
        let bases = families
            .iter()
            .map(|family| {
                let face = self
                    .provider
                    .match_family_style(family, style)
                    .ok_or_else(|| invalid("Registered family has no matching face"))?;
                self.registered
                    .get(&face.unique_id())
                    .cloned()
                    .ok_or_else(|| invalid("Font source metadata is unavailable"))
            })
            .collect::<Result<Vec<_>>>()?;
        let selected =
            self.collection
                .find_typefaces_with_font_arguments(&families, style, Some(arguments));
        if selected.len() != families.len() {
            return Err(invalid(
                "Font selection did not resolve the declared families",
            ));
        }
        let information = selected
            .into_iter()
            .zip(bases)
            .map(|(face, base)| Ok((face.unique_id(), base.instance(&face)?)))
            .collect::<Result<_>>()?;
        Ok(ResolvedFonts {
            families,
            information,
        })
    }

    pub(super) fn ready_fallback_count(
        &mut self,
        primary: &str,
        style: FontStyle,
    ) -> Result<usize> {
        self.ensure_style(primary, style)?;
        Ok(self
            .fallbacks
            .iter()
            .take_while(|family| {
                !self.pending.iter().any(|face| {
                    face.names
                        .iter()
                        .any(|name| name.to_lowercase() == family.to_lowercase())
                })
            })
            .count())
    }
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new("INVALID_ARGUMENT", message)
}

#[cfg(test)]
mod sharing_tests {
    use super::*;
    use crate::text::{TextBlock, TextSpec};

    #[test]
    fn unused_families_do_not_create_native_faces_or_eagerly_load_configured_fallbacks() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fonts/faces.ttc");
        let mut fonts = Fonts::new(&[path]).unwrap();
        assert_eq!(fonts.registered.len(), 1);
        assert_eq!(fonts.pending.len(), 4);
        fonts
            .set_fallback_families(&["LP Test Supplement".into()])
            .unwrap();
        assert_eq!(fonts.pending.len(), 4);
        fonts
            .layout(
                &TextSpec {
                    content: "日本語".into(),
                    ..Default::default()
                },
                None,
            )
            .unwrap();
        assert_eq!(fonts.registered.len(), 1);
        assert_eq!(fonts.pending.len(), 4);
        fonts
            .layout(
                &TextSpec {
                    content: "A".into(),
                    family: "LP Test Sans".into(),
                    ..Default::default()
                },
                None,
            )
            .unwrap();
        assert_eq!(fonts.pending.len(), 1);
        assert_eq!(fonts.registered.len(), 4);
        assert!(
            fonts.pending[0]
                .names
                .iter()
                .any(|name| name == "LP Test Supplement")
        );
        fonts
            .layout(
                &TextSpec {
                    content: "\u{f0000}".into(),
                    family: "LP Test Sans".into(),
                    ..Default::default()
                },
                None,
            )
            .unwrap();
        assert!(fonts.pending.is_empty());
        assert_eq!(fonts.registered.len(), 5);
    }

    fn raster(block: TextBlock) -> Vec<u8> {
        let mut surface = skia_safe::surfaces::raster_n32_premul((360, 100)).unwrap();
        surface.canvas().clear(skia_safe::Color::WHITE);
        block.paragraph.paint(surface.canvas(), (4.0, 4.0));
        skia_safe::png_encoder::encode_image(None, &surface.image_snapshot(), &Default::default())
            .unwrap()
            .as_bytes()
            .to_vec()
    }

    #[test]
    fn shared_bundled_faces_match_owned_font_sources_after_contexts_drop_and_across_threads() {
        std::thread::scope(|scope| {
            for weight in [100, 400, 700, 900] {
                scope.spawn(move || {
                    let spec = TextSpec {
                        content: "日本語 ABC 123".into(),
                        size: 26.0,
                        weight,
                        ..Default::default()
                    };
                    let copied = {
                        // Configured fonts retain their independent owned-data
                        // path, providing the pre-sharing rendering reference.
                        let sources = [(
                            PathBuf::from("configured.ttf"),
                            Arc::from(DEFAULT_FONT_BYTES),
                        )];
                        let mut fonts = Fonts::from_sources(&sources).unwrap();
                        let block = fonts.layout(&spec, Some(340.0)).unwrap();
                        assert!(fonts.system.is_some());
                        assert!(!block.metrics.fonts.is_empty());
                        assert!(
                            block
                                .metrics
                                .fonts
                                .iter()
                                .all(|info| info.source == "configured.ttf")
                        );
                        block
                    };
                    let shared = {
                        let mut fonts = Fonts::new(&[]).unwrap();
                        let block = fonts.layout(&spec, Some(340.0)).unwrap();
                        assert!(
                            block
                                .metrics
                                .fonts
                                .iter()
                                .all(|info| info.source == "bundled")
                        );
                        block
                    };
                    assert_eq!(
                        (
                            copied.metrics.width,
                            copied.metrics.height,
                            copied.metrics.intrinsic_width
                        ),
                        (
                            shared.metrics.width,
                            shared.metrics.height,
                            shared.metrics.intrinsic_width
                        )
                    );
                    let identities = |block: &TextBlock| {
                        block
                            .metrics
                            .fonts
                            .iter()
                            .map(|font| (font.hash.clone(), font.face_hash.clone()))
                            .collect::<Vec<_>>()
                    };
                    assert_eq!(identities(&copied), identities(&shared));
                    assert_eq!(hash(&raster(copied)), hash(&raster(shared)));
                });
            }
        });
        // Once initialized, the bundled face can shape Japanese in a new
        // context without constructing its operating-system font manager.
        let mut fonts = Fonts::new(&[]).unwrap();
        assert!(fonts.system.is_none());
        let block = fonts
            .layout(
                &TextSpec {
                    content: "日本語 ABC 123".into(),
                    ..Default::default()
                },
                Some(340.0),
            )
            .unwrap();
        assert!(fonts.system.is_none());
        assert!(
            block
                .metrics
                .fonts
                .iter()
                .all(|info| info.source == "bundled")
        );
    }
}
