//! An owned page acquisition, independent of a frontend or document worker.
use super::*;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PageAcquisition {
    pub url: String,
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    pub zoom: f64,
    pub scroll_x: f64,
    pub scroll_y: f64,
    pub device: Option<String>,
    pub settings: BrowserSettings,
    pub dynamic: DynamicSettings,
    pub retained: Option<(String, String)>,
    #[serde(skip)]
    pub resources: Option<Arc<crate::cdp::LocalFiles>>,
    pub resource_identity: Option<Value>,
}

impl PageAcquisition {
    pub(crate) fn validate(&self) -> Result<bool> {
        self.settings.validate()?;
        self.dynamic.validate()?;
        if self.url.len() > 1024 * 1024 {
            return Err(page_error("Browser URL exceeds 1 MiB"));
        }
        if self.resources.is_some() {
            crate::cdp::local_file_url(&self.url)?;
        } else {
            validate_url(&self.url)?;
        }
        if self.resources.is_some() != self.resource_identity.is_some() {
            return Err(page_error("Local browser resource identity is missing"));
        }
        crate::cdp::check_network_navigation(self.settings.network_policy, &self.url)?;
        dimensions(
            self.width as f64 * self.scale as f64,
            self.height as f64 * self.scale as f64,
        )?;
        if !(0.1..=5.0).contains(&self.zoom)
            || !self.scroll_x.is_finite()
            || !self.scroll_y.is_finite()
            || self.scroll_x < 0.0
            || self.scroll_y < 0.0
        {
            return Err(page_error("Invalid page zoom or scroll offsets"));
        }
        let mobile = match self.device.as_deref() {
            None | Some("desktop") => false,
            Some("phone" | "tablet") => true,
            _ => {
                return Err(Error::new(
                    "UNSUPPORTED_FEATURE",
                    "Unknown browser deviceId; currently supported IDs are desktop, phone, and tablet",
                ));
            }
        };
        Ok(mobile)
    }

