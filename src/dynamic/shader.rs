//! Reflect compiler preprocessing before preparing typed project inputs.

use super::*;
use crate::project::Project;

impl DynamicRuntime {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn shader_for_node(
        &mut self,
        project: &Project,
        node: &ResolvedNode,
        fill: &Value,
        property: &str,
        width: u32,
        height: u32,
        coordinates: Option<[f64; 2]>,
        time: f64,
        object: Option<&[u8]>,
        background: Option<&[u8]>,
    ) -> Result<Raster> {
        self.check()?;
        dimensions(width as f64, height as f64)?;
        let cancel = self.cancel.clone();
        // Declaration acquisition, typed asset capture and the draw share one
        // deadline. A second browser operation must not reset the shader limit.
        let budget = Budget::new(
            self.remaining()?
                .min(Duration::from_millis(self.shader_settings.timeout_ms)),
            cancel.as_ref(),
        );
        let (source, _) = crate::shader::source(project, node, fill, property)?;
        // Conservatively include hashes inside comments. Sources without any #
        // cannot contain directives and retain their browser-free CPU checks.
        let expanded = source
            .contains('#')
            .then(|| self.shader_declarations(source, &budget))
            .transpose()?;
        let request = crate::shader::request_with_declarations(
            project,
            node,
            fill,
            property,
            width,
            height,
            coordinates,
            time,
            object,
            background,
            expanded.as_deref(),
        )?;
        self.shader_with_budget(request, &budget)
    }

    fn shader_declarations(&mut self, source: &str, budget: &Budget<'_>) -> Result<Arc<str>> {
        budget.check()?;
        let key = format!(
            "shader-source:{}:{}",
            preprocessor::hash(),
            hash(source.as_bytes())
        );
        if let Some(Cached::ShaderSource(source)) = self.cached_entry(&key) {
            return Ok(source);
        }
        let browser = self.browser(Purpose::Shader, budget)?;
        let result = browser.with_tab(budget, |browser, session| {
            let request = json!({"source":source,"width":1,"height":1});
            let expression = format!(
                "({})({},{},true)",
                include_str!("../webgl.js"),
                request,
                json!(preprocessor::encoded())
            );
            let result = browser.evaluate(session, &expression, budget)?;
            if let Some(error) = result["error"].as_str() {
                return Err(shader_error(error.chars().take(8192).collect::<String>()));
            }
            let source = result["source"]
                .as_str()
                .filter(|source| source.len() <= 4 * 1024 * 1024)
                .ok_or_else(|| shader_error("Missing or excessive expanded shader source"))?;
            Ok(Arc::<str>::from(source))
        });
        self.shader_used = Some(Instant::now());
        let result = result
            .map_err(|error| budget.check().err().unwrap_or(error))
            .and_then(|source| {
                budget.check()?;
                self.check()?;
                Ok(source)
            });
        match result {
            Ok(source) => {
                // Source text and raster bytes share the same LRU, byte and
                // entry limits; disabled caching also disables this reuse.
                self.cache_entry(key, Cached::ShaderSource(source.clone()));
                Ok(source)
            }
            Err(error) => {
                self.shaders.take();
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expanded_sources_share_raster_cache_limits_refresh_and_runtime_identity() {
        let mut runtime = DynamicRuntime::new("unused");
        let mut settings = RuntimeSettings::default();
        settings.dynamic.cache_mib = 1;
        settings.dynamic.cache_entries = 2;
        runtime.configure(settings.clone()).unwrap();
        runtime.cache_entry(
            "source".into(),
            Cached::ShaderSource("x".repeat(700_000).into()),
        );
        let page = Raster {
            png: vec![0; 400_000].into(),
            metadata: json!({"kind":"browser"}),
        };
        runtime.cache("page".into(), page);
        assert!(!runtime.cache.contains_key("source"));
        runtime.cache_entry(
            "source".into(),
            Cached::ShaderSource("x".repeat(400_000).into()),
        );
        runtime.refresh_pages();
        assert!(!runtime.cache.contains_key("page"));
        assert!(matches!(
            runtime.cached_entry("source"),
            Some(Cached::ShaderSource(_))
        ));
        assert!(runtime.idle_deadline().is_none());
        runtime.cache_entry(
            "excessive".into(),
            Cached::ShaderSource("x".repeat(1024 * 1024 + 1).into()),
        );
        assert!(!runtime.cache.contains_key("excessive"));
        runtime = runtime.with_executable("another-runtime");
        assert!(runtime.cache.is_empty());
        for (bytes, entries) in [(0, 2), (1, 0)] {
            settings.dynamic.cache_mib = bytes;
            settings.dynamic.cache_entries = entries;
            runtime.configure(settings.clone()).unwrap();
            runtime.cache_entry("disabled".into(), Cached::ShaderSource("small".into()));
            assert!(runtime.cache.is_empty());
        }
        settings.dynamic.cache_mib = 1;
        settings.dynamic.cache_entries = 2;
        runtime.configure(settings).unwrap();
        runtime.cache_entry("source".into(), Cached::ShaderSource("small".into()));
        runtime.begin_render(None, true);
        assert!(runtime.cache.is_empty());
        assert!(runtime.cache_order.is_empty());
    }
}
