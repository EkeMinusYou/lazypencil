# Browser snapshots

Use a browser node when the design needs a rendered web page. Ordinary frames and text continue to use native layout and Skia. Browser rendering needs a separately installed Chromium runtime. A missing runtime is an error with a setup command; rendering never downloads it automatically.

## Define the captured view

Set url, width, and height. Optional zoom, scrollX, and scrollY control the captured view. Current device IDs are desktop, phone, and tablet; other preset IDs return UNSUPPORTED_FEATURE. HTTP(S), HTML data URLs, and about:blank are supported. Embedded URL credentials and arbitrary file URLs are rejected. Local test pages can be served over loopback HTTP.

## Capture local resources

For a local page, declare its resource directory with an edit operation such as {"op":"set_browser_resources","roots":[{"url":"file:///original/site/","path":"web"}]}. The original file directory URL must be canonical and end in a slash; path is relative to this document. This declaration uses the existing project read boundaries and cannot grant access outside them or into lazypencil's private data. A file browser node can then use file:///original/site/index.html?theme=dark#top. URL text is literal, including dollar signs. Only captured files from the owning document's declared namespace are served; unlisted file requests are denied. HTML, CSS and JavaScript retain their original URLs. Use {"op":"pack_browser_resources"} to save declared inputs into portable assets, and carry those assets with the .pen document.

Setting roots to [] removes this document's declaration. These declarations and copy-context records use lazypencil metadata.

A captured namespace serves only declared resources. File metadata, range/media loading, worklets, popups, and worker bootstrap may differ from a directly opened local site; inspect the requested behavior when it depends on these features.

## Configure readiness

Use get_document with include_configuration:true to inspect configuration.settings.runtime.browser. The following capture preferences are host settings, not extra node properties. Configure them for the requested capture behavior.

Capture waits for document load, web fonts, decoded images, and two animation frames within a deadline. The host setting runtime.browser.wait_until defaults to load; network_idle additionally requires Chromium's networkIdle lifecycle notification for the current main-frame loader. It uses the same acquisition/render deadline. Ongoing requests can therefore time out instead of producing an early image. Neither policy guarantees settled application state, future requests, animations, workers, or every cross-origin frame. Inspect the image and acquisition metadata to confirm the intended state.

Use runtime.browser.ready_selector for a CSS readiness marker such as #app[data-state='ready']. An empty string disables it, including clearing an inherited selector. It applies to every browser acquisition using that configuration. selector_state defaults to visible; attached accepts the first matching main-document element even when hidden. Visible requires positive-area layout and Chromium visibility checks, including opacity zero and skipped content. It does not guarantee absence of occlusion or viewport intersection, and does not enter iframes or shadow roots.

Zoom and instant scroll are applied before these checks and repeated as content appears. Selector syntax errors fail immediately; missing/hidden targets use the existing deadline and cancellation. Metadata records the selector and state.

Use runtime.browser.additional_wait_ms for an extra 0–60,000 ms after load/selector readiness and before final font/image preparation. It defaults to zero and shares the acquisition/render deadline and cancellation. The host measures the interval, so page timers cannot shorten it. It does not predict later updates. Explicit zero clears an inherited wait; cached acquisitions do not wait again.

Set runtime.browser.animation_policy to paused to pause native document animations after readiness and the extra wait. The default running preserves normal playback. CSS animations, transitions and Web Animations use Chromium's document timeline; this is not a clock freeze for JavaScript timers, video, animated images or cross-process frames, and it does not seek to a fixed timestamp. An unchanged retained context stays paused on refresh. Policy changes invalidate acquisitions and can replace retained contexts; metadata records animation_policy. The settings inherit per field.

## Refresh, retained state, and playback

The CLI/MCP render path reuses an in-memory page raster until refresh is requested or the cache evicts it. Use refresh: true in render for a new acquisition. Without a dedicated profile, each new acquisition uses a fresh isolated browser context. TUI page sessions can retain DOM state across refreshes.

The result records an acquisition ID, timestamp, browser version, dimensions, zoom, scroll, scale, and PNG hash. Acquisition and calculation IDs identify different things. page_state records retained, id, reused, and retained_limit at acquisition time, including on cached images; it does not establish that the page is still alive. URL/device changes, eviction, idle expiry, failure, document rebinding, or browser-policy changes may require a new state. Resource changes retire the old DOM even outside the viewport; dimensions alone preserve DOM. Metadata redacts URL queries, fragments, credentials, and HTML data contents.

TUI Preview uses p to start or pause playback. Preview · Play… selects 1–10 fps (default 5). Playback periodically captures retained DOM; slow rendering drops frames instead of queuing work. One requested frame may finish after pausing. Paused dependency polls reuse pixels, and idle expiry still applies. Frames reuse unchanged layouts and time-independent shader rasters.

Page JavaScript and CSS use real browser time. Preview · Seek shader time… changes shader uniforms without rewinding the page or running document scripts on a clock. Browser-only documents support playback.

## Network policy and resource limits

Use runtime.browser.network_policy for online (default) or offline web requests. Offline rejects HTTP(S) and WebSockets, including localhost and worker/child-frame requests. Captured local resources, data and about:blank keep their existing access boundaries. Remote main-document navigation fails. Changing the network policy invalidates acquisitions and restarts the page process for fresh and retained pages.

Offline also restricts WebRTC to proxy routes, including its non-proxied UDP path; WebTransport uses the deny proxy. Metadata records network_policy. A project can restrict user online to offline, but cannot relax user offline to online. This policy applies to browser capture and HTTP image acquisition; other applications and AI runtimes use their own network settings.

Captured runtime.browser settings control acquisition/launch deadlines and idle expiry; runtime.dynamic bounds the shared render duration and retained encoded-raster cache. Each acquisition reports its effective limits. Configuration changes invalidate old cached acquisitions, and a zero cache capacity disables reuse. Inspect the configuration and diagnostics when a deadline fails.

## Authenticated profiles

An explicitly created dedicated authentication profile can be selected with runtime.browser.profile in user configuration. Project settings may disable it with an empty string but cannot select one. The user manages login with runtime profile create/login and sets LAZYPENCIL_CHROMIUM to the same full Chrome/Chromium executable for both login and authenticated acquisition.

Matching major versions are insufficient to mix full-browser and headless-shell stores. First use binds the profile to its canonical executable path; another path needs a new profile and login. Documents and agents do not import credentials. These acquisitions use new tabs in the profile's persistent browser context; TUI retention still controls each tab's DOM lifetime. Cookies, localStorage, IndexedDB and Cache Storage remain in private user data. Profile names and credentials are omitted from acquisition metadata, and shaders never use this state. Ordinary anonymous acquisitions keep isolated contexts.

## Compositing and verification

The captured image receives the node's transform, opacity, clipping, border, and supported effects. Chromium evaluates page JavaScript and CSS; shader jobs use a separate process. Navigation, decoding, startup, and deadline failures return errors. Inspect the requested page state and follow `verification` for reporting unavailable inputs.
