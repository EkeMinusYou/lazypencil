//! Exact completed native frames, invalidated with the captured project.
use super::*;

// Two raw HiDPI viewports must fit alongside prepared compressed neighbors.
const MAX_BYTES: usize = 256 * 1024 * 1024;
const MAX_ENTRIES: usize = 64;
const MAX_RECENT: usize = 2;

struct Entry {
    cells: Size,
    background: image::Rgba<u8>,
    preview: Preview,
    bytes: usize,
}

pub(super) struct Frames {
    entries: Vec<Entry>,
    bytes: usize,
    font: ratatui_image::FontSize,
}

impl Frames {
    pub(super) fn retained(&self) -> (usize, usize) {
        (self.entries.len(), self.bytes)
    }
    pub(super) fn new(font: ratatui_image::FontSize) -> Self {
        Self {
            entries: Vec::new(),
            bytes: 0,
            font,
        }
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    fn pixels(&self, cells: Size) -> [u32; 2] {
        [
            u32::from(cells.width) * u32::from(self.font.width),
            u32::from(cells.height) * u32::from(self.font.height),
        ]
    }

    pub(super) fn get(
        &mut self,
        cells: Size,
        background: image::Rgba<u8>,
        view: View,
    ) -> Option<Preview> {
        let pixels = self.pixels(cells);
        let index = self.entries.iter().position(|entry| {
            entry.cells == cells
                && entry.background == background
                && same_pixels(entry.preview.view, view, pixels)
        })?;
        let entry = self.entries.remove(index);
        let mut preview = entry.preview.clone();
        // Keep the caller's precise camera state even when its f32 render
        // region is identical to a previously completed frame.
        preview.view = view;
        self.entries.push(entry);
        Some(preview)
    }

    pub(super) fn insert(&mut self, cells: Size, background: image::Rgba<u8>, preview: &Preview) {
        self.retain(cells, background, preview, true);
    }

    pub(super) fn insert_recent(
        &mut self,
        cells: Size,
        background: image::Rgba<u8>,
        preview: &Preview,
    ) {
        self.retain(cells, background, preview, false);
    }

    fn retain(
        &mut self,
        cells: Size,
        background: image::Rgba<u8>,
        preview: &Preview,
        compress: bool,
    ) {
        if preview.attachment || preview.time_dependent || preview.live_pages {
            return;
        }
        let Some(encoded) = preview.protocol.native_bytes() else {
            return;
        };
        let pixels = self.pixels(cells);
        if let Some(index) = self.entries.iter().position(|entry| {
            entry.cells == cells
                && entry.background == background
                && same_pixels(entry.preview.view, preview.view, pixels)
        }) {
            let entry = self.entries.remove(index);
            self.entries.push(entry);
            return;
        }
        // Initial preparation compresses distant sources before publication.
        // Keep at most two recent sources raw so another PNG compression cannot
        // delay prediction or the next input. Both forms share the byte budget.
        let PreviewImage::Pixels(pixels) = &preview.image else {
            return;
        };
        let (source, source_bytes) = if compress {
            let png = match pixels.png() {
                Ok(png) => Arc::<[u8]>::from(png),
                Err(_) => return,
            };
            let bytes = png.len();
            (PreviewImage::Png(png), bytes)
        } else {
            (preview.image.clone(), pixels.retained_bytes())
        };
        let bytes = source_bytes.saturating_add(encoded);
        if bytes > MAX_BYTES {
            return;
        }
        if !compress {
            while self
                .entries
                .iter()
                .filter(|entry| matches!(entry.preview.image, PreviewImage::Pixels(_)))
                .count()
                >= MAX_RECENT
            {
                let index = self
                    .entries
                    .iter()
                    .position(|entry| matches!(entry.preview.image, PreviewImage::Pixels(_)))
                    .unwrap();
                self.bytes -= self.entries.remove(index).bytes;
            }
        }
        while self.entries.len() >= MAX_ENTRIES || self.bytes + bytes > MAX_BYTES {
            self.bytes -= self.entries.remove(0).bytes;
        }
        self.bytes += bytes;
        let mut preview = preview.clone();
        preview.image = source;
        self.entries.push(Entry {
            cells,
            background,
            preview,
            bytes,
        });
    }
}

pub(super) fn same_pixels(a: View, b: View, pixels: [u32; 2]) -> bool {
    a.zoom == b.zoom && a.time == b.time && a.region(pixels) == b.region(pixels)
}
