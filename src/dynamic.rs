//! Browser acquisitions and WebGL rasters, separate from document layout.

use crate::{
    Error, Result,
    cdp::{Browser, Budget, Purpose},
    config::runtime::{
        BrowserSettings, DynamicSettings, PageSelectorState, PageWaitUntil, RuntimeSettings,
        ShaderSettings,
    },
    document::hash,
    project::ResolvedNode,
};
use base64::Engine;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct Raster {
    pub png: Arc<[u8]>,
    pub metadata: Value,
}

pub struct DynamicRuntime {
    data: PathBuf,
    executable: Option<PathBuf>,
    page_scope: Option<String>,
    page_snapshot: Option<String>,
    pages: Option<Browser>,
    shaders: Option<Browser>,
    page_used: Option<Instant>,
    shader_used: Option<Instant>,
    cache: BTreeMap<String, Raster>,
    cache_order: VecDeque<String>,
    cancel: Option<CancellationToken>,
    begun: Instant,
    refresh: bool,
    browser_settings: BrowserSettings,
    shader_settings: ShaderSettings,
    settings: DynamicSettings,
}

impl DynamicRuntime {
    pub fn new(data: impl Into<PathBuf>) -> Self {
        Self {
            data: data.into(),
            executable: None,
            page_scope: None,
            page_snapshot: None,
            pages: None,
            shaders: None,
            page_used: None,
            shader_used: None,
            cache: BTreeMap::new(),
            cache_order: VecDeque::new(),
            cancel: None,
            begun: Instant::now(),
            refresh: false,
            browser_settings: BrowserSettings::default(),
            shader_settings: ShaderSettings::default(),
            settings: DynamicSettings::default(),
        }
    }
    pub fn with_executable(mut self, path: impl Into<PathBuf>) -> Self {
        self.executable = Some(path.into());
        self
    }
    /// Opt into in-memory page state for this document. Ordinary CLI/MCP
    /// runtimes retain their fresh-context acquisition behavior.
    pub fn with_retained_pages(mut self, document: &Path) -> Self {
        let scope = hash(document.as_os_str().as_encoded_bytes());
        if self.page_scope.as_ref() != Some(&scope) {
            self.pages.take();
            self.page_used = None;
            self.cache.clear();
            self.cache_order.clear();
            self.page_scope = Some(scope);
            self.page_snapshot = None;
        }
        self
    }

    fn page_identity(&self, node: &ResolvedNode) -> Option<String> {
        self.page_scope.as_ref().map(|scope| {
            hash(json!({"scope":scope,"address":&node.address,"source":hash(node.source_path.as_os_str().as_encoded_bytes())}).to_string().as_bytes())
        })
    }

