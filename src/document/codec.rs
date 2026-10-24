//! Explicit wire versions sharing the currently implemented entity model.

use super::Document;
use crate::{Error, Result};

pub const SUPPORTED_DOCUMENT_VERSIONS: &[&str] = &["2.19", "2.21"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Pen219,
    Pen221,
}

impl Codec {
    pub const DEFAULT: Self = Self::Pen221;

    pub fn parse(version: &str) -> Option<Self> {
        match version {
            "2.19" => Some(Self::Pen219),
            "2.21" => Some(Self::Pen221),
            _ => None,
        }
    }

    pub const fn version(self) -> &'static str {
        match self {
            Self::Pen219 => "2.19",
            Self::Pen221 => "2.21",
        }
    }

    /// Encode an application-owned template for its consumer. This is not an
    /// implicit migration path for user documents or imported user libraries.
    pub(crate) fn encode_owned_template(self, template: &Document) -> Result<Document> {
        if template.codec().is_none() {
            return Err(Error::new(
                "UNSUPPORTED_VERSION",
                "Cannot encode an unknown template schema",
            ));
        }
        let mut output = template.clone();
        if output.version() != self.version() {
            output.set_property("", "version", &serde_json::json!(self.version()))?;
        }
        Ok(output)
    }
}
