//! Explicit, portable font manifests. Ordinary reads never update their baseline.

use crate::{
    Error, Result,
    document::hash,
    project::Project,
    store::Store,
    text::{FontInfo, Fonts},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    version: u32,
    engine: String,
    native_sources: String,
    bundled_pack: Value,
    input_hashes: Vec<String>,
    default_family: String,
    fallback_families: Vec<String>,
    locale: String,
    faces: Vec<FontInfo>,
}

fn bundled_pack() -> Value {
    crate::text::bundled_manifest()
}

fn native_sources() -> String {
    hash(include_bytes!("../vendor/skia-bindings/Cargo.toml"))
}

fn portable(mut info: FontInfo) -> FontInfo {
    if !matches!(info.source.as_str(), "bundled" | "system:font-data") {
        info.source = "file".into();
    }
    info
}

fn invalid(message: &str) -> Error {
    Error::new("FONT_LOCK_INVALID", message)
}

fn mismatch(reason: &str) -> Error {
    Error::new(
        "FONT_LOCK_MISMATCH",
        "Fonts differ from the project's lockfile; review and explicitly update it",
    )
    .details(json!({"reason":reason}))
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

impl Manifest {
    fn capture(fonts: &mut Fonts, input_hashes: Vec<String>) -> Result<Self> {
        let mut faces = fonts
            .inventory()?
            .into_iter()
            .map(portable)
            .collect::<Vec<_>>();
        // Source paths and native unique IDs must not bind a portable baseline.
        faces.sort_by_cached_key(|face| serde_json::to_string(face).expect("Font metadata"));
        faces.dedup();
        let manifest = Self {
            version: 1,
            engine: crate::layout::ENGINE.into(),
            native_sources: native_sources(),
            bundled_pack: bundled_pack(),
            input_hashes,
            default_family: fonts.default_family().into(),
            fallback_families: fonts.fallback_families().into(),
            locale: fonts.locale().into(),
            faces,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    pub(crate) fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > crate::config::MAX_FONT_LOCK_BYTES {
            return Err(invalid("Font lockfile exceeds 1 MiB"));
        }
        // Do not echo a file's contents through JSON parser diagnostics.
        let manifest: Self =
            serde_json::from_slice(bytes).map_err(|_| invalid("Malformed font lockfile"))?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<()> {
        if self.version != 1
            || self.engine.len() > 256
            || !digest(&self.native_sources)
            || self.input_hashes.len() > 256
            || self.input_hashes.iter().any(|value| !digest(value))
            || self.faces.is_empty()
            || self.faces.len() > 256
            || self.default_family.is_empty()
            || self.default_family.len() > 256
            || self.fallback_families.len() > 256
            || self
                .fallback_families
                .iter()
                .any(|family| family.is_empty() || family.len() > 256)
            || self.locale.is_empty()
            || self.locale.len() > 128
        {
            return Err(invalid("Unsupported or out-of-bounds font lockfile"));
        }
        let mut seen = std::collections::BTreeSet::new();
        for face in &self.faces {
            if face.family.is_empty()
                || face.family.len() > 256
                || !matches!(
                    face.source.as_str(),
                    "file" | "bundled" | "system:font-data"
                )
                || !digest(&face.hash)
                || !digest(&face.face_hash)
                || face.face_index >= 256
                || !(1..=1000).contains(&face.weight)
                || !(1..=9).contains(&face.width)
                || !matches!(face.slant.as_str(), "upright" | "italic" | "oblique")
                || face
                    .postscript_name
                    .as_ref()
                    .is_some_and(|name| name.len() > 256)
                || face.axes.len() > 64
            {
                return Err(invalid("Invalid locked font face"));
            }
            let mut axes = std::collections::BTreeSet::new();
            for axis in &face.axes {
                if axis.tag.len() != 4
                    || !axis.tag.is_ascii()
                    || !axes.insert(&axis.tag)
                    || ![axis.min, axis.default, axis.max, axis.value]
                        .iter()
                        .all(|value| value.is_finite())
                    || !(axis.min <= axis.default
                        && axis.default <= axis.max
                        && axis.min <= axis.value
                        && axis.value <= axis.max)
                {
                    return Err(invalid("Invalid locked font variation axis"));
                }
            }
            if !seen.insert(serde_json::to_string(face).expect("Finite font metadata")) {
                return Err(invalid("Duplicate locked font face"));
            }
        }
        Ok(())
    }

    pub(crate) fn require_inputs(&self, hashes: &[String]) -> Result<()> {
        if self.engine != crate::layout::ENGINE
            || self.native_sources != native_sources()
            || self.bundled_pack != bundled_pack()
        {
            return Err(mismatch("engine_or_bundled_pack"));
        }
        if self.input_hashes != hashes {
            return Err(mismatch("configured_font_data_or_order"));
        }
        Ok(())
    }

    pub(crate) fn require_settings(&self, fonts: &Fonts) -> Result<()> {
        if self.default_family != fonts.default_family()
            || self.fallback_families != fonts.fallback_families()
            || self.locale != fonts.locale()
        {
            return Err(mismatch("font_selection_settings"));
        }
        Ok(())
    }

    pub(crate) fn require_face(&self, face: &FontInfo) -> Result<()> {
        if !self.faces.contains(&portable(face.clone())) {
            return Err(mismatch("unlocked_font_face").details(json!({"reason":"unlocked_font_face","family":face.family,"hash":face.hash,"face_index":face.face_index,"face_hash":face.face_hash})));
        }
        Ok(())
    }
}

/// Compute the real document (including generated text) before explicitly publishing a baseline.
pub fn create(
    store: &Store,
    files: &[PathBuf],
    output: Option<&Path>,
    expected_revision: Option<&str>,
) -> Result<Value> {
    let mut runtime = crate::script::ScriptRuntime::current()?;
    let cancel = runtime.cancellation();
    let mut project = Project::load(store)?;
    project.set_cancellation(cancel.as_ref());
    project.add_font_files(files)?;
    let configured = project.font_lock_path();
    let requested = output
        .map(Path::to_path_buf)
        .or_else(|| configured.clone())
        .ok_or_else(|| {
            Error::new(
                "INVALID_ARGUMENT",
                "Configure project font_lock or supply --output",
            )
        })?;
    let parent = requested
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = crate::paths::canonical(parent)?;
    let path = parent.join(
        requested
            .file_name()
            .ok_or_else(|| Error::new("INVALID_ARGUMENT", "Font lockfile needs a filename"))?,
    );
    if path.to_str().is_none() || path.extension().is_none_or(|extension| extension != "json") {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Font lockfile output must be a UTF-8 .json path",
        ));
    }
    // The publication lock also coordinates with dependency readers/editors.
    let _lock = store.lock_publication(&path, cancel.as_ref())?;
    if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Font lockfile output must not be a symbolic link",
        ));
    }
    if path == store.path()
        || project
            .dependencies()
            .iter()
            .any(|dependency| dependency.path == path && dependency.kind != "font_lock")
    {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Font lockfile must not overwrite a document, configuration, font or asset",
        ));
    }
    let absent = crate::paths::MissingPath::capture(&path)?;
    let previous = if absent.is_some() {
        None
    } else {
        Some(crate::config::read_font_lock(&path)?)
    };
    if expected_revision.is_some_and(|expected| {
        previous
            .as_ref()
            .is_none_or(|bytes| hash(bytes) != expected)
    }) {
        return Err(Error::new(
            "REVISION_CONFLICT",
            "Font lockfile changed; review its current hash",
        ));
    }
    project.verify_unchanged()?;
    let mut fonts = project.capture_fonts()?;
    let snapshot = crate::script::compute(&mut project, &mut fonts, &mut runtime)?;
    let manifest = Manifest::capture(&mut fonts, project.font_input_hashes()?)?;
    let mut bytes =
        serde_json::to_vec_pretty(&manifest).map_err(|_| invalid("Cannot encode font lockfile"))?;
    bytes.push(b'\n');
    if bytes.len() > crate::config::MAX_FONT_LOCK_BYTES {
        return Err(invalid("Font lockfile exceeds 1 MiB"));
    }
    let reused = previous
        .as_ref()
        .is_some_and(|old| old.as_ref() == bytes.as_slice());
    if let Some(previous) = &previous
        && !reused
        && expected_revision.is_none()
    {
        return Err(Error::new(
            "REVISION_CONFLICT",
            "Updating a font lockfile requires --expected-lock-revision",
        )
        .details(json!({"actual_lock_revision":hash(previous)})));
    }
    crate::store::require_revision(&store.read()?, &snapshot.revision)?;
    project.verify_unchanged()?;
    if crate::paths::canonical(
        requested
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )? != parent
    {
        return Err(Error::new(
            "REVISION_CONFLICT",
            "Font lockfile parent changed",
        ));
    }
    if let Some(absent) = &absent {
        absent.verify()?;
    } else if crate::config::read_font_lock(&path)?
        != previous.as_ref().expect("Existing lockfile").clone()
    {
        return Err(Error::new(
            "REVISION_CONFLICT",
            "Font lockfile changed during acquisition",
        ));
    }
    crate::cancel::check(cancel.as_ref())?;
    if !reused {
        if absent.is_some() {
            let mut file = tempfile::NamedTempFile::new_in(&parent)
                .map_err(|error| Error::io(&parent, error))?;
            file.write_all(&bytes)
                .map_err(|error| Error::io(&path, error))?;
            file.as_file()
                .sync_all()
                .map_err(|error| Error::io(&path, error))?;
            crate::cancel::check(cancel.as_ref())?;
            file.persist_noclobber(&path)
                .map_err(|error| Error::io(&path, error.error))?;
            crate::store::sync_directory(&parent)?;
        } else {
            crate::store::atomic_replace(&path, &bytes)?;
        }
    }
    let active = configured.as_ref().is_some_and(|configured| {
        crate::paths::canonical(configured).is_ok_and(|configured| configured == path)
    });
    Ok(
        json!({"path":path,"hash":hash(&bytes),"version":1,"faces":manifest.faces.len(),"active":active,"reused":reused,"document_revision":snapshot.revision,"configuration_key":"font_lock"}),
    )
}

pub fn check(store: &Store, files: &[PathBuf]) -> Result<Value> {
    let mut runtime = crate::script::ScriptRuntime::current()?;
    let cancel = runtime.cancellation();
    let mut project = Project::load(store)?;
    project.set_cancellation(cancel.as_ref());
    project.add_font_files(files)?;
    let path = project.font_lock_path().ok_or_else(|| {
        Error::new(
            "INVALID_ARGUMENT",
            "No font_lock is configured for this project",
        )
    })?;
    let mut fonts = project.load_fonts()?;
    let snapshot = crate::script::compute(&mut project, &mut fonts, &mut runtime)?;
    fonts.inventory()?;
    crate::store::require_revision(&store.read()?, &snapshot.revision)?;
    project.verify_unchanged()?;
    crate::cancel::check(cancel.as_ref())?;
    Ok(
        json!({"valid":true,"path":path,"revision":snapshot.revision,"dependency_revision":project.dependency_revision(),"snapshot_id":snapshot.id}),
    )
}
