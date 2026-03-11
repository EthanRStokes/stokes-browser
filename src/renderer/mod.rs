pub(crate) mod text;
mod image;
pub(crate) mod background;
mod cache;
pub(crate) mod kurbo_css;
mod layers;
mod shadow;
mod gradient;
mod sizing;
pub mod painter;
pub(crate) mod webrender_compositor;

use std::any::Any;
use std::collections::HashMap;
use crate::dom::node::{ListItemLayout, ListItemLayoutPosition, Marker, SpecialElementData, TextInputData};
use crate::dom::{Dom, DomNode, ElementData};
use crate::renderer::kurbo_css::{CssBox, Edge, NonUniformRoundedRectRadii};
use crate::renderer::text::{draw_text_selection, stroke_text, SELECTION_COLOR};
use crate::renderer::painter::ToColorColor;
use anyrender::{CustomPaint, Paint, PaintScene};
use color::{AlphaColor, Srgb};
use kurbo::{Affine, BezPath, Insets, Point, Rect, Vec2};
use peniko::Fill;
use style::properties::generated::longhands::border_collapse::computed_value::T as BorderCollapse;
use style::properties::style_structs::Font;
use style::properties::ComputedValues;
use style::servo_arc::Arc;
use style::values::computed::{BorderCornerRadius, BorderStyle, CSSPixelLength, OutlineStyle};
use style::values::generics::color::GenericColorOrAuto;
use taffy::Layout;
use crate::renderer::background::{to_image_quality, to_peniko_image};
use crate::renderer::sizing::compute_object_fit;

pub(crate) trait ElementRenderContext {
    fn dom(&self) -> &Dom;
    fn selection_ranges(&self) -> &HashMap<usize, (usize, usize)>;
    fn scale_factor(&self) -> f64;

    fn element<'a>(
        &'a self,
        node: &'a DomNode,
        layout: Layout,
        position: Point,
    ) -> Element<'a, Self>
    where
        Self: Sized,
    {
        let style = node
            .stylo_data
            .borrow()
            .as_ref()
            .map(|elem_data| elem_data.styles.primary().clone())
            .unwrap_or(ComputedValues::initial_values_with_font_override(Font::initial_values()));

        let scale = self.scale_factor();
        let frame = create_css_rect(&style, &layout, scale);
        let transform = Affine::translate(position.to_vec2() * scale);
        let element = node.element_data().unwrap();

        Element {
            context: self,
            frame,
            style,
            position,
            scale_factor: scale,
            node,
            element,
            transform,
            svg: element.svg_data(),
            text_input: element.text_input_data(),
            list_item: element.list_item_data.as_deref(),
        }
    }
}

pub(crate) struct FragmentElementContext<'dom, 'sel> {
    dom: &'dom Dom,
    selection_ranges: &'sel HashMap<usize, (usize, usize)>,
    scale_factor: f64,
}

impl<'dom, 'sel> FragmentElementContext<'dom, 'sel> {
    pub(crate) fn new(
        dom: &'dom Dom,
        selection_ranges: &'sel HashMap<usize, (usize, usize)>,
        scale_factor: f64,
    ) -> Self {
        Self {
            dom,
            selection_ranges,
            scale_factor,
        }
    }
}

impl ElementRenderContext for FragmentElementContext<'_, '_> {
    fn dom(&self) -> &Dom {
        self.dom
    }

    fn selection_ranges(&self) -> &HashMap<usize, (usize, usize)> {
        self.selection_ranges
    }

    fn scale_factor(&self) -> f64 {
        self.scale_factor
    }
}

fn insets_from_taffy_rect(input: taffy::Rect<f64>) -> Insets {
    Insets {
        x0: input.left,
        y0: input.top,
        x1: input.right,
        y1: input.bottom,
    }
}

