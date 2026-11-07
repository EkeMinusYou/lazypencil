//! Validate a captured render request before allocating drawing resources.
use super::*;

pub(super) struct RasterPlan {
    pub region: Rect,
    pub width: u64,
    pub height: u64,
}

impl RasterPlan {
    pub fn new(
        project: &Project,
        snapshot: &LayoutSnapshot,
        options: &RenderOptions,
        preview: bool,
    ) -> Result<Self> {
        if !options.time.is_finite() || options.time < 0.0 {
            return Err(invalid("Shader time must be a finite nonnegative number"));
        }
        // Cached TUI previews can downsample large viewports below the public
        // export scale range. Their output still passes the raster size limits.
        let minimum_scale = if preview { f32::MIN_POSITIVE } else { 0.01 };
        if !options.scale.is_finite() || !(minimum_scale..=16.0).contains(&options.scale) {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Render scale must be within 0.01..16",
            ));
        }
        if snapshot.revision != project.root_document().revision()
            || snapshot.dependency_revision != project.dependency_revision()
        {
            return Err(Error::new(
                "REVISION_CONFLICT",
                "The layout snapshot does not belong to this project revision",
            ));
        }
        let selected = options
            .node
            .as_ref()
            .map(|id| {
                snapshot
                    .items
                    .iter()
                    .position(|i| &i.record.id == id)
                    .ok_or_else(|| {
                        Error::new("INVALID_ARGUMENT", format!("No resolved node {id:?}"))
                    })
            })
            .transpose()?;
        let region = options.region.unwrap_or_else(|| {
            selected
                .map(|i| snapshot.items[i].record.visual_bounds)
                .unwrap_or(snapshot.bounds)
        });
        if [region.x, region.y, region.width, region.height]
            .iter()
            .any(|n| !n.is_finite())
            || region.width < 0.0
            || region.height < 0.0
        {
            return Err(Error::new("INVALID_ARGUMENT", "Invalid render region"));
        }
        let width = (region.width * options.scale).ceil().max(1.0) as u64;
        let height = (region.height * options.scale).ceil().max(1.0) as u64;
        Self::check_size(width, height)?;
        Ok(Self {
            region,
            width,
            height,
        })
    }

    pub fn with_viewport(mut self, viewport: Option<[u32; 2]>) -> Result<Self> {
        if let Some([width, height]) = viewport {
            let (width, height) = (u64::from(width), u64::from(height));
            Self::check_size(width, height)?;
            if self.width.abs_diff(width) > 1 || self.height.abs_diff(height) > 1 {
                return Err(invalid(
                    "Preview viewport does not match the render region and scale",
                ));
            }
            // A preview already knows its integer viewport. Dividing it by a
            // fractional zoom and multiplying back in f32 can add one pixel to
            // ceil(). Allocate the requested viewport without resampling pixels.
            self.width = width;
            self.height = height;
        }
        Ok(self)
    }

    fn check_size(width: u64, height: u64) -> Result<()> {
        if width == 0
            || height == 0
            || width > 16384
            || height > 16384
            || width * height > MAX_PIXELS
        {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Render exceeds 16384 pixels per side or 32 megapixels",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_viewport_keeps_rounding_adjustments_bounded_and_checks_allocation_limits() {
        let plan = || RasterPlan {
            region: Rect::new(0.0, 0.0, 100.0, 100.0),
            width: 996,
            height: 981,
        };
        for pixels in [[996, 980], [996, 981], [997, 982]] {
            let result = plan().with_viewport(Some(pixels)).unwrap();
            assert_eq!([result.width, result.height], pixels.map(u64::from));
        }
        assert_eq!(plan().with_viewport(None).unwrap().height, 981);
        for pixels in [
            [0, 980],
            [996, 0],
            [998, 981],
            [996, 983],
            [u32::MAX, u32::MAX],
        ] {
            assert!(plan().with_viewport(Some(pixels)).is_err());
        }
        let plan = RasterPlan {
            region: Rect::default(),
            width: 16384,
            height: 2048,
        };
        assert!(plan.with_viewport(Some([16384, 2049])).is_err());
    }
}
