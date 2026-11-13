# Browser resources and capture configuration

Read [browser](browser.md) for node properties, refresh, and preview playback. Use this guide to declare local resources or configure capture readiness, network policy, runtime limits, and authenticated profiles.

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

## Network policy and resource limits

Use runtime.browser.network_policy for online (default) or offline web requests. Offline rejects HTTP(S) and WebSockets, including localhost and worker/child-frame requests. Captured local resources, data and about:blank keep their existing access boundaries. Remote main-document navigation fails. Changing the network policy invalidates acquisitions and restarts the page process for fresh and retained pages.

Offline also restricts WebRTC to proxy routes, including its non-proxied UDP path; WebTransport uses the deny proxy. Metadata records network_policy. A project can restrict user online to offline, but cannot relax user offline to online. This policy applies to browser capture and HTTP image acquisition; other applications and AI runtimes use their own network settings.

Captured runtime.browser settings control acquisition/launch deadlines and idle expiry; runtime.dynamic bounds the shared render duration and retained encoded-raster cache. Each acquisition reports its effective limits. Configuration changes invalidate old cached acquisitions, and a zero cache capacity disables reuse. Inspect the configuration and diagnostics when a deadline fails.

## Authenticated profiles

An explicitly created dedicated authentication profile can be selected with runtime.browser.profile in user configuration. Project settings may disable it with an empty string but cannot select one. The user manages login with runtime profile create/login and sets LAZYPENCIL_CHROMIUM to the same full Chrome/Chromium executable for both login and authenticated acquisition.

Matching major versions are insufficient to mix full-browser and headless-shell stores. First use binds the profile to its canonical executable path; another path needs a new profile and login. Documents and agents do not import credentials. These acquisitions use new tabs in the profile's persistent browser context; TUI retention still controls each tab's DOM lifetime. Cookies, localStorage, IndexedDB and Cache Storage remain in private user data. Profile names and credentials are omitted from acquisition metadata, and shaders never use this state. Ordinary anonymous acquisitions keep isolated contexts.
