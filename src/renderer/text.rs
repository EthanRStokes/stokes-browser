use super::font::FontManager;
use crate::css::ComputedValues;
use crate::layout::LayoutBox;
// Text rendering functionality
use skia_safe::{BlurStyle, Canvas, Font, MaskFilter, Paint, TextBlob};

/// Render text with CSS styles applied and DPI scale factor
pub fn render_text_node(
    canvas: &Canvas,
    layout_box: &LayoutBox,
    computed_styles: Option<&ComputedValues>,
    font_manager: &FontManager,
    default_text_paint: &Paint,
    scale_factor: f32,
) {
    if let Some(text) = &layout_box.content {
        let content_rect = layout_box.dimensions.content;

        // Create text paint with CSS colors and font properties
        let mut text_paint = default_text_paint.clone();
        let mut font_size = 14.0; // Default font size
        let mut text_align = crate::css::TextAlign::Left; // Default alignment
        let mut font_style = crate::css::FontStyle::Normal; // Default font style
        let mut font_family = "Arial".to_string(); // Default font family
        let mut font_weight = "normal".to_string(); // Default font weight
        let mut line_height_value = crate::css::LineHeight::Normal; // Default line height
        let mut vertical_align = crate::css::VerticalAlign::Baseline; // Default vertical alignment
        let mut text_transform = crate::css::TextTransform::None; // Default text transform
        let mut white_space = crate::css::WhiteSpace::Normal; // Default white-space
        let mut text_shadows = Vec::new(); // Default text shadows

        if let Some(styles) = computed_styles {
            // Apply CSS color
            if let Some(text_color) = &styles.color {
                let mut color = text_color.to_skia_color();
                // Apply opacity to text color
                color = color.with_a((color.a() as f32 * styles.opacity) as u8);
                text_paint.set_color(color);
            }

            // Apply CSS font size
            font_size = styles.font_size;

            // Apply CSS text-align
            text_align = styles.text_align.clone();

            // Apply CSS font-style
            font_style = styles.font_style.clone();

            // Apply CSS font-family
            font_family = styles.font_family.clone();

            // Apply CSS font-weight
            font_weight = styles.font_weight.clone();

            // Apply CSS line-height
            line_height_value = styles.line_height.clone();

            // Apply CSS vertical-align
            vertical_align = styles.vertical_align.clone();

            // Apply CSS text-transform
            text_transform = styles.text_transform.clone();

            // Apply CSS white-space
            white_space = styles.white_space.clone();

            // Apply CSS text-shadow
            text_shadows = styles.text_shadow.clone();
        }

        // Apply text transformation to the content
        let transformed_text = text_transform.apply(text);

        // Apply DPI scaling to font size
        let scaled_font_size = font_size * scale_factor as f32;

        // Calculate line height based on CSS line-height property
        let line_height = line_height_value.to_px(scaled_font_size);

        // Calculate vertical alignment offset
        let vertical_align_offset = vertical_align.to_px(scaled_font_size, line_height) * scale_factor as f32;

        // Get or create font with the scaled size, family, weight, and style
        let font = font_manager.get_font(&font_family, scaled_font_size, &font_weight, &font_style);

        // TODO: Wrap text based on actual font metrics, container width, and white-space property
        let wrapped_lines: Vec<&str> = transformed_text.split('\n')
            .map(|line| line.trim_start()) // Remove leading whitespace from each line
            .collect();

        // Position text within the content area with scaled padding
        let scaled_padding = 2.0 * scale_factor as f32;
        let mut current_y = content_rect.top + scaled_font_size; // Start at baseline position

        // Render each line separately
        for line in wrapped_lines {
            if let Some(text_blob) = TextBlob::new(&line, &font) {
                let text_bounds = text_blob.bounds();
                let text_width = text_bounds.width();

                // Calculate x position based on text-align
                let start_x = match text_align {
                    crate::css::TextAlign::Left => content_rect.left + scaled_padding,
                    crate::css::TextAlign::Right => content_rect.right - text_width - scaled_padding,
                    crate::css::TextAlign::Center => content_rect.left + (content_rect.width() - text_width) / 2.0,
                    crate::css::TextAlign::Justify => {
                        // For now, justify is treated as left-align
                        // Full justify implementation would require word spacing adjustments
                        content_rect.left + scaled_padding
                    }
                };

                // Apply vertical alignment offset to the y position
                let adjusted_y = current_y + vertical_align_offset;

                // Render text shadows first (so they appear behind the text)
                if let Some(styles) = computed_styles {
                    for shadow in &text_shadows {
                        let shadow_px = shadow.to_px(scaled_font_size, 0.0);

                        // Skip shadow if it has no effect
                        if !shadow_px.has_shadow() {
                            continue;
                        }

                        // Create shadow paint
                        let mut shadow_paint = text_paint.clone();

                        // Apply shadow color with opacity
                        let mut shadow_color = shadow_px.color.to_skia_color();
                        shadow_color = shadow_color.with_a((shadow_color.a() as f32 * styles.opacity) as u8);
                        shadow_paint.set_color(shadow_color);

                        // Apply blur if specified
                        if shadow_px.blur_radius > 0.0 {
                            let blur_sigma = shadow_px.blur_radius / 2.0;
                            if let Some(mask_filter) = MaskFilter::blur(BlurStyle::Normal, blur_sigma, None) {
                                shadow_paint.set_mask_filter(mask_filter);
                            }
                        }

                        // Draw shadow at offset position
                        let shadow_x = start_x + shadow_px.offset_x * scale_factor as f32;
                        let shadow_y = adjusted_y + shadow_px.offset_y * scale_factor as f32;
                        canvas.draw_text_blob(&text_blob, (shadow_x, shadow_y), &shadow_paint);
                    }
                }

                // Render the actual text on top of shadows
                canvas.draw_text_blob(&text_blob, (start_x, adjusted_y), &text_paint);

                // Render text decorations if specified (with opacity applied)
                if let Some(styles) = computed_styles {
                    // Create decoration paint with opacity
                    let decoration_paint = text_paint.clone();
                    super::decorations::render_text_decorations(
                        canvas,
                        &text_blob,
                        (start_x, adjusted_y),
                        &styles.text_decoration,
                        &decoration_paint,
                        scaled_font_size,
                        scale_factor,
                    );
                }
            }
            current_y += line_height; // Move to next line using computed line height
        }
    }
}

