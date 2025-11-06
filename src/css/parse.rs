use style::values::computed::Percentage;

pub(crate) fn parse_color(value: &str) -> Option<(u8, u8, u8, f32)> {
    if !value.starts_with('#') {
        return None;
    }

    let value = &value[1..];
    if value.len() == 3 {
        let r = u8::from_str_radix(&value[0..1], 16).ok()?;
        let g = u8::from_str_radix(&value[1..2], 16).ok()?;
        let b = u8::from_str_radix(&value[2..3], 16).ok()?;
        return Some((r, g, b, 1.0));
    }

    if value.len() == 6 {
        let r = u8::from_str_radix(&value[0..2], 16).ok()?;
        let g = u8::from_str_radix(&value[2..4], 16).ok()?;
        let b = u8::from_str_radix(&value[4..6], 16).ok()?;
        return Some((r, g, b, 1.0));
    }

    None
}

pub(crate) fn parse_size(
    value: &str,
    filter_fn: impl FnOnce(&f32) -> bool,
) -> Option<style::values::specified::LengthPercentage> {
    use style::values::specified::{AbsoluteLength, LengthPercentage, NoCalcLength};
    if let Some(value) = value.strip_suffix("px") {
        let val: f32 = value.parse().ok()?;
        return Some(LengthPercentage::Length(NoCalcLength::Absolute(
            AbsoluteLength::Px(val),
        )));
    }

    if let Some(value) = value.strip_suffix("%") {
        let val: f32 = value.parse().ok()?;
        return Some(LengthPercentage::Percentage(Percentage(val / 100.0)));
    }

    let val: f32 = value.parse().ok().filter(filter_fn)?;
    Some(LengthPercentage::Length(NoCalcLength::Absolute(
        AbsoluteLength::Px(val),
    )))
}