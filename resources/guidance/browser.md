# Browser snapshots

Use a browser node when the design needs a rendered web page. Ordinary frames and text continue to use native layout and Skia. Browser rendering needs a separately installed Chromium runtime. A missing runtime is an error with a setup command; rendering never downloads it automatically.

Set url, width, and height. Optional zoom, scrollX, and scrollY control the captured view. Current device IDs are desktop, phone, and tablet; other Pencil preset IDs still require compatibility work. HTTP(S), HTML data URLs, and about:blank are supported. Embedded URL credentials and arbitrary file URLs are rejected. Local test pages can be served over loopback HTTP.

The current capture waits for the document load, web fonts, decoded images, and two animation frames within a deadline. A page may still change after that point. Inspect the returned image and acquisition metadata rather than assuming a URL determines permanent pixels. Metadata redacts URL queries, fragments, credentials, and HTML data contents.

Render reuses an in-memory page raster until refresh is requested or the cache evicts it. Each new acquisition uses a fresh isolated browser context. It does not currently retain an interactive page session between refreshes. Use refresh: true in the render tool to obtain a new acquisition. The result records its ID, timestamp, browser version, dimensions, zoom, scroll, scale, and PNG hash. A calculation snapshot and a page acquisition have different identities.

The captured image receives the node's transform, opacity, clipping, border, and supported effects. Browser JavaScript and CSS are evaluated by Chromium. WebGL shader jobs use a separate process. Do not invent a result when navigation, decoding, runtime startup, or a deadline fails. Persistent authenticated profiles, configurable network and waiting policies, complete Pencil device-preset compatibility, and TUI lifecycle integration remain unfinished.
