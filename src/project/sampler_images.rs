//! Preserve string values while relocating the image files used by samplers.

use super::*;

pub(super) fn parse(document: &Document) -> Result<BTreeMap<String, String>> {
    let Some(value) = document
        .value()
        .pointer("/metadata/lazypencil/sampler_images")
    else {
        return Ok(BTreeMap::new());
    };
    let object = value
        .as_object()
        .ok_or_else(|| invalid("Sampler image bindings must be an object"))?;
    if object.len() != 2 || !object.contains_key("version") || !object.contains_key("images") {
        return Err(invalid("Invalid sampler image binding fields"));
    }
    if value["version"] != 1 {
        return Err(Error::new(
            "UNSUPPORTED_VERSION",
            "Unsupported sampler image bindings version",
        ));
    }
    let images = value["images"]
        .as_object()
        .filter(|images| images.len() <= 4096)
        .ok_or_else(|| invalid("Sampler image bindings require at most 4096 images"))?;
    let mut bytes = 0usize;
    images
        .iter()
        .map(|(uri, target)| {
            let target = target
                .as_str()
                .ok_or_else(|| invalid("A sampler image binding target must be a string"))?;
            bytes = bytes.saturating_add(uri.len()).saturating_add(target.len());
            if bytes > 4 * 1024 * 1024
                || uri.is_empty()
                || uri.len() > 4096
                || uri.chars().any(char::is_control)
                || uri.starts_with(['$', '#'])
                || uri.starts_with("pencil:")
                || (uri.contains("://") || uri.starts_with("file:"))
                    && crate::paths::local_file_url(uri).is_err()
                || target.is_empty()
                || target.len() > 4096
                || target.contains(['\\', ':'])
                || target.chars().any(char::is_control)
                || target
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..")
                || Path::new(target)
                    .components()
                    .any(|part| !matches!(part, std::path::Component::Normal(_)))
            {
                return Err(invalid("Invalid or oversized sampler image binding paths"));
            }
            Ok((uri.clone(), target.into()))
        })
        .collect()
}

impl Project {
    pub(super) fn sampler_uri<'a>(&'a self, origin: &Path, uri: &'a str) -> &'a str {
        self.sampler_images
            .get(origin)
            .and_then(|images| images.get(uri))
            .map(String::as_str)
            .unwrap_or(uri)
    }
}