    pub(crate) fn acquire(&self, browser: &mut Browser, budget: &Budget<'_>) -> Result<Raster> {
        let mobile = self.validate()?;
        let (pixel_width, pixel_height) = dimensions(
            self.width as f64 * self.scale as f64,
            self.height as f64 * self.scale as f64,
        )?;
        let settings = &self.settings;
        let load = if settings.wait_until == PageWaitUntil::NetworkIdle {
            "load, main-frame networkIdle lifecycle"
        } else {
            "load"
        };
        let selector = match (settings.ready_selector.is_empty(), settings.selector_state) {
            (true, _) => "",
            (false, PageSelectorState::Attached) => ", attached CSS selector",
            (false, PageSelectorState::Visible) => ", visible CSS selector",
        };
        let additional = if settings.additional_wait_ms == 0 {
            String::new()
        } else {
            format!(", additional wait {}ms", settings.additional_wait_ms)
        };
        let animation =
            if settings.animation_policy == crate::config::runtime::PageAnimationPolicy::Paused {
                ", paused document animations"
            } else {
                ""
            };
        let ready_condition = format!(
            "{load}{selector}{additional}{animation}, fonts, decoded images, two animation frames"
        );
        let retained = self
            .retained
            .as_ref()
            .map(|(key, signature)| (key.as_str(), signature.as_str()));
        let version = browser.version.clone();
        browser.with_page(retained, budget, |browser, session, page_state| {
            browser.command(Some(session), "Page.enable", json!({}), budget)?;
            if !page_state.reused && let Some(resources) = &self.resources {
                browser.intercept_local_files(session, resources.clone(), budget)?;
            }
            if page_state.retained { browser.command(Some(session), "Page.bringToFront", json!({}), budget)?; }
            if settings.wait_until == PageWaitUntil::NetworkIdle { browser.watch_page_idle(session, budget)?; }
            browser.command(Some(session), "Emulation.setDeviceMetricsOverride", json!({"width":self.width.ceil().max(1.0) as u32,"height":self.height.ceil().max(1.0) as u32,"deviceScaleFactor":self.scale,"mobile":mobile}), budget)?;
            browser.command(Some(session), "Emulation.setTouchEmulationEnabled", json!({"enabled":mobile}), budget)?;
            let navigation = if page_state.reused { Value::Null } else { browser.command(Some(session), "Page.navigate", json!({"url":self.url}), budget)? };
            if navigation.get("errorText").is_some() { return Err(navigation_error(&navigation)); }
            loop {
                let frame = browser.command(Some(session), "Page.getFrameTree", json!({}), budget)?;
                crate::cdp::check_network_frame(settings.network_policy, &frame["frameTree"]["frame"])?;
                let committed = navigation.get("loaderId").is_none_or(|loader| frame["frameTree"]["frame"]["loaderId"] == *loader);
                let idle = settings.wait_until == PageWaitUntil::Load || browser.page_is_idle(session, &frame["frameTree"]["frame"]);
                if committed && idle && browser.evaluate(session, "document.readyState === 'complete'", budget)? == true { break; }
                std::thread::sleep(Duration::from_millis(20)); budget.check()?;
            }
            browser.command(Some(session), "Emulation.setPageScaleFactor", json!({"pageScaleFactor":self.zoom}), budget)?;
            browser.wait_for_selector(session, &settings.ready_selector, settings.selector_state, (self.scroll_x,self.scroll_y), budget)?;
            browser.wait_before_capture(session, Duration::from_millis(settings.additional_wait_ms), budget)?;
            if settings.animation_policy == crate::config::runtime::PageAnimationPolicy::Paused { browser.set_page_animation_policy(session, settings.animation_policy, budget)?; }
            let expression = format!("(async()=>{{await document.fonts.ready;await Promise.all([...document.images].map(image=>image.decode()));window.scrollTo({{left:{},top:{},behavior:'instant'}});await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));return {{url:window.location.href,scrollX:window.scrollX,scrollY:window.scrollY}};}})()", self.scroll_x, self.scroll_y);
            let state = browser.evaluate(session, &expression, budget)?;
            if let Some(url) = state["url"].as_str() { crate::cdp::check_network_navigation(settings.network_policy,url)?; }
            let screenshot = browser.command(Some(session), "Page.captureScreenshot", json!({"format":"png","fromSurface":true,"captureBeyondViewport":false}), budget)?;
            let png = base64::engine::general_purpose::STANDARD.decode(screenshot["data"].as_str().ok_or_else(|| page_error("No page image was returned"))?).map_err(|_| page_error("Invalid page PNG encoding"))?;
            let mut metadata = json!({"kind":"browser","acquisition_id":uuid::Uuid::new_v4().to_string(),"requested_url":redacted_url(&self.url),"url_hash":hash(self.url.as_bytes()),
                "final_url":state["url"].as_str().map(redacted_url),"browser_version":version,"viewport_width":self.width,"viewport_height":self.height,"pixel_width":pixel_width,"pixel_height":pixel_height,
                "scale":self.scale,"zoom":self.zoom,"scroll_x":state["scrollX"],"scroll_y":state["scrollY"],"device_id":self.device,
                "wait_until":settings.wait_until,"ready_selector":settings.ready_selector,"selector_state":settings.selector_state,"additional_wait_ms":settings.additional_wait_ms,"animation_policy":settings.animation_policy,"network_policy":settings.network_policy,"ready_condition":ready_condition,"acquired_at":timestamp(),"png_hash":hash(&png),"limits":{"browser":self.settings,"dynamic":self.dynamic},"page_state":page_state});
            if let Some(identity) = &self.resource_identity {
                metadata["resources"] = identity.clone();
                metadata["local_files"] = serde_json::to_value(browser.local_file_stats(session).ok_or_else(|| page_error("Local browser resource session is missing"))?).expect("local resource statistics");
            }
            checked_png(png, metadata)
        })
    }
}
