# Browser snapshots

Use a browser node when the design needs a rendered web page. Ordinary frames and text continue to use native layout and Skia. Browser rendering needs a separately installed Chromium runtime. A missing runtime is an error with a setup command; rendering never downloads it automatically.

## Define the captured view

Set url, width, and height. Optional zoom, scrollX, and scrollY control the captured view. Current device IDs are desktop, phone, and tablet; other preset IDs return UNSUPPORTED_FEATURE. HTTP(S), HTML data URLs, and about:blank are supported. Embedded URL credentials and arbitrary file URLs are rejected. Local test pages can be served over loopback HTTP.

## Readiness and local pages

By default, acquisition waits for document load, web fonts, decoded images, and two animation frames. A loaded page may still have ongoing requests or animations; inspect the intended state.

Read [browser-config](browser-config.md) through get_guidance when a local page needs declared resource roots, when capture readiness needs adjustment, or when using network policy, runtime limits, or an authenticated profile.

## Refresh, retained state, and playback

The CLI/MCP render path reuses an in-memory page raster until refresh is requested or the cache evicts it. Use refresh: true in render for a new acquisition. Without a dedicated profile, each new acquisition uses a fresh isolated browser context. TUI page sessions can retain DOM state across refreshes.

The result records an acquisition ID, timestamp, browser version, dimensions, zoom, scroll, scale, and PNG hash. Acquisition and calculation IDs identify different things. page_state records retained, id, reused, and retained_limit at acquisition time, including on cached images; it does not establish that the page is still alive. URL/device changes, eviction, idle expiry, failure, document rebinding, or browser-policy changes may require a new state. Resource changes retire the old DOM even outside the viewport; dimensions alone preserve DOM. Metadata redacts URL queries, fragments, credentials, and HTML data contents.

TUI Preview uses p to start or pause playback. Preview · Play… selects 1–10 fps (default 5). Playback periodically captures retained DOM; slow rendering drops frames instead of queuing work. One requested frame may finish after pausing. Paused dependency polls reuse pixels, and idle expiry still applies. Frames reuse unchanged layouts and time-independent shader rasters.

Page JavaScript and CSS use real browser time. Preview · Seek shader time… changes shader uniforms without rewinding the page or running document scripts on a clock. Browser-only documents support playback.

## Compositing and verification

The captured image receives the node's transform, opacity, clipping, border, and supported effects. Chromium evaluates page JavaScript and CSS; shader jobs use a separate process. Navigation, decoding, startup, and deadline failures return errors. Inspect the requested page state and follow `verification` for reporting unavailable inputs.
