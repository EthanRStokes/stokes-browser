use anyrender::PaintScene;
use kurbo::Affine;
use peniko::{ImageAlphaType, ImageBrush, ImageData, ImageFormat, ImageQuality, ImageSampler};

/// Render SVGs through resvg first (better feature coverage like masks),
/// then fall back to anyrender_svg if rasterization fails.
pub(crate) fn render_svg_tree(scene: &mut impl PaintScene, svg: &usvg::Tree, transform: Affine) {
    if let Some(image) = rasterize_svg(svg) {
        scene.draw_image(image.as_ref(), transform);
    } else {
        anyrender_svg::render_svg_tree(scene, svg, transform);
    }
}

fn rasterize_svg(svg: &usvg::Tree) -> Option<ImageBrush> {
    let size = svg.size();
    let width = size.width().ceil().max(1.0) as u32;
    let height = size.height().ceil().max(1.0) as u32;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;

    let mut pixmap_mut = pixmap.as_mut();
    resvg::render(svg, resvg::tiny_skia::Transform::identity(), &mut pixmap_mut);

    let data = pixmap.take();
    Some(ImageBrush {
        image: ImageData {
            data: peniko::Blob::new(std::sync::Arc::new(data)),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::AlphaPremultiplied,
            width,
            height,
        },
        sampler: ImageSampler {
            x_extend: peniko::Extend::Repeat,
            y_extend: peniko::Extend::Repeat,
            quality: ImageQuality::Medium,
            alpha: 1.0,
        },
    })
}

