//! Parse native effect values once before scaling and constructing Skia filters.
use super::*;
use crate::effects::Geometry;

#[derive(Clone, Copy)]
enum Kind<'a> {
    Blur,
    Shadow {
        inner: bool,
        color: Color,
        mode: &'a str,
    },
}

pub(super) struct NativeEffect<'a> {
    pub geometry: Geometry,
    kind: Kind<'a>,
}

impl<'a> NativeEffect<'a> {
    pub fn parse(node: &ResolvedNode, effect: &'a Value) -> Result<Self> {
        let kind = match effect["type"].as_str() {
            Some("blur") => Kind::Blur,
            Some("shadow") => Kind::Shadow {
                inner: blending::shadow_type(effect)
                    .map_err(|error| blending::at_node(error, node, "effect"))?
                    == "inner",
                color: color::parse(
                    effect
                        .get("color")
                        .and_then(Value::as_str)
                        .unwrap_or("#00000040"),
                )?,
                mode: blending::name(effect)
                    .map_err(|error| blending::at_node(error, node, "effect"))?,
            },
            _ => return Err(unsupported(node, "This effect is not implemented yet")),
        };
        Ok(Self {
            geometry: Geometry::parse(effect)?,
            kind,
        })
    }

    pub fn scaled(mut self, transform: Affine, scale: f32) -> Result<Self> {
        self.geometry = self.geometry.scaled(transform, scale)?;
        Ok(self)
    }

    pub fn outer_shadow(&self) -> Option<(Color, &'a str)> {
        match self.kind {
            Kind::Shadow {
                inner: false,
                color,
                mode,
            } => Some((color, mode)),
            _ => None,
        }
    }

    pub fn is_shadow(&self) -> bool {
        matches!(self.kind, Kind::Shadow { .. })
    }

    pub fn filter(
        &self,
        input: Option<ImageFilter>,
        node: &ResolvedNode,
    ) -> Result<Option<ImageFilter>> {
        let Geometry {
            blur,
            offset: [x, y],
        } = self.geometry;
        Ok(match self.kind {
            Kind::Blur => {
                image_filters::blur((blur / 2.0, blur / 2.0), TileMode::Decal, input, None)
            }
            Kind::Shadow {
                inner: false,
                color,
                mode,
            } => {
                if mode != "normal" {
                    return Err(Error::new(
                        "RENDER_ERROR",
                        "A blended outer shadow requires scene-aware compositing",
                    ));
                }
                image_filters::drop_shadow(
                    (x, y),
                    (blur / 2.0, blur / 2.0),
                    color,
                    None,
                    input,
                    None,
                )
            }
            Kind::Shadow {
                inner: true,
                color,
                mode,
            } => {
                let blender = blending::inner_shadow(mode)
                    .map_err(|error| blending::at_node(error, node, "effect"))?;
                let shifted = image_filters::offset((x, y), input.clone(), None);
                let blurred =
                    image_filters::blur((blur / 2.0, blur / 2.0), TileMode::Decal, shifted, None);
                let inverse_alpha = skia_safe::color_filters::matrix_row_major(
                    &[
                        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                        0.0, 0.0, 0.0, -1.0, 1.0,
                    ],
                    None,
                );
                let inverted = image_filters::color_filter(inverse_alpha, blurred, None);
                let tint = skia_safe::color_filters::blend(color, BlendMode::SrcIn)
                    .ok_or_else(|| invalid("Cannot construct inner shadow color"))?;
                let shadow = image_filters::color_filter(tint, inverted, None);
                image_filters::blend(blender, input, shadow, None)
            }
        })
    }
}

pub(super) fn filter(node: &ResolvedNode) -> Result<Option<ImageFilter>> {
    let mut filter = None;
    if let Some(effects) = node.properties.get("effect") {
        for effect in crate::effects::active(effects) {
            filter = NativeEffect::parse(node, effect)?.filter(filter, node)?;
        }
    }
    Ok(filter)
}

#[cfg(target_os = "macos")]
pub(super) fn bound_filter_output(paint: &mut Paint, clip: Rect, text_mask: bool) {
    unsafe extern "C" {
        fn C_Lazypencil_boundFilterOutput(
            paint: *mut std::ffi::c_void,
            size: usize,
            alignment: usize,
            left: f32,
            top: f32,
            right: f32,
            bottom: f32,
            text_mask: bool,
        );
    }
    // The native bridge checks the transparent Paint wrapper's size and alignment.
    // The mutable borrow gives it exclusive access to the initialized native paint.
    unsafe {
        C_Lazypencil_boundFilterOutput(
            std::ptr::from_mut(paint).cast(),
            std::mem::size_of::<Paint>(),
            std::mem::align_of::<Paint>(),
            clip.x,
            clip.y,
            clip.right(),
            clip.bottom(),
            text_mask,
        );
    }
}