    pub(crate) fn prepare_pages(&mut self, snapshot: &crate::layout::LayoutSnapshot) -> Result<()> {
        if self.page_scope.is_none() || self.page_snapshot.as_deref() == Some(snapshot.id.as_str())
        {
            return Ok(());
        }
        // Changed inputs must not resurrect an old URL's acquisition after a
        // node was removed, edited and later restored. Viewport-only requests
        // still reuse the same captured layout and cache.
        self.cache.clear();
        self.cache_order.clear();
        if self.pages.is_some() {
            let keys: BTreeSet<_> = snapshot
                .items
                .iter()
                .filter(|item| item.record.enabled && item.node.kind() == "browser")
                .filter_map(|item| self.page_identity(&item.node))
                .collect();
            let cancel = self.cancel.clone();
            let budget = Budget::new(self.remaining()?, cancel.as_ref());
            let result = match self.pages.as_mut() {
                Some(browser) => browser.retain_page_keys(&keys, &budget),
                None => Ok(()),
            };
            if let Err(error) = result {
                self.pages.take();
                return Err(budget.check().err().unwrap_or(error));
            }
        }
        self.page_snapshot = Some(snapshot.id.clone());
        Ok(())
    }
    /// Apply captured data-only limits without launching a process.
    pub fn configure(&mut self, settings: RuntimeSettings) -> Result<()> {
        settings.validate()?;
        if self.page_scope.is_some() && self.browser_settings != settings.browser {
            self.pages.take();
            self.page_used = None;
        }
        if self.browser_settings != settings.browser
            || self.shader_settings != settings.shader
            || self.settings != settings.dynamic
        {
            self.cache.clear();
            self.cache_order.clear();
            self.browser_settings = settings.browser;
            self.shader_settings = settings.shader;
            self.settings = settings.dynamic;
        }
        self.expire_idle(Instant::now());
        Ok(())
    }
    pub fn begin_render(&mut self, cancel: Option<CancellationToken>, refresh: bool) {
        self.expire_idle(Instant::now());
        self.cancel = cancel;
        self.begun = Instant::now();
        self.refresh = refresh;
        if refresh {
            self.cache.clear();
            self.cache_order.clear();
        }
    }
    /// Refresh browser acquisitions for one render without invalidating shaders.
    pub(crate) fn refresh_pages(&mut self) {
        // Remove prior-frame acquisitions once. Repeated composition of the
        // same page in this frame then shares its newly captured raster.
        self.cache
            .retain(|_, raster| raster.metadata["kind"] != "browser");
        let cache = &self.cache;
        self.cache_order.retain(|key| cache.contains_key(key));
    }
    /// The owned Chromium child IDs, for runtime diagnostics.
    pub fn process_ids(&self) -> Vec<u32> {
        [self.pages.as_ref(), self.shaders.as_ref()]
            .into_iter()
            .flatten()
            .map(Browser::process_id)
            .collect()
    }
    /// Page and shader processes expire independently; cached rasters remain valid.
    pub fn idle_deadline(&self) -> Option<Instant> {
        [
            (
                self.pages.as_ref().and(self.page_used),
                self.browser_settings.idle_seconds,
            ),
            (
                self.shaders.as_ref().and(self.shader_used),
                self.shader_settings.idle_seconds,
            ),
        ]
        .into_iter()
        .filter_map(|(used, seconds)| used.map(|used| used + Duration::from_secs(seconds)))
        .min()
    }
    pub fn expire_idle(&mut self, now: Instant) {
        for (slot, used, seconds) in [
            (
                &mut self.pages,
                &mut self.page_used,
                self.browser_settings.idle_seconds,
            ),
            (
                &mut self.shaders,
                &mut self.shader_used,
                self.shader_settings.idle_seconds,
            ),
        ] {
            if slot.is_some() && used.is_some_and(|used| used + Duration::from_secs(seconds) <= now)
            {
                slot.take();
                *used = None;
            }
        }
    }
    pub fn check(&self) -> Result<()> {
        if self
            .cancel
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Err(Error::new("CANCELLED", "Render cancelled"));
        }
        self.remaining().map(|_| ())
    }
    fn browser(&mut self, purpose: Purpose, operation: &Budget<'_>) -> Result<&mut Browser> {
        let milliseconds = if purpose == Purpose::Page {
            self.browser_settings.launch_timeout_ms
        } else {
            self.shader_settings.launch_timeout_ms
        };
        let launch_duration = self
            .remaining()?
            .min(operation.remaining()?)
            .min(Duration::from_millis(milliseconds));
        let launch_budget = Budget::new(launch_duration, self.cancel.as_ref());
        let slot = if purpose == Purpose::Page {
            &mut self.pages
        } else {
            &mut self.shaders
        };
        if slot.is_none() {
            let browser = if let Some(path) = &self.executable {
                crate::runtime::InstalledBrowser {
                    executable: crate::paths::canonical(path)?,
                    version: "external-unverified".into(),
                    platform: crate::runtime::platform()?.into(),
                }
            } else {
                crate::runtime::installed(&self.data)?
            };
            *slot = Some(Browser::launch(&browser, purpose, &launch_budget)?);
        }
        Ok(slot.as_mut().expect("browser initialized"))
    }
    fn remaining(&self) -> Result<Duration> {
        Duration::from_millis(self.settings.render_timeout_ms)
            .checked_sub(self.begun.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| {
                Error::new(
                    "RENDER_TIMEOUT",
                    "Dynamic rendering exceeded its configured render budget",
                )
            })
    }
    fn cache(&mut self, key: String, raster: Raster) -> Raster {
        let capacity = self.settings.cache_mib as usize * 1024 * 1024;
        if self.settings.cache_entries == 0 || capacity == 0 || raster.png.len() > capacity {
            return raster;
        }
        self.cache.remove(&key);
        self.cache_order.retain(|existing| existing != &key);
        let mut total = self
            .cache
            .values()
            .map(|entry| entry.png.len())
            .sum::<usize>();
        while self.cache.len() >= self.settings.cache_entries || total + raster.png.len() > capacity
        {
            let oldest = self.cache_order.pop_front().expect("cache eviction order");
            if let Some(evicted) = self.cache.remove(&oldest) {
                total -= evicted.png.len();
            }
        }
        self.cache_order.push_back(key.clone());
        self.cache.insert(key, raster.clone());
        raster
    }

    fn cached(&mut self, key: &str) -> Option<Raster> {
        let raster = self.cache.get(key)?.clone();
        self.cache_order.retain(|existing| existing != key);
        self.cache_order.push_back(key.into());
        Some(raster)
    }

    pub fn page(
        &mut self,
        node: &ResolvedNode,
        width: f32,
        height: f32,
        scale: f32,
    ) -> Result<Raster> {
        self.check()?;
        let url = node
            .properties
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
            .ok_or_else(|| page_error("Browser node needs a URL"))?;
        validate_url(url)?;
        let zoom = number(node.properties.get("zoom"), 1.0)?;
        if !(0.1..=5.0).contains(&zoom) {
            return Err(page_error("Page zoom must be within 0.1..5"));
        }
        let scroll_x = number(node.properties.get("scrollX"), 0.0)?;
        let scroll_y = number(node.properties.get("scrollY"), 0.0)?;
        if scroll_x < 0.0 || scroll_y < 0.0 {
            return Err(page_error("Page scroll offsets must be nonnegative"));
        }
        let device = node.properties.get("deviceId").and_then(Value::as_str);
        let mobile = match device {
            None | Some("desktop") => false,
            Some("phone" | "tablet") => true,
            _ => {
                return Err(Error::new(
                    "UNSUPPORTED_FEATURE",
                    "Unknown browser deviceId; currently supported IDs are desktop, phone, and tablet",
                ));
            }
        };
        let (pixel_width, pixel_height) =
            dimensions(width as f64 * scale as f64, height as f64 * scale as f64)?;
        let request = json!({"url":url,"width":width,"height":height,"scale":scale,"zoom":zoom,"scroll_x":scroll_x,"scroll_y":scroll_y,"device":device});
        let identity = self.page_identity(node);
        let signature = hash(json!({"url":url,"device":device}).to_string().as_bytes());
        let key = if identity.is_some() {
            hash(
                json!({"request":request,"page":identity})
                    .to_string()
                    .as_bytes(),
            )
        } else {
            hash(request.to_string().as_bytes())
        };
        if !self.refresh
            && let Some(cached) = self.cached(&key)
        {
            return Ok(cached);
        }
        let cancel = self.cancel.clone();
        let limits = json!({"browser":self.browser_settings,"dynamic":self.settings});
        let wait_until = self.browser_settings.wait_until;
        let ready_selector = self.browser_settings.ready_selector.clone();
        let selector_state = self.browser_settings.selector_state;
        let load_condition = if wait_until == PageWaitUntil::NetworkIdle {
            "load, main-frame networkIdle lifecycle"
        } else {
            "load"
        };
        let selector_condition = match (ready_selector.is_empty(), selector_state) {
            (true, _) => "",
            (false, PageSelectorState::Attached) => ", attached CSS selector",
            (false, PageSelectorState::Visible) => ", visible CSS selector",
        };
        let ready_condition = format!(
            "{load_condition}{selector_condition}, fonts, decoded images, two animation frames"
        );
        let duration = self
            .remaining()?
            .min(Duration::from_millis(self.browser_settings.timeout_ms));
        let budget = Budget::new(duration, cancel.as_ref());
        let browser = self.browser(Purpose::Page, &budget)?;
        let version = browser.version.clone();
        let retained = identity.as_deref().map(|key| (key, signature.as_str()));
        let result=browser.with_page(retained,&budget,|browser,session,page_state| {
            browser.command(Some(session),"Page.enable",json!({}),&budget)?;
            if page_state.retained {
                browser.command(Some(session),"Page.bringToFront",json!({}),&budget)?;
            }
            if wait_until == PageWaitUntil::NetworkIdle {
                browser.watch_page_idle(session, &budget)?;
            }
            browser.command(Some(session),"Emulation.setDeviceMetricsOverride",json!({"width":width.ceil().max(1.0) as u32,"height":height.ceil().max(1.0) as u32,"deviceScaleFactor":scale,"mobile":mobile}),&budget)?;
            browser.command(Some(session),"Emulation.setTouchEmulationEnabled",json!({"enabled":mobile}),&budget)?;
            let navigation=if page_state.reused { Value::Null } else { browser.command(Some(session),"Page.navigate",json!({"url":url}),&budget)? };
            if navigation.get("errorText").is_some() { return Err(navigation_error(&navigation)); }
            loop {
                let frame=browser.command(Some(session),"Page.getFrameTree",json!({}),&budget)?;
                let committed=navigation.get("loaderId").is_none_or(|loader| frame["frameTree"]["frame"]["loaderId"]==*loader);
                let idle = wait_until == PageWaitUntil::Load || browser.page_is_idle(session, &frame["frameTree"]["frame"]);
                if committed && idle && browser.evaluate(session,"document.readyState === 'complete'",&budget)?==true { break; }
                std::thread::sleep(Duration::from_millis(20));budget.check()?;
            }
            browser.command(Some(session),"Emulation.setPageScaleFactor",json!({"pageScaleFactor":zoom}),&budget)?;
            browser.wait_for_selector(session, &ready_selector, selector_state, (scroll_x,scroll_y), &budget)?;
            let expression=format!("(async()=>{{await document.fonts.ready;await Promise.all([...document.images].map(image=>image.decode()));window.scrollTo({{left:{scroll_x},top:{scroll_y},behavior:'instant'}});await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));return {{url:window.location.href,scrollX:window.scrollX,scrollY:window.scrollY}};}})()");
            let state=browser.evaluate(session,&expression,&budget)?;
            let screenshot=browser.command(Some(session),"Page.captureScreenshot",json!({"format":"png","fromSurface":true,"captureBeyondViewport":false}),&budget)?;
            let png=base64::engine::general_purpose::STANDARD.decode(screenshot["data"].as_str().ok_or_else(|| page_error("No page image was returned"))?).map_err(|_| page_error("Invalid page PNG encoding"))?;
            let metadata=json!({"kind":"browser","acquisition_id":uuid::Uuid::new_v4().to_string(),"requested_url":redacted_url(url),"url_hash":hash(url.as_bytes()),
                "final_url":state["url"].as_str().map(redacted_url),"browser_version":version,"viewport_width":width,"viewport_height":height,"pixel_width":pixel_width,"pixel_height":pixel_height,
                "scale":scale,"zoom":zoom,"scroll_x":state["scrollX"],"scroll_y":state["scrollY"],"device_id":device,
                "wait_until":wait_until,"ready_selector":ready_selector,"selector_state":selector_state,"ready_condition":ready_condition,"acquired_at":timestamp(),"png_hash":hash(&png),"limits":limits,"page_state":page_state});
            checked_png(png,metadata)
        });
        self.page_used = Some(Instant::now());
        let result = result
            .map_err(|error| budget.check().err().unwrap_or(error))
            .and_then(|raster| {
                budget.check()?;
                self.check()?;
                Ok(raster)
            });
        match result {
            Ok(raster) => Ok(self.cache(key, raster)),
            Err(error) => {
                self.pages.take();
                Err(error)
            }
        }
    }

    pub fn shader(&mut self, request: Value) -> Result<Raster> {
        self.check()?;
        let width = request["width"]
            .as_u64()
            .ok_or_else(|| shader_error("Shader width is missing"))?;
        let height = request["height"]
            .as_u64()
            .ok_or_else(|| shader_error("Shader height is missing"))?;
        dimensions(width as f64, height as f64)?;
        let mut cache_request = request.clone();
        if request["time_dependent"] == false {
            // Uniform/sampler values already capture every used input. The
            // selected timeline is not an input to a time-independent shader.
            cache_request
                .as_object_mut()
                .expect("shader request object")
                .remove("time");
        }
        let key = hash(cache_request.to_string().as_bytes());
        if let Some(cached) = self.cached(&key) {
            return Ok(cached);
        }
        let cancel = self.cancel.clone();
        let limits = json!({"shader":self.shader_settings,"dynamic":self.settings});
        let duration = self
            .remaining()?
            .min(Duration::from_millis(self.shader_settings.timeout_ms));
        let budget = Budget::new(duration, cancel.as_ref());
        let browser = self.browser(Purpose::Shader, &budget)?;
        let version = browser.version.clone();
        let result=browser.with_tab(&budget,|browser,session| {
            browser.command(Some(session),"Network.enable",json!({}),&budget)?;
            browser.command(Some(session),"Network.setBlockedURLs",json!({"urls":["http://*","https://*","file://*","ftp://*"]}),&budget)?;
            let expression=format!("({})({})",include_str!("webgl.js"),request);
            let result=browser.evaluate(session,&expression,&budget)?;
            if let Some(error)=result.get("error").and_then(Value::as_str) { return Err(shader_error(error.chars().take(8192).collect::<String>())); }
            let encoded=result["png"].as_str().and_then(|value| value.strip_prefix("data:image/png;base64,")).ok_or_else(|| shader_error("WebGL returned no PNG"))?;
            let png=base64::engine::general_purpose::STANDARD.decode(encoded).map_err(|_| shader_error("Invalid WebGL PNG"))?;
            let metadata=json!({"kind":"shader","browser_version":version,"renderer":result["renderer"],"extensions":result["extensions"],"width":width,"height":height,"request_hash":key,"png_hash":hash(&png),"time":request["time"],"time_dependent":request["time_dependent"].as_bool().unwrap_or(false),"limits":limits});
            checked_png(png,metadata)
        });
        self.shader_used = Some(Instant::now());
        let result = result
            .map_err(|error| budget.check().err().unwrap_or(error))
            .and_then(|raster| {
                budget.check()?;
                self.check()?;
                Ok(raster)
            });
        match result {
            Ok(raster) => Ok(self.cache(key, raster)),
            Err(error) => {
                self.shaders.take();
                Err(error)
            }
        }
    }
}

