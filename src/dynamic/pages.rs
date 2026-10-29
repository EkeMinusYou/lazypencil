//! Local pages consume captured project responses, never browser filesystem IO.

use super::*;
use crate::project::{BrowserResources, Project};

pub(super) struct PageInput {
    pub resources: Option<BrowserResources>,
    pub identity: Option<Value>,
}

impl PageInput {
    pub fn capture(project: Option<&Project>, node: &ResolvedNode) -> Result<Self> {
        let url = page_url(node)?;
        if url.len() > 1024 * 1024 {
            return Err(page_error("Browser URL exceeds 1 MiB"));
        }
        let local = url::Url::parse(url)
            .map_err(|_| page_error("Invalid browser URL"))?
            .scheme()
            == "file";
        if local && let Some(project) = project {
            let resources = project.browser_node_resources(node)?;
            let identity = json!({"revision":resources.revision,
                "owner_hash":hash(node.browser_origin().as_os_str().as_encoded_bytes())});
            Ok(Self {
                resources: Some(resources),
                identity: Some(identity),
            })
        } else {
            validate_url(url)?;
            Ok(Self {
                resources: None,
                identity: None,
            })
        }
    }

    pub fn signature(&self, node: &ResolvedNode) -> Result<String> {
        Ok(hash(json!({"url":page_url(node)?,"device":node.properties.get("deviceId").and_then(Value::as_str),
            "resources":self.identity}).to_string().as_bytes()))
    }
}

pub(super) fn page_url(node: &ResolvedNode) -> Result<&str> {
    node.properties
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .ok_or_else(|| page_error("Browser node needs a URL"))
}

impl DynamicRuntime {
    pub(crate) fn render_result<T>(&mut self, result: Result<T>) -> Result<T> {
        if result
            .as_ref()
            .is_err_and(|error| error.code == "CANCELLED")
        {
            self.pages.take();
            #[cfg(unix)]
            self.shared_pages.take();
            self.shaders.take();
            self.page_used = None;
            self.shader_used = None;
            self.page_snapshot = None;
            self.cache.clear();
            self.cache_order.clear();
        }
        result
    }

    /// Render a local file URL only from this project's immutable, bounded
    /// namespace. HTTP/data pages keep the ordinary acquisition behavior.
    pub fn page_in_project(
        &mut self,
        project: &Project,
        node: &ResolvedNode,
        width: f32,
        height: f32,
        scale: f32,
    ) -> Result<Raster> {
        let result = (|| {
            self.check()?;
            let input = PageInput::capture(Some(project), node)?;
            self.page_with_input(node, width, height, scale, input)
        })();
        self.page_result(result)
    }

    pub(super) fn page_result<T>(&mut self, result: Result<T>) -> Result<T> {
        if result.is_err() {
            self.pages.take();
            #[cfg(unix)]
            self.shared_pages.take();
            self.page_used = None;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use std::fs;

    #[test]
    fn local_page_identity_binds_captured_bytes_and_owner_without_expanding_file_access() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join(".git")).unwrap();
        fs::create_dir(temp.path().join("web")).unwrap();
        let html = temp.path().join("web/index.html");
        fs::write(&html, "<html>one</html>").unwrap();
        let url = format!(
            "file:///original/site/index.html?q={}#literal-$",
            "x".repeat(128 * 1024)
        );
        let mut signatures = Vec::new();
        let mut revisions = Vec::new();
        for name in ["first", "second"] {
            let store = Store::init(
                temp.path().join(format!("{name}.pen")),
                Some(temp.path().join(format!("private-{name}"))),
            )
            .unwrap();
            fs::write(store.path(),json!({"version":"2.21","metadata":{"lazypencil":{"browser_resources":{"version":1,"roots":[{"url":"file:///original/site/","path":"web"}]}}},"children":[{"id":"page","type":"browser","url":url,"width":32,"height":32}]}).to_string()).unwrap();
            let project = Project::load(&store).unwrap();
            let scene = project.resolve().unwrap();
            let node = &scene.roots[0];
            assert!(PageInput::capture(None, node).is_err());
            let input = PageInput::capture(Some(&project), node).unwrap();
            signatures.push(input.signature(node).unwrap());
            revisions.push(input.resources.as_ref().unwrap().revision.clone());
            let mut unlisted = node.clone();
            unlisted
                .properties
                .insert("url".into(), json!("file:///original/unlisted.html"));
            assert!(PageInput::capture(Some(&project), &unlisted).is_err());
            unlisted.properties.insert(
                "url".into(),
                json!(format!(
                    "file:///original/site/index.html?q={}",
                    "x".repeat(1024 * 1024)
                )),
            );
            assert!(PageInput::capture(Some(&project), &unlisted).is_err());
            fs::write(&html, "<html>two</html>").unwrap();
            let changed = Project::load(&store).unwrap();
            let scene = changed.resolve().unwrap();
            let next = PageInput::capture(Some(&changed), &scene.roots[0]).unwrap();
            assert_ne!(
                next.signature(&scene.roots[0]).unwrap(),
                signatures.last().unwrap().as_str()
            );
            fs::write(&html, "<html>one</html>").unwrap();
        }
        assert_eq!(revisions[0], revisions[1]);
        assert_ne!(signatures[0], signatures[1]);
    }
}
