use wasm_bindgen::JsValue;

pub fn now_ms() -> f64 {
    js_sys::Date::now()
}

fn date(ms: i64) -> js_sys::Date {
    js_sys::Date::new(&JsValue::from_f64(ms as f64))
}

/// "3:07 PM"
pub fn clock(ms: i64) -> String {
    let d = date(ms);
    let hours = d.get_hours();
    let suffix = if hours >= 12 { "PM" } else { "AM" };
    let hour = match hours % 12 {
        0 => 12,
        h => h,
    };
    format!("{hour}:{:02} {suffix}", d.get_minutes())
}

fn day_key(ms: i64) -> (u32, u32, u32) {
    let d = date(ms);
    (d.get_full_year(), d.get_month(), d.get_date())
}

pub fn same_day(a: i64, b: i64) -> bool {
    day_key(a) == day_key(b)
}

/// "Today", "Yesterday" or "Mar 4, 2026"
pub fn day_label(ms: i64) -> String {
    let now = now_ms() as i64;
    if same_day(ms, now) {
        return "Today".into();
    }
    if same_day(ms, now - 24 * 60 * 60 * 1000) {
        return "Yesterday".into();
    }
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let (year, month, day) = day_key(ms);
    format!("{} {day}, {year}", MONTHS[month as usize % 12])
}

/// "Today at 3:07 PM"
pub fn stamp(ms: i64) -> String {
    format!("{} at {}", day_label(ms), clock(ms))
}

pub fn initials(name: &str) -> String {
    let mut words = name.split_whitespace();
    let first = words.next().and_then(|w| w.chars().next());
    let second = words.next().and_then(|w| w.chars().next());
    match (first, second) {
        (Some(a), Some(b)) => format!("{a}{b}").to_uppercase(),
        (Some(a), None) => a.to_uppercase().to_string(),
        _ => "?".into(),
    }
}

pub fn avatar_style(hue: i64) -> String {
    format!("background: hsl({hue} 55% 42%)")
}