pub fn dimensions(width: f64, height: f64) -> Result<(u32, u32)> {
    if !width.is_finite()
        || !height.is_finite()
        || width <= 0.0
        || height <= 0.0
        || width > 8192.0
        || height > 8192.0
        || width.ceil() * height.ceil() > 16.0 * 1024.0 * 1024.0
    {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Dynamic raster exceeds 8192 pixels per side or 16 megapixels",
        ));
    }
    Ok((width.ceil() as u32, height.ceil() as u32))
}
fn checked_png(png: Vec<u8>, mut metadata: Value) -> Result<Raster> {
    if png.len() > 32 * 1024 * 1024 || !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(page_error("Dynamic PNG is invalid or exceeds 32 MiB"));
    }
    let header = png
        .get(12..24)
        .filter(|header| &header[..4] == b"IHDR")
        .ok_or_else(|| page_error("Dynamic PNG has no valid dimension header"))?;
    let width = u32::from_be_bytes(header[4..8].try_into().expect("PNG width"));
    let height = u32::from_be_bytes(header[8..12].try_into().expect("PNG height"));
    dimensions(width as f64, height as f64)?;
    metadata["pixel_width"] = json!(width);
    metadata["pixel_height"] = json!(height);
    Ok(Raster {
        png: png.into(),
        metadata,
    })
}
fn navigation_error(navigation: &Value) -> Error {
    let code = navigation["errorText"].as_str().filter(|code| {
        code.len() <= 128
            && code.strip_prefix("net::ERR_").is_some_and(|suffix| {
                !suffix.is_empty()
                    && suffix.bytes().all(|byte| {
                        byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
                    })
            })
    });
    page_error("Page navigation failed")
        .details(json!({"stage":"page_navigation","browser_error":code}))
}

