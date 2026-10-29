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
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
pub(crate) mod acquisition;
mod pages;
mod preprocessor;
mod shader;
use acquisition::PageAcquisition;
use pages::{PageInput, page_url};

#[derive(Clone)]
pub struct Raster {
    pub png: Arc<[u8]>,
    pub metadata: Value,
}

#[derive(Clone)]
enum Cached {
    Raster(Raster),
    ShaderSource(Arc<str>),
}

impl Cached {
    fn bytes(&self) -> usize {
        match self {
            Self::Raster(raster) => raster.png.len(),
            Self::ShaderSource(source) => source.len(),
        }
    }
}

pub struct DynamicRuntime {
    data: PathBuf,
    executable: Option<PathBuf>,
    supervisor: Option<PathBuf>,
    page_scope: Option<String>,
    page_snapshot: Option<String>,
    pages: Option<Browser>,
    #[cfg(unix)]
    shared_pages: Option<crate::runtime::profiles::pages::Client>,
    shaders: Option<Browser>,
    page_used: Option<Instant>,
    shader_used: Option<Instant>,
    cache: BTreeMap<String, Cached>,
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
            supervisor: None,
            page_scope: None,
            page_snapshot: None,
            pages: None,
            #[cfg(unix)]
            shared_pages: None,
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
        self.shaders.take();
        self.pages.take();
        #[cfg(unix)]
        self.shared_pages.take();
        self.page_used = None;
        self.shader_used = None;
        self.cache.clear();
        self.cache_order.clear();
        self.executable = Some(path.into());
        self
    }
    /// Bind browser workers to the actual CLI when using the library from another host.
    pub fn with_supervisor_executable(mut self, path: impl Into<PathBuf>) -> Self {
        self.supervisor = Some(path.into());
        self
    }
    /// Opt into in-memory page state for this document. Ordinary CLI/MCP
    /// runtimes retain their fresh-context acquisition behavior.
    pub fn with_retained_pages(mut self, document: &Path) -> Self {
        let scope = hash(document.as_os_str().as_encoded_bytes());
        if self.page_scope.as_ref() != Some(&scope) {
            self.pages.take();
            #[cfg(unix)]
            self.shared_pages.take();
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

    pub(crate) fn prepare_pages(
        &mut self,
        project: &crate::project::Project,
        snapshot: &crate::layout::LayoutSnapshot,
    ) -> Result<()> {
        let result = self.prepare_page_inputs(project, snapshot);
        self.page_result(result)
    }

    fn prepare_page_inputs(
        &mut self,
        project: &crate::project::Project,
        snapshot: &crate::layout::LayoutSnapshot,
    ) -> Result<()> {
        if self.page_scope.is_none() || self.page_snapshot.as_deref() == Some(snapshot.id.as_str())
        {
            return Ok(());
        }
        // Changed inputs must not resurrect an old URL's acquisition after a
        // node was removed, edited and later restored. Viewport-only requests
        // still reuse the same captured layout and cache.
        self.cache.clear();
        self.cache_order.clear();
        if self.has_pages() {
            let mut signatures = BTreeMap::new();
            for item in snapshot
                .items
                .iter()
                .filter(|item| item.record.enabled && item.node.kind() == "browser")
            {
                self.check()?;
                if let Some(key) = self.page_identity(&item.node) {
                    match PageInput::capture(Some(project), &item.node)
                        .and_then(|input| input.signature(&item.node))
                    {
                        Ok(signature) => {
                            signatures.insert(key, signature);
                        }
                        Err(error) if error.code == "CANCELLED" => return Err(error),
                        // Retire an invalid offscreen page now; the normal visible
                        // acquisition reports its own error if it is requested.
                        Err(_) => {}
                    }
                }
            }
            let cancel = self.cancel.clone();
            let budget = Budget::new(self.remaining()?, cancel.as_ref());
            let result = match self.pages.as_mut() {
                Some(browser) => browser.retain_page_signatures(&signatures, &budget),
                None => Ok(()),
            };
            result.map_err(|error| budget.check().err().unwrap_or(error))?;
            #[cfg(unix)]
            if let Some(client) = self.shared_pages.as_mut() {
                client.retain(&signatures, &budget)?;
            }
        }
        self.page_snapshot = Some(snapshot.id.clone());
        Ok(())
    }
    /// Apply captured data-only limits without launching a process.
    pub fn configure(&mut self, settings: RuntimeSettings) -> Result<()> {
        settings.validate()?;
        if (self.page_scope.is_some() && self.browser_settings != settings.browser)
            || self.browser_settings.network_policy != settings.browser.network_policy
            || self.browser_settings.profile != settings.browser.profile
        {
            self.pages.take();
            #[cfg(unix)]
            self.shared_pages.take();
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
        self.cache.retain(|_, entry| {
            !matches!(entry, Cached::Raster(raster) if raster.metadata["kind"] == "browser")
        });
        let cache = &self.cache;
        self.cache_order.retain(|key| cache.contains_key(key));
    }
    /// The owned Chromium child IDs, for runtime diagnostics.
    pub fn process_ids(&self) -> Vec<u32> {
        let ids = [self.pages.as_ref(), self.shaders.as_ref()]
            .into_iter()
            .flatten()
            .map(Browser::process_id)
            .collect::<Vec<_>>();
        #[cfg(unix)]
        let ids = ids
            .into_iter()
            .chain(
                self.shared_pages
                    .as_ref()
                    .and_then(|client| client.process_id()),
            )
            .collect();
        ids
    }
    /// Number of live page contexts retained by this runtime for diagnostics.
    pub fn retained_page_count(&self) -> usize {
        let count = self.pages.as_ref().map_or(0, Browser::retained_page_count);
        #[cfg(unix)]
        let count = count
            + self
                .shared_pages
                .as_ref()
                .map_or(0, |client| client.retained_count());
        count
    }
    fn has_pages(&self) -> bool {
        let present = self.pages.is_some();
        #[cfg(unix)]
        let present = present || self.shared_pages.is_some();
        present
    }
    /// Page and shader processes expire independently; cached rasters remain valid.
    pub fn idle_deadline(&self) -> Option<Instant> {
        [
            (
                self.has_pages().then_some(self.page_used).flatten(),
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
        #[cfg(unix)]
        if self.shared_pages.is_some()
            && self.page_used.is_some_and(|used| {
                used + Duration::from_secs(self.browser_settings.idle_seconds) <= now
            })
        {
            self.shared_pages.take();
            self.page_used = None;
        }
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
            let profile = if purpose == Purpose::Page && !self.browser_settings.profile.is_empty() {
                Some(crate::runtime::profiles::Profile::open(
                    &self.data,
                    &self.browser_settings.profile,
                )?)
            } else {
                None
            };
            *slot = Some(Browser::launch_for_runtime(
                &browser,
                purpose,
                self.browser_settings.network_policy,
                &launch_budget,
                self.supervisor.as_deref(),
                profile.as_ref(),
            )?);
        }
        Ok(slot.as_mut().expect("browser initialized"))
    }
    #[cfg(unix)]
    fn shared_page_client(
        &mut self,
        operation: &Budget<'_>,
    ) -> Result<&mut crate::runtime::profiles::pages::Client> {
        if self.shared_pages.is_none() {
            let duration =
                self.remaining()?
                    .min(operation.remaining()?)
                    .min(Duration::from_millis(
                        self.browser_settings.launch_timeout_ms,
                    ));
            let budget = Budget::new(duration, self.cancel.as_ref());
            let browser = if let Some(path) = &self.executable {
                crate::runtime::InstalledBrowser {
                    executable: crate::paths::canonical(path)?,
                    version: "external-unverified".into(),
                    platform: crate::runtime::platform()?.into(),
                }
            } else {
                crate::runtime::installed(&self.data)?
            };
            let profile = crate::runtime::profiles::Profile::open(
                &self.data,
                &self.browser_settings.profile,
            )?;
            self.shared_pages = Some(crate::runtime::profiles::pages::Client::connect(
                &profile,
                &browser,
                self.supervisor.as_deref(),
                &budget,
            )?);
        }
        Ok(self
            .shared_pages
            .as_mut()
            .expect("profile client initialized"))
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
        self.cache_entry(key, Cached::Raster(raster.clone()));
        raster
    }

    fn cache_entry(&mut self, key: String, entry: Cached) {
        let capacity = self.settings.cache_mib as usize * 1024 * 1024;
        if self.settings.cache_entries == 0 || capacity == 0 || entry.bytes() > capacity {
            return;
        }
        self.cache.remove(&key);
        self.cache_order.retain(|existing| existing != &key);
        let mut total = self.cache.values().map(Cached::bytes).sum::<usize>();
        while self.cache.len() >= self.settings.cache_entries || total + entry.bytes() > capacity {
            let oldest = self.cache_order.pop_front().expect("cache eviction order");
            if let Some(evicted) = self.cache.remove(&oldest) {
                total -= evicted.bytes();
            }
        }
        self.cache_order.push_back(key.clone());
        self.cache.insert(key, entry);
    }

    fn cached(&mut self, key: &str) -> Option<Raster> {
        match self.cached_entry(key)? {
            Cached::Raster(raster) => Some(raster),
            Cached::ShaderSource(_) => None,
        }
    }

    fn cached_entry(&mut self, key: &str) -> Option<Cached> {
        let entry = self.cache.get(key)?.clone();
        self.cache_order.retain(|existing| existing != key);
        self.cache_order.push_back(key.into());
        Some(entry)
    }

    pub fn page(
        &mut self,
        node: &ResolvedNode,
        width: f32,
        height: f32,
        scale: f32,
    ) -> Result<Raster> {
        let result = (|| {
            self.check()?;
            let input = PageInput::capture(None, node)?;
            self.page_with_input(node, width, height, scale, input)
        })();
        self.page_result(result)
    }

    fn page_with_input(
        &mut self,
        node: &ResolvedNode,
        width: f32,
        height: f32,
        scale: f32,
        input: PageInput,
    ) -> Result<Raster> {
        self.check()?;
        let url = page_url(node)?;
        crate::cdp::check_network_navigation(self.browser_settings.network_policy, url)?;
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
        match device {
            None | Some("desktop" | "phone" | "tablet") => {}
            _ => {
                return Err(Error::new(
                    "UNSUPPORTED_FEATURE",
                    "Unknown browser deviceId; currently supported IDs are desktop, phone, and tablet",
                ));
            }
        };
        dimensions(width as f64 * scale as f64, height as f64 * scale as f64)?;
        let request = json!({"url":url,"width":width,"height":height,"scale":scale,"zoom":zoom,"scroll_x":scroll_x,"scroll_y":scroll_y,"device":device,"resources":input.identity});
        let identity = self.page_identity(node);
        let signature = input.signature(node)?;
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
        let request = PageAcquisition {
            url: url.to_owned(),
            width,
            height,
            scale,
            zoom,
            scroll_x,
            scroll_y,
            device: device.map(str::to_owned),
            settings: self.browser_settings.clone(),
            dynamic: self.settings,
            retained: identity.map(|key| (key, signature)),
            resources: input.resources.map(|resources| resources.files),
            resource_identity: input.identity,
        };
        let cancel = self.cancel.clone();
        let duration = self
            .remaining()?
            .min(Duration::from_millis(self.browser_settings.timeout_ms));
        let budget = Budget::new(duration, cancel.as_ref());
        #[cfg(unix)]
        let result = if !self.browser_settings.profile.is_empty() {
            self.shared_page_client(&budget)?.page(request, &budget)
        } else {
            request.acquire(self.browser(Purpose::Page, &budget)?, &budget)
        };
        #[cfg(not(unix))]
        let result = request.acquire(self.browser(Purpose::Page, &budget)?, &budget);
        self.page_used = Some(Instant::now());
        let result = result
            .map_err(|error| budget.check().err().unwrap_or(error))
            .and_then(|raster| {
                budget.check()?;
                self.check()?;
                Ok(raster)
            });
        result.map(|raster| self.cache(key, raster))
    }

    pub fn shader(&mut self, request: Value) -> Result<Raster> {
        self.check()?;
        let cancel = self.cancel.clone();
        let duration = self
            .remaining()?
            .min(Duration::from_millis(self.shader_settings.timeout_ms));
        let budget = Budget::new(duration, cancel.as_ref());
        self.shader_with_budget(request, &budget)
    }

    fn shader_with_budget(&mut self, request: Value, budget: &Budget<'_>) -> Result<Raster> {
        self.check()?;
        budget.check()?;
        let source = request["source"]
            .as_str()
            .ok_or_else(|| shader_error("Shader source is missing"))?;
        if source.len() > 1024 * 1024 {
            return Err(shader_error("Shader source exceeds 1 MiB"));
        }
        let source_hash = hash(source.as_bytes());
        let width = request["width"]
            .as_u64()
            .ok_or_else(|| shader_error("Shader width is missing"))?;
        let height = request["height"]
            .as_u64()
            .ok_or_else(|| shader_error("Shader height is missing"))?;
        dimensions(width as f64, height as f64)?;
        let mut cache_request = request.clone();
        cache_request["preprocessor_hash"] = json!(preprocessor::hash());
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
        let limits = json!({"shader":self.shader_settings,"dynamic":self.settings});
        let browser = self.browser(Purpose::Shader, budget)?;
        let version = browser.version.clone();
        let result=browser.with_tab(budget,|browser,session| {
            browser.command(Some(session),"Network.enable",json!({}),budget)?;
            browser.command(Some(session),"Network.setBlockedURLs",json!({"urls":["http://*","https://*","file://*","ftp://*"]}),budget)?;
            let expression=format!("({})({},{})",include_str!("webgl.js"),request,json!(preprocessor::encoded()));
            let result=browser.evaluate(session,&expression,budget)?;
            if let Some(error)=result.get("error").and_then(Value::as_str) { return Err(shader_error(error.chars().take(8192).collect::<String>())); }
            let encoded=result["png"].as_str().and_then(|value| value.strip_prefix("data:image/png;base64,")).ok_or_else(|| shader_error("WebGL returned no PNG"))?;
            let png=base64::engine::general_purpose::STANDARD.decode(encoded).map_err(|_| shader_error("Invalid WebGL PNG"))?;
            let metadata=json!({"kind":"shader","browser_version":version,"renderer":result["renderer"],"extensions":result["extensions"],"width":width,"height":height,"request_hash":key,"png_hash":hash(&png),"source_hash":source_hash,"coordinate_size":result["coordinate_size"],"preprocessor_hash":preprocessor::hash(),"time":request["time"],"time_dependent":request["time_dependent"].as_bool().unwrap_or(false),"limits":limits});
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
pub(crate) fn checked_png(png: Vec<u8>, mut metadata: Value) -> Result<Raster> {
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
            assert!(runtime.cache.values().map(Cached::bytes).sum::<usize>() <= 1024 * 1024);
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