/// Wrap text based on actual font metrics and available width
pub fn wrap_text_with_font(text: &str, font: &Font, max_width: f32, white_space: &crate::css::WhiteSpace) -> Vec<String> {
    let mut wrapped_lines = Vec::new();

    // If white-space is nowrap or pre, don't wrap at all
    if !white_space.should_wrap() {
        // Split by explicit newlines only (for pre modes)
        if white_space.preserve_whitespace() {
            // For pre/pre-wrap modes, preserve all whitespace including newlines
            return text.lines().map(|s| s.to_string()).collect();
        } else {
            // For nowrap, collapse whitespace but don't wrap
            let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
            return vec![collapsed];
        }
    }

    // Split by explicit newlines first
    let paragraphs: Vec<&str> = text.split('\n').collect();

    for paragraph in paragraphs {
        if paragraph.is_empty() {
            wrapped_lines.push(String::new());
            continue;
        }

        // Split paragraph into words
        let words: Vec<&str> = paragraph.split_whitespace().collect();

        if words.is_empty() {
            wrapped_lines.push(String::new());
            continue;
        }

        let mut current_line = String::new();

        for word in words {
            // Check if adding this word would exceed the line width
            let test_line = if current_line.is_empty() {
                word.to_string()
            } else {
                format!("{} {}", current_line, word)
            };

            // Measure the actual width of the test line using the font
            if let Some(text_blob) = TextBlob::new(&test_line, font) {
                let text_bounds = text_blob.bounds();
                let text_width = text_bounds.width();

                if text_width <= max_width {
                    current_line = test_line;
                } else {
                    // If current line is not empty, save it and start a new line
                    if !current_line.is_empty() {
                        wrapped_lines.push(current_line);
                        current_line = word.to_string();
                    } else {
                        // Word itself is too long, need to break it up
                        // Check if the word fits on its own line
                        if let Some(word_blob) = TextBlob::new(word, font) {
                            let word_width = word_blob.bounds().width();
                            if word_width > max_width {
                                // Word is too long, break it character by character
                                let mut char_line = String::new();
                                for ch in word.chars() {
                                    let test_char_line = format!("{}{}", char_line, ch);
                                    if let Some(char_blob) = TextBlob::new(&test_char_line, font) {
                                        let char_width = char_blob.bounds().width();
                                        if char_width <= max_width {
                                            char_line = test_char_line;
                                        } else {
                                            if !char_line.is_empty() {
                                                wrapped_lines.push(char_line);
                                            }
                                            char_line = ch.to_string();
                                        }
                                    }
                                }
                                current_line = char_line;
                            } else {
                                current_line = word.to_string();
                            }
                        }
                    }
                }
            }
        }

        // Add the last line if it's not empty
        if !current_line.is_empty() {
            wrapped_lines.push(current_line);
        }
    }

    // Return at least one empty line if everything was empty
    if wrapped_lines.is_empty() {
        wrapped_lines.push(String::new());
    }

    wrapped_lines
}