/// Convert Stylo and Taffy types into Kurbo types
fn create_css_rect(style: &ComputedValues, layout: &Layout, scale: f64) -> CssBox {
    // Resolve and rescale
    // We have to scale since document pixels are not same same as rendered pixels
    let width: f64 = layout.size.width as f64;
    let height: f64 = layout.size.height as f64;
    let border_box = Rect::new(0.0, 0.0, width * scale, height * scale);
    let border = insets_from_taffy_rect(layout.border.map(|p| p as f64 * scale));
    let padding = insets_from_taffy_rect(layout.padding.map(|p| p as f64 * scale));
    let outline_width = style.get_outline().outline_width.0.to_f64_px() * scale;

    // Resolve the radii to a length. need to downscale since the radii are in document pixels
    let resolve_w = CSSPixelLength::new(width as _);
    let resolve_h = CSSPixelLength::new(height as _);
    let resolve_radii = |radius: &BorderCornerRadius| -> Vec2 {
        Vec2 {
            x: scale * radius.0.width.0.resolve(resolve_w).px() as f64,
            y: scale * radius.0.height.0.resolve(resolve_h).px() as f64,
        }
    };
    let s_border = style.get_border();
    let border_radii = NonUniformRoundedRectRadii {
        top_left: resolve_radii(&s_border.border_top_left_radius),
        top_right: resolve_radii(&s_border.border_top_right_radius),
        bottom_right: resolve_radii(&s_border.border_bottom_right_radius),
        bottom_left: resolve_radii(&s_border.border_bottom_left_radius),
    };

    CssBox::new(border_box, border, padding, outline_width, border_radii)
}

/// Create a `CssBox` from the serializable `ResolvedStyle` and a `taffy::Layout`.
/// Used by `FragmentTreeRenderer` on the parent process side.
pub(crate) fn create_css_rect_from_fragment(
    style: &crate::fragment_tree::ResolvedStyle,
    layout: &Layout,
    scale: f64,
) -> CssBox {
    let width: f64 = layout.size.width as f64;
    let height: f64 = layout.size.height as f64;
    let border_box = Rect::new(0.0, 0.0, width * scale, height * scale);
    let border = insets_from_taffy_rect(layout.border.map(|p| p as f64 * scale));
    let padding = insets_from_taffy_rect(layout.padding.map(|p| p as f64 * scale));
    let outline_width = style.outline_width * scale;

    let border_radii = NonUniformRoundedRectRadii {
        top_left: Vec2 {
            x: style.border_top_left_radius.0 * scale,
            y: style.border_top_left_radius.1 * scale,
        },
        top_right: Vec2 {
            x: style.border_top_right_radius.0 * scale,
            y: style.border_top_right_radius.1 * scale,
        },
        bottom_right: Vec2 {
            x: style.border_bottom_right_radius.0 * scale,
            y: style.border_bottom_right_radius.1 * scale,
        },
        bottom_left: Vec2 {
            x: style.border_bottom_left_radius.0 * scale,
            y: style.border_bottom_left_radius.1 * scale,
        },
    };

    CssBox::new(border_box, border, padding, outline_width, border_radii)
}

pub(crate) struct Element<'a, C: ElementRenderContext + ?Sized> {
    context: &'a C,
    frame: CssBox,
    pub(crate) style: Arc<ComputedValues>,
    pub(crate) position: Point,
    pub(crate) scale_factor: f64,
    pub(crate) node: &'a DomNode,
    pub(crate) element: &'a ElementData,
    pub(crate) transform: Affine,
    svg: Option<&'a usvg::Tree>,
    text_input: Option<&'a TextInputData>,
    list_item: Option<&'a ListItemLayout>,
}