fn validate_url(value: &str) -> Result<()> {
    let url = url::Url::parse(value).map_err(|_| page_error("Invalid browser URL"))?;
    if !matches!(url.scheme(), "http" | "https" | "data" | "about")
        || url.scheme() == "about" && value != "about:blank"
        || url.scheme() == "data" && !value.starts_with("data:text/html")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(page_error(
            "Browser URLs must use HTTP(S), HTML data, or about:blank without embedded credentials",
        ));
    }
    if value.len() > 1024 * 1024 {
        return Err(page_error("Browser URL exceeds 1 MiB"));
    }
    Ok(())
}
fn redacted_url(value: &str) -> String {
    if value.starts_with("data:") {
        return "data:text/html,[omitted]".into();
    }
    match url::Url::parse(value) {
        Ok(mut url) => {
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.set_query(None);
            url.set_fragment(None);
            url.to_string()
        }
        Err(_) => "[invalid URL]".into(),
    }
}
fn number(value: Option<&Value>, default: f64) -> Result<f64> {
    value
        .map(|value| {
            value
                .as_f64()
                .filter(|value| value.is_finite())
                .ok_or_else(|| page_error("Expected a finite number"))
        })
        .unwrap_or(Ok(default))
}
fn timestamp() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}
fn page_error(message: impl Into<String>) -> Error {
    Error::new("PAGE_LOAD_FAILED", message)
}
fn shader_error(message: impl Into<String>) -> Error {
    Error::new("SHADER_COMPILE_FAILED", message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_failure_exposes_only_a_bounded_network_code() {
        let error = navigation_error(&json!({"errorText":"net::ERR_CONNECTION_RESET"}));
        assert_eq!(error.code, "PAGE_LOAD_FAILED");
        assert_eq!(error.message, "Page navigation failed");
        let details = error.details.unwrap();
        assert_eq!(details["stage"], "page_navigation");
        assert_eq!(details["browser_error"], "net::ERR_CONNECTION_RESET");
        for raw in [
            json!("private-value https://example.test/?token=private-value"),
            json!("net::ERR_PRIVATE_VALUE\nprivate-value"),
            json!(format!("net::ERR_{}", "X".repeat(129))),
            json!("net::ERR_"),
            json!("net::ERR_private_value"),
            json!(42),
            Value::Null,
        ] {
            let error = navigation_error(&json!({"errorText":raw}));
            assert!(error.details.as_ref().unwrap()["browser_error"].is_null());
            assert!(
                !serde_json::to_string(&error)
                    .unwrap()
                    .contains("private-value")
            );
        }
    }

    fn raster(bytes: usize) -> Raster {
        Raster {
            png: vec![0; bytes].into(),
            metadata: json!({}),
        }
    }

    #[test]
    fn captured_cache_policy_enforces_bytes_entries_and_disabling() {
        let mut runtime = DynamicRuntime::new("unused");
        let mut settings = RuntimeSettings::default();
        settings.dynamic.cache_mib = 1;
        settings.dynamic.cache_entries = 2;
        runtime.configure(settings.clone()).unwrap();
        runtime.cache("a".into(), raster(700_000));
        runtime.cache("b".into(), raster(400_000));
        assert!(!runtime.cache.contains_key("a"));
        assert!(runtime.cache.contains_key("b"));
        let large = runtime.cache("oversized".into(), raster(1024 * 1024 + 1));
        assert_eq!(large.png.len(), 1024 * 1024 + 1);
        assert!(!runtime.cache.contains_key("oversized"));
        runtime.cache("c".into(), raster(20));
        assert_eq!(runtime.cache.len(), 2);
        runtime.cache("d".into(), raster(20));
        assert_eq!(runtime.cache.len(), 2);
        assert!(!runtime.cache.contains_key("b"));
        assert!(runtime.cache.contains_key("c"));
        runtime.configure(settings.clone()).unwrap();
        assert!(runtime.cache.contains_key("d"));
        settings.browser.timeout_ms = 1234;
        runtime.configure(settings.clone()).unwrap();
        assert!(runtime.cache.is_empty());
        runtime.cache("e".into(), raster(20));
        let mut invalid = settings.clone();
        invalid.shader.timeout_ms = 0;
        assert_eq!(
            runtime.configure(invalid).unwrap_err().code,
            "CONFIGURATION_ERROR"
        );
        assert!(runtime.cache.contains_key("e"));
        assert_eq!(runtime.shader_settings, settings.shader);
        for (mib, entries) in [(0, 2), (1, 0)] {
            settings.dynamic.cache_mib = mib;
            settings.dynamic.cache_entries = entries;
            runtime.configure(settings.clone()).unwrap();
            assert_eq!(runtime.cache("disabled".into(), raster(20)).png.len(), 20);
            assert!(runtime.cache.is_empty());
        }
        assert!(runtime.process_ids().is_empty());
    }

    #[test]
    fn recently_used_acquisitions_survive_frame_churn_with_bounded_lru_state() {
        let mut runtime = DynamicRuntime::new("unused");
        let mut settings = RuntimeSettings::default();
        settings.dynamic.cache_entries = 3;
        settings.dynamic.cache_mib = 1;
        runtime.configure(settings).unwrap();
        runtime.cache("page".into(), raster(200_000));
        for frame in 0..100 {
            assert!(runtime.cached("page").is_some());
            runtime.cache(format!("frame-{frame}"), raster(400_000));
            assert!(runtime.cache.contains_key("page"));
            assert!(runtime.cache.len() <= 3);
            assert_eq!(runtime.cache_order.len(), runtime.cache.len());
            assert!(
                runtime
                    .cache
                    .values()
                    .map(|raster| raster.png.len())
                    .sum::<usize>()
                    <= 1024 * 1024
            );
        }
        // Replacing a key must not accumulate eviction records or double-count bytes.
        for _ in 0..20 {
            runtime.cache("page".into(), raster(250_000));
        }
        assert_eq!(runtime.cache_order.len(), runtime.cache.len());
        assert!(runtime.cached("page").is_some());
        assert!(
            runtime.idle_deadline().is_none(),
            "Cached reads do not launch or extend runtime processes"
        );
        runtime.begin_render(None, true);
        assert!(runtime.cache.is_empty());
        assert!(runtime.cache_order.is_empty());
    }

    #[test]
    fn render_budget_uses_captured_limit_and_retains_cancellation_precedence() {
        let mut runtime = DynamicRuntime::new("unused");
        let mut settings = RuntimeSettings::default();
        settings.dynamic.render_timeout_ms = 20;
        runtime.configure(settings).unwrap();
        runtime.begun = Instant::now() - Duration::from_millis(50);
        assert_eq!(runtime.check().unwrap_err().code, "RENDER_TIMEOUT");
        let token = CancellationToken::new();
        token.cancel();
        runtime.cancel = Some(token);
        assert_eq!(runtime.check().unwrap_err().code, "CANCELLED");
        runtime.begin_render(None, false);
        runtime.check().unwrap();
        assert!(runtime.idle_deadline().is_none());
    }
}
