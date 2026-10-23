//! Browser acquisitions and WebGL rasters, separate from document layout.

use crate::{
    Error, Result,
    cdp::{Browser, Budget, Purpose},
    document::hash,
    project::ResolvedNode,
};
use base64::Engine;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct Raster {
    pub png: Arc<[u8]>,
    pub metadata: Value,
}

pub struct DynamicRuntime {
    data: PathBuf,
    executable: Option<PathBuf>,
    pages: Option<Browser>,
    shaders: Option<Browser>,
    page_used: Option<Instant>,
    shader_used: Option<Instant>,
    cache: BTreeMap<String, Raster>,
    cancel: Option<CancellationToken>,
    begun: Instant,
    refresh: bool,
}

impl DynamicRuntime {
    pub fn new(data: impl Into<PathBuf>) -> Self {
        Self {
            data: data.into(),
            executable: None,
            pages: None,
            shaders: None,
            page_used: None,
            shader_used: None,
            cache: BTreeMap::new(),
            cancel: None,
            begun: Instant::now(),
            refresh: false,
        }
    }
    pub fn with_executable(mut self, path: impl Into<PathBuf>) -> Self {
        self.executable = Some(path.into());
        self
    }
    pub fn begin_render(&mut self, cancel: Option<CancellationToken>, refresh: bool) {
        self.expire_idle(Instant::now());
        self.cancel = cancel;
        self.begun = Instant::now();
        self.refresh = refresh;
        if refresh {
            self.cache.clear();
        }
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
            self.pages.as_ref().and(self.page_used),
            self.shaders.as_ref().and(self.shader_used),
        ]
        .into_iter()
        .flatten()
        .map(|used| used + IDLE_TIMEOUT)
        .min()
    }
    pub fn expire_idle(&mut self, now: Instant) {
        for (slot, used) in [
            (&mut self.pages, &mut self.page_used),
            (&mut self.shaders, &mut self.shader_used),
        ] {
            if slot.is_some() && used.is_some_and(|used| used + IDLE_TIMEOUT <= now) {
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
    fn browser(&mut self, purpose: Purpose) -> Result<&mut Browser> {
        let launch_duration = self.remaining()?.min(Duration::from_secs(15));
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
            *slot = Some(Browser::launch(
                &browser,
                purpose,
                &Budget::new(launch_duration, self.cancel.as_ref()),
            )?);
        }
        Ok(slot.as_mut().expect("browser initialized"))
    }
    fn remaining(&self) -> Result<Duration> {
        Duration::from_secs(45)
            .checked_sub(self.begun.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| {
                Error::new(
                    "RENDER_TIMEOUT",
                    "Dynamic rendering exceeded its 45 second budget",
                )
            })
    }
    fn cache(&mut self, key: String, raster: Raster) -> Raster {
        let total = self
            .cache
            .values()
            .map(|entry| entry.png.len())
            .sum::<usize>();
        if self.cache.len() >= 32 || total + raster.png.len() > 64 * 1024 * 1024 {
            self.cache.clear();
        }
        self.cache.insert(key, raster.clone());
        raster
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
        let key = hash(request.to_string().as_bytes());
        if !self.refresh
            && let Some(cached) = self.cache.get(&key)
        {
            return Ok(cached.clone());
        }
        let cancel = self.cancel.clone();
        let duration = self.remaining()?.min(Duration::from_secs(30));
        let budget = Budget::new(duration, cancel.as_ref());
        let browser = self.browser(Purpose::Page)?;
        let version = browser.version.clone();
        let result=browser.with_tab(&budget,|browser,session| {
            browser.command(Some(session),"Page.enable",json!({}),&budget)?;
            browser.command(Some(session),"Emulation.setDeviceMetricsOverride",json!({"width":width.ceil().max(1.0) as u32,"height":height.ceil().max(1.0) as u32,"deviceScaleFactor":scale,"mobile":mobile}),&budget)?;
            browser.command(Some(session),"Emulation.setTouchEmulationEnabled",json!({"enabled":mobile}),&budget)?;
            let navigation=browser.command(Some(session),"Page.navigate",json!({"url":url}),&budget)?;
            if navigation.get("errorText").is_some() { return Err(page_error("Page navigation failed")); }
            loop {
                let frame=browser.command(Some(session),"Page.getFrameTree",json!({}),&budget)?;
                let committed=navigation.get("loaderId").is_none_or(|loader| frame["frameTree"]["frame"]["loaderId"]==*loader);
                if committed && browser.evaluate(session,"document.readyState === 'complete'",&budget)?==true { break; }
                std::thread::sleep(Duration::from_millis(20));budget.check()?;
            }
            browser.command(Some(session),"Emulation.setPageScaleFactor",json!({"pageScaleFactor":zoom}),&budget)?;
            let expression=format!("(async()=>{{await document.fonts.ready;await Promise.all([...document.images].map(image=>image.decode()));window.scrollTo({scroll_x},{scroll_y});await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));return {{url:location.href,scrollX:scrollX,scrollY:scrollY}};}})()");
            let state=browser.evaluate(session,&expression,&budget)?;
            let screenshot=browser.command(Some(session),"Page.captureScreenshot",json!({"format":"png","fromSurface":true,"captureBeyondViewport":false}),&budget)?;
            let png=base64::engine::general_purpose::STANDARD.decode(screenshot["data"].as_str().ok_or_else(|| page_error("No page image was returned"))?).map_err(|_| page_error("Invalid page PNG encoding"))?;
            let metadata=json!({"kind":"browser","acquisition_id":uuid::Uuid::new_v4().to_string(),"requested_url":redacted_url(url),"url_hash":hash(url.as_bytes()),
                "final_url":state["url"].as_str().map(redacted_url),"browser_version":version,"viewport_width":width,"viewport_height":height,"pixel_width":pixel_width,"pixel_height":pixel_height,
                "scale":scale,"zoom":zoom,"scroll_x":state["scrollX"],"scroll_y":state["scrollY"],"device_id":device,
                "ready_condition":"load, fonts, decoded images, two animation frames","acquired_at":timestamp(),"png_hash":hash(&png)});
            checked_png(png,metadata)
        });
        self.page_used = Some(Instant::now());
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
        let key = hash(request.to_string().as_bytes());
        if let Some(cached) = self.cache.get(&key) {
            return Ok(cached.clone());
        }
        let cancel = self.cancel.clone();
        let duration = self.remaining()?.min(Duration::from_secs(15));
        let budget = Budget::new(duration, cancel.as_ref());
        let browser = self.browser(Purpose::Shader)?;
        let version = browser.version.clone();
        let result=browser.with_tab(&budget,|browser,session| {
            browser.command(Some(session),"Network.enable",json!({}),&budget)?;
            browser.command(Some(session),"Network.setBlockedURLs",json!({"urls":["http://*","https://*","file://*","ftp://*"]}),&budget)?;
            let expression=format!("({})({})",include_str!("webgl.js"),request);
            let result=browser.evaluate(session,&expression,&budget)?;
            if let Some(error)=result.get("error").and_then(Value::as_str) { return Err(shader_error(error.chars().take(8192).collect::<String>())); }
            let encoded=result["png"].as_str().and_then(|value| value.strip_prefix("data:image/png;base64,")).ok_or_else(|| shader_error("WebGL returned no PNG"))?;
            let png=base64::engine::general_purpose::STANDARD.decode(encoded).map_err(|_| shader_error("Invalid WebGL PNG"))?;
            let metadata=json!({"kind":"shader","browser_version":version,"renderer":result["renderer"],"extensions":result["extensions"],"width":width,"height":height,"request_hash":key,"png_hash":hash(&png),"time":request["time"]});
            checked_png(png,metadata)
        });
        self.shader_used = Some(Instant::now());
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
