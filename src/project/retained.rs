//! Owned node payloads, excluding the inline storage charged by LayoutItem.
use super::*;
use std::mem::size_of;

impl NodeAddress {
    fn retained_payload_bytes(&self) -> usize {
        self.node_id.capacity()
            + self.instance_path.capacity() * size_of::<String>()
            + self
                .instance_path
                .iter()
                .map(String::capacity)
                .sum::<usize>()
    }
}

impl ResolvedNode {
    pub(crate) fn retained_payload_bytes(&self) -> usize {
        // Count properties once, independently of JSON escaping or serialization
        // length. Keep the existing estimates for opaque execution contexts.
        self.retained_context_bytes()
            + self.address.retained_payload_bytes()
            + self.source_path.capacity()
            + self.source_id.capacity()
            + self.component_key.capacity()
            + self
                .instance_identity
                .as_ref()
                .map_or(0, |(path, id)| path.capacity() + id.capacity())
            + self
                .properties
                .iter()
                .map(|(key, value)| key.capacity() + 96 + crate::retained::json_bytes(value))
                .sum::<usize>()
            + self
                .property_sources
                .iter()
                .map(|(key, path)| {
                    size_of::<(String, PathBuf)>() + 96 + key.capacity() + path.capacity()
                })
                .sum::<usize>()
            + self
                .theme
                .iter()
                .map(|(key, value)| {
                    size_of::<(String, String)>() + 96 + key.capacity() + value.capacity()
                })
                .sum::<usize>()
            + self.browser_scope.as_ref().map_or(0, PathBuf::capacity)
            + self.browser_url_scope.as_ref().map_or(0, PathBuf::capacity)
            + self.children.capacity() * size_of::<ResolvedNode>()
            + self
                .children
                .iter()
                .map(Self::retained_payload_bytes)
                .sum::<usize>()
            + self
                .generated_by
                .as_ref()
                .map_or(0, NodeAddress::retained_payload_bytes)
            + self
                .icon_metadata
                .as_ref()
                .map_or(0, crate::retained::json_bytes)
            + self.icon.as_ref().map_or(0, |icon| match icon {
                // The pack is charged by Project's icon registry, not per node.
                Ok(icon) => {
                    size_of::<crate::icons::ResolvedIcon>()
                        + 2 * size_of::<usize>()
                        + icon.name.capacity()
                        + icon.canonical.capacity()
                }
                Err(error) => {
                    error.message.capacity()
                        + error
                            .details
                            .as_ref()
                            .map_or(0, crate::retained::json_bytes)
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> crate::layout::LayoutSnapshot {
        let project = Project::local(
            Document::parse(
                json!({
                    "version":"2.21", "children":[{
                        "id":"box", "type":"rectangle", "width":10, "height":10,
                        "metadata":{"type":"test", "payload":""}
                    }]
                })
                .to_string(),
            )
            .unwrap(),
        )
        .unwrap();
        crate::layout::compute(
            project.resolve().unwrap(),
            &mut crate::text::Fonts::new(&[]).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn retained_node_properties_charge_owned_capacity_once_without_json_expansion() {
        let mut snapshot = snapshot();
        let baseline = snapshot.retained_bytes();
        let bytes = 1024 * 1024;
        for character in ['a', '\n', '\0'] {
            snapshot.items[0].node.properties["metadata"]["payload"] =
                Value::String(character.to_string().repeat(bytes));
            assert_eq!(snapshot.retained_bytes() - baseline, bytes);
        }
        let Value::String(payload) = &mut snapshot.items[0].node.properties["metadata"]["payload"]
        else {
            unreachable!()
        };
        payload.reserve(bytes);
        let capacity = payload.capacity();
        assert!(capacity >= bytes * 2);
        assert_eq!(snapshot.retained_bytes() - baseline, capacity);
    }

    #[test]
    fn retained_node_identity_sources_and_generated_addresses_keep_spare_capacity() {
        let mut snapshot = snapshot();
        let baseline = snapshot.retained_bytes();
        let node = &mut snapshot.items[0].node;
        let mut name = String::with_capacity(4096);
        name.push_str("instance");
        node.generated_by = Some(NodeAddress {
            node_id: name,
            instance_path: Vec::with_capacity(5),
        });
        let mut path = PathBuf::with_capacity(8192);
        path.push("source.pen");
        let mut key = String::with_capacity(2048);
        key.push_str("custom");
        let expected = 4096
            + 5 * size_of::<String>()
            + size_of::<(String, PathBuf)>()
            + 96
            + key.capacity()
            + path.capacity();
        node.property_sources.insert(key, path);
        assert_eq!(snapshot.retained_bytes() - baseline, expected);
    }

    #[test]
    fn retained_project_counts_each_document_source_once_and_preserves_other_inputs() {
        let mut source = String::with_capacity(8192);
        source.push_str(r#"{"version":"2.21","children":[]}"#);
        let document = Document::parse(source).unwrap();
        let expected = document.retained_bytes();
        let mut project = Project::local(document).unwrap();
        assert_eq!(project.retained_bytes(), expected);

        let library = Document::parse(r#"{"version":"2.21","children":[]}"#.into()).unwrap();
        let library_bytes = library.retained_bytes();
        let source_bytes = project.captured_bytes() + library.text().len();
        project
            .load_document(
                Path::new("library.pen"),
                Some(library),
                &mut BTreeSet::new(),
            )
            .unwrap();
        assert_eq!(project.captured_bytes(), source_bytes);
        assert_eq!(project.retained_bytes(), expected + library_bytes);

        let asset: Arc<[u8]> = vec![0; 4096].into();
        project
            .capture_bytes("asset.bin".into(), "image", asset)
            .unwrap();
        assert_eq!(project.captured_bytes(), source_bytes + 4096);
        assert_eq!(project.retained_bytes(), expected + library_bytes + 4096);
    }
}
