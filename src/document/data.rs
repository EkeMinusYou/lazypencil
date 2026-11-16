//! Document resource declarations live in a disabled, standard frame's metadata.
use super::*;

const TYPE: &str = "lazypencil-document";

/// Build a standard frame carrying document resource declarations.
pub fn data_node(namespace: Value) -> Value {
    json!({"id":"lazypencil-document-data",
        "type":"frame","name":"Document resources","enabled":false,
        "layout":"none","width":0,"height":0,
        "metadata":{"type":TYPE,"lazypencil":namespace}})
}

impl Document {
    fn data_pointer(&self) -> Result<Option<String>> {
        let mut pointer = None;
        for (index, node) in self.value["children"]
            .as_array()
            .expect("parsed document")
            .iter()
            .enumerate()
        {
            if node.pointer("/metadata/type").and_then(Value::as_str) != Some(TYPE) {
                continue;
            }
            if pointer.is_some() {
                return Err(invalid(
                    "A document may have only one document resource metadata node",
                ));
            }
            pointer = Some(format!("/children/{index}/metadata"));
        }
        Ok(pointer)
    }

    /// Read the document's application data from a standard node metadata map.
    pub fn document_data(&self) -> Result<Option<&Value>> {
        let Some(pointer) = self.data_pointer()? else {
            return Ok(None);
        };
        let value = self
            .value
            .pointer(&pointer)
            .expect("metadata node")
            .get("lazypencil");
        if value.is_some_and(|value| !value.is_object()) {
            return Err(invalid("Document resource metadata must be an object"));
        }
        Ok(value)
    }

    /// Update application data without rewriting unrelated authored source.
    pub fn set_document_data(&mut self, key: &str, value: &Value) -> Result<()> {
        self.document_data()?;
        if let Some(pointer) = self.data_pointer()? {
            if self
                .value
                .pointer(&pointer)
                .expect("metadata node")
                .get("lazypencil")
                .is_none()
            {
                self.set_property(&pointer, "lazypencil", &json!({}))?;
            }
            self.set_property(&format!("{pointer}/lazypencil"), key, value)
        } else {
            let mut node = data_node(json!({key:value}));
            let mut index = 0;
            while self
                .nodes
                .contains_key(node["id"].as_str().expect("data node ID"))
            {
                index += 1;
                node["id"] = json!(format!("lazypencil-document-data-{index}"));
            }
            self.insert_element(
                "/children",
                self.value["children"]
                    .as_array()
                    .expect("parsed document")
                    .len(),
                &node.to_string(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_metadata_uses_standard_nodes_and_preserves_unrelated_source() {
        let source = "{\"version\":\"2.22\",\"children\":[\n// Keep this shape.\n{\"id\":\"box\",\"type\":\"rectangle\",\"width\":1.000e2}]}";
        let mut document = Document::parse(source.into()).unwrap();
        document
            .set_document_data("browser_resources", &json!({"version":1,"roots":[]}))
            .unwrap();
        document
            .set_document_data("sampler_images", &json!({"version":1,"images":{}}))
            .unwrap();
        assert!(document.value().get("metadata").is_none());
        assert_eq!(
            document.document_data().unwrap().unwrap()["browser_resources"]["roots"],
            json!([])
        );
        assert_eq!(document.node_count(), 2);
        assert!(document.text().contains(
            "// Keep this shape.\n{\"id\":\"box\",\"type\":\"rectangle\",\"width\":1.000e2}"
        ));
        crate::operations::validate_local(&document).unwrap();
        let mut duplicate = data_node(json!({}));
        duplicate["id"] = json!("second-data-node");
        document
            .insert_element("/children", 2, &duplicate.to_string())
            .unwrap();
        assert_eq!(
            document.document_data().unwrap_err().code,
            "INVALID_DOCUMENT"
        );
    }
}