impl<C: ElementRenderContext + ?Sized> Element<'_, C> {
    pub(crate) fn draw_marker(&self, painter: &mut impl PaintScene, pos: Point) {
        if let Some(ListItemLayout {
                        marker,
                        position: ListItemLayoutPosition::Outside(layout),
                    }) = self.list_item
        {
            // Right align and pad the bullet when rendering outside
            let x_padding = match marker {
                Marker::Char(_) => 8.0,
                Marker::String(_) => 0.0,
            };
            let x_offset = -(layout.full_width() / layout.scale() + x_padding);

            // Align the marker with the baseline of the first line of text in the list item
            let y_offset = if let Some(first_text_line) = &self
                .element
                .inline_layout_data
                .as_ref()
                .and_then(|text_layout| text_layout.layout.lines().next())
            {
                (first_text_line.metrics().baseline
                    - layout.lines().next().unwrap().metrics().baseline)
                    / layout.scale()
            } else {
                0.0
            };

            let pos = Point {
                x: pos.x + x_offset as f64,
                y: pos.y + y_offset as f64,
            };

            let transform =
                Affine::translate((pos.x * self.scale_factor, pos.y * self.scale_factor));

            stroke_text(painter, layout.lines(), self.context.dom(), transform);
        }
    }

    pub(crate) fn draw_inline_layout(&self, painter: &mut impl PaintScene, pos: Point) {
        if self.node.flags.is_inline_root() {
            let text_layout = self.element.inline_layout_data.as_ref().unwrap_or_else(|| {
                panic!("Tried to render node marked as inline root but has no inline layout data: {:?}", self.node)
            });

            let transform =
                Affine::translate((pos.x * self.scale_factor, pos.y * self.scale_factor));

            if let Some(&(start, end)) = self.context.selection_ranges().get(&self.node.id) {
                draw_text_selection(
                    painter,
                    &text_layout.layout,
                    transform,
                    start,
                    end,
                );
            }

            stroke_text(
                painter,
                text_layout.layout.lines(),
                    self.context.dom(),
                transform,
            )
        }
    }

    pub(crate) fn draw_text_input_text(&self, painter: &mut impl PaintScene, pos: Point) {
        if let Some(input_data) = self.text_input {
            let y_offset = self.node.text_input_v_centering_offset(self.scale_factor);
            let pos = Point {
                x: pos.x,
                y: pos.y + y_offset,
            };

            let transform = Affine::translate((pos.x * self.scale_factor, pos.y * self.scale_factor));

            if self.node.is_focused() {
                // Render selection/caret
                for (rect, |line_idx) in input_data.editor.selection_geometry().iter() {
                    painter.fill(
                        Fill::NonZero,
                        transform,
                        SELECTION_COLOR,
                        None,
                        &Rect::new(rect.x0, rect.y0, rect.x1, rect.y1),
                    )
                }

                if let Some(cursor) = input_data.editor.cursor_geometry(1.5) {
                    let color = self.style.get_inherited_text().color;
                    let caret_color = match &self.style.get_inherited_ui().caret_color.0 {
                        GenericColorOrAuto::Color(caret_color) => caret_color.resolve_to_absolute(&color),
                        GenericColorOrAuto::Auto => color,
                    };

                    painter.fill(
                        Fill::NonZero,
                        transform,
                        caret_color.as_color_color(),
                        None,
                        &Rect::new(cursor.x0, cursor.y0, cursor.x1, cursor.y1),
                    );
                }
            }

            stroke_text(
                painter,
                input_data.editor.try_layout().unwrap().lines(),
                self.context.dom(),
                transform,
            );
        }
    }

    pub(crate) fn draw_border(&self, painter: &mut impl PaintScene) {
        let style = &*self.style;
        let border = style.get_border();
        let current_color = style.clone_color();

        let mut borders: [(AlphaColor<Srgb>, Option<BezPath>); 4] = [
            (AlphaColor::TRANSPARENT, None),
            (AlphaColor::TRANSPARENT, None),
            (AlphaColor::TRANSPARENT, None),
            (AlphaColor::TRANSPARENT, None),
        ];
        let mut count = 0;

        for &edge in &[Edge::Top, Edge::Right, Edge::Bottom, Edge::Left] {
            let color = match edge {
                Edge::Top => &border.border_top_color,
                Edge::Right => &border.border_right_color,
                Edge::Bottom => &border.border_bottom_color,
                Edge::Left => &border.border_left_color,
            }
                .resolve_to_absolute(&current_color)
                .as_color_color();

            if color.components[3] > 0.0 {
                borders[count] = (color, Some(self.frame.border_edge_shape(edge)));
                count += 1;
            }
        }

        if count == 0 {
            return;
        }

        // Group together identical colors by sorting.
        let active_slice = &mut borders[0..count];
        active_slice.sort_unstable_by(|a, b| {
            a.0.components
                .partial_cmp(&b.0.components)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut start_border_index = 0;
        while start_border_index < count {
            let color = borders[start_border_index].0;
            let mut next_border_index = start_border_index + 1;
            let has_multiple_edges =
                next_border_index < count && borders[next_border_index].0 == color;
            if has_multiple_edges {
                let mut border_path = borders[start_border_index].1.take().unwrap();
                while next_border_index < count && borders[next_border_index].0 == color {
                    border_path.extend(&borders[next_border_index].1.take().unwrap());
                    next_border_index += 1;
                }
                painter.fill(Fill::NonZero, self.transform, color, None, &border_path);
            } else {
                painter.fill(
                    Fill::NonZero,
                    self.transform,
                    color,
                    None,
                    borders[start_border_index].1.as_ref().unwrap(),
                );
            }
            start_border_index = next_border_index;
        }
    }

    pub(crate) fn draw_outline(&self, painter: &mut impl PaintScene) {
        let outline = self.style.get_outline();

        let current_color = self.style.clone_color();
        let color = outline.outline_color.resolve_to_absolute(&current_color).as_color_color();

        let style = match outline.outline_style {
            OutlineStyle::Auto => return,
            OutlineStyle::BorderStyle(style) => style,
        };

        let path = match style {
            BorderStyle::None | BorderStyle::Hidden => return,
            BorderStyle::Solid => self.frame.outline(),

            _ => {
                // TODO For other styles, just draw solid for now
                self.frame.outline()
            }
        };

        painter.fill(Fill::NonZero, self.transform, color, None, &path)
    }

    pub(crate) fn draw_table_borders(&self, scene: &mut impl PaintScene) {
        let SpecialElementData::TableRoot(table) = &self.element.special_data else {
            return;
        };
        // Borders are only handled at the table level when BorderCollapse::Collapse
        if table.border_collapse != BorderCollapse::Collapse {
            return;
        }

        let Some(grid_info) = &mut *table.computed_grid_info.borrow_mut() else {
            return;
        };
        let Some(border_style) = table.border_style.as_deref() else {
            return;
        };

        let outer_border_style = self.style.get_border();

        let cols = &grid_info.columns;
        let rows = &grid_info.rows;

        let inner_width =
            (cols.sizes.iter().sum::<f32>() + cols.gutters.iter().sum::<f32>()) as f64;
        let inner_height =
            (rows.sizes.iter().sum::<f32>() + rows.gutters.iter().sum::<f32>()) as f64;

        // TODO: support different colors for different borders
        let current_color = self.style.clone_color();
        let border_color = border_style
            .border_top_color
            .resolve_to_absolute(&current_color)
            .as_color_color();

        // No need to draw transparent borders (as they won't be visible anyway)
        if border_color == AlphaColor::TRANSPARENT {
            return;
        }

        let border_width = border_style.border_top_width.0.to_f64_px();

        // Draw horizontal inner borders
        let mut y = 0.0;
        for (&height, &gutter) in rows.sizes.iter().zip(rows.gutters.iter()) {
            let shape =
                Rect::new(0.0, y, inner_width, y + gutter as f64).scale_from_origin(self.scale_factor);
            scene.fill(Fill::NonZero, self.transform, border_color, None, &shape);

            y += (height + gutter) as f64;
        }

        // Draw horizontal outer borders
        // Top border
        if outer_border_style.border_top_style != BorderStyle::Hidden {
            let shape =
                Rect::new(0.0, 0.0, inner_width, border_width).scale_from_origin(self.scale_factor);
            scene.fill(Fill::NonZero, self.transform, border_color, None, &shape);
        }
        // Bottom border
        if outer_border_style.border_bottom_style != BorderStyle::Hidden {
            let shape = Rect::new(0.0, inner_height, inner_width, inner_height + border_width)
                .scale_from_origin(self.scale_factor);
            scene.fill(Fill::NonZero, self.transform, border_color, None, &shape);
        }

        // Draw vertical inner borders
        let mut x = 0.0;
        for (&width, &gutter) in cols.sizes.iter().zip(cols.gutters.iter()) {
            let shape =
                Rect::new(x, 0.0, x + gutter as f64, inner_height).scale_from_origin(self.scale_factor);
            scene.fill(Fill::NonZero, self.transform, border_color, None, &shape);

            x += (width + gutter) as f64;
        }

        // Draw vertical outer borders
        // Left border
        if outer_border_style.border_left_style != BorderStyle::Hidden {
            let shape =
                Rect::new(0.0, 0.0, border_width, inner_height).scale_from_origin(self.scale_factor);
            scene.fill(Fill::NonZero, self.transform, border_color, None, &shape);
        }
        // Right border
        if outer_border_style.border_right_style != BorderStyle::Hidden {
            let shape = Rect::new(inner_width, 0.0, inner_width + border_width, inner_height)
                .scale_from_origin(self.scale_factor);
            scene.fill(Fill::NonZero, self.transform, border_color, None, &shape);
        }
    }

    pub(crate) fn draw_svg(&self, scene: &mut impl PaintScene) {
        use style::properties::generated::longhands::object_fit::computed_value::T as ObjectFit;

        let Some(svg) = self.svg else {
            return;
        };

        let width = self.frame.content_box.width() as u32;
        let height = self.frame.content_box.height() as u32;
        let svg_size = svg.size();

        let x = self.frame.content_box.origin().x;
        let y = self.frame.content_box.origin().y;

        // let object_fit = self.style.clone_object_fit();
        let object_position = self.style.clone_object_position();

        // Apply object-fit algorithm
        let container_size = taffy::Size {
            width: width as f32,
            height: height as f32,
        };
        let object_size = taffy::Size {
            width: svg_size.width(),
            height: svg_size.height(),
        };
        let paint_size = compute_object_fit(container_size, Some(object_size), ObjectFit::Contain);

        // Compute object-position
        let x_offset = object_position.horizontal.resolve(
            CSSPixelLength::new(container_size.width - paint_size.width) / self.scale_factor as f32,
        ) * self.scale_factor as f32;
        let y_offset = object_position.vertical.resolve(
            CSSPixelLength::new(container_size.height - paint_size.height) / self.scale_factor as f32,
        ) * self.scale_factor as f32;
        let x = x + x_offset.px() as f64;
        let y = y + y_offset.px() as f64;

        let x_scale = paint_size.width as f64 / object_size.width as f64;
        let y_scale = paint_size.height as f64 / object_size.height as f64;

        let transform = self
            .transform
            .pre_scale_non_uniform(x_scale, y_scale)
            .then_translate(Vec2 { x, y });

        anyrender_svg::render_svg_tree(scene, svg, transform);
    }

    pub(crate) fn draw_image(&self, painter: &mut impl PaintScene) {
        if let Some(image) = self.element.raster_image_data() {
            let width = self.frame.content_box.width() as u32;
            let height = self.frame.content_box.height() as u32;
            let x = self.frame.content_box.origin().x;
            let y = self.frame.content_box.origin().y;

            let object_fit = self.style.clone_object_fit();
            let object_position = self.style.clone_object_position();
            let image_rendering = self.style.clone_image_rendering();
            let quality = to_image_quality(image_rendering);

            // Apply object-fit algorithm
            let container_size = taffy::Size {
                width: width as f32,
                height: height as f32,
            };
            let object_size = taffy::Size {
                width: image.width as f32,
                height: image.height as f32,
            };
            let paint_size = compute_object_fit(container_size, Some(object_size), object_fit);

            // Compute object-position
            let x_offset = object_position.horizontal.resolve(
                CSSPixelLength::new(container_size.width - paint_size.width) / self.scale_factor as f32,
            ) * self.scale_factor as f32;
            let y_offset = object_position.vertical.resolve(
                CSSPixelLength::new(container_size.height - paint_size.height) / self.scale_factor as f32,
            ) * self.scale_factor as f32;
            let x = x + x_offset.px() as f64;
            let y = y + y_offset.px() as f64;

            let x_scale = paint_size.width as f64 / object_size.width as f64;
            let y_scale = paint_size.height as f64 / object_size.height as f64;
            let transform = self
                .transform
                .pre_translate(Vec2 { x, y })
                .pre_scale_non_uniform(x_scale, y_scale);

            painter.draw_image(to_peniko_image(image, quality).as_ref(), transform);
        }
    }

    pub(crate) fn draw_canvas(&self, painter: &mut impl PaintScene) {
        let Some(custom_paint_source) = self.element.canvas_data() else {
            return;
        };

        let width = self.frame.content_box.width() as u32;
        let height = self.frame.content_box.height() as u32;
        let x = self.frame.content_box.origin().x;
        let y = self.frame.content_box.origin().y;

        let transform = self.transform.then_translate(Vec2 { x, y} );

        painter.fill(
            Fill::NonZero,
            transform,
            Paint::Custom(&CustomPaint {
                source_id: custom_paint_source.custom_paint_source_id,
                width,
                height,
                scale: self.scale_factor
            } as &(dyn Any + Send + Sync)),
            None,
            &Rect::from_origin_size((0.0, 0.0), (width as f64, height as f64)),
        );
    }
}
