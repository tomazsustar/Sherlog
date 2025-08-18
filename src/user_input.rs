// src/user_input.rs

// +0D 00:00:00.000
pub fn parse_duration(s: &str) -> chrono::Duration {
    let re = regex::Regex::new(
        r"^([+-])?\s*(?:(\d+)D\s*)?(?:(\d{1,2}):)?(?:(\d{1,2}):)?(\d{1,2})(?:\.(\d{1,3}))?$"
    ).unwrap();
    if let Some(caps) = re.captures(s.trim()) {
        let sign = match caps.get(1).map(|m| m.as_str()) {
            Some("-") => -1,
            _ => 1,
        };
        let days: i64 = caps.get(2).map_or(0, |m| m.as_str().parse().unwrap_or(0));
        let hours: i64 = caps.get(3).map_or(0, |m| m.as_str().parse().unwrap_or(0));
        let minutes: i64 = caps.get(4).map_or(0, |m| m.as_str().parse().unwrap_or(0));
        let seconds: i64 = caps.get(5).map_or(0, |m| m.as_str().parse().unwrap_or(0));
        let millis: i64 = caps.get(6).map_or(0, |m| {
            let ms = m.as_str();
            // Pad to 3 digits if needed
            format!("{:0<3}", ms).parse().unwrap_or(0)
        });

        let total_millis =
            (((days * 24 + hours) * 60 + minutes) * 60 + seconds) * 1000 + millis;
        chrono::Duration::milliseconds(sign * total_millis)
    } else {
        log::info!("timeshift invalid");
        chrono::Duration::zero()
    }
}

pub fn format_duration(duration: chrono::Duration) -> String {
    let sign = if duration.num_milliseconds() < 0 { '-' } else { '+' };
    let mut ms = i64::abs(duration.num_milliseconds());
    let days = ms / 86_400_000;
    ms -= days * 86_400_000;
    let hours = ms / 3_600_000;
    ms -= hours * 3_600_000;
    let minutes = ms / 60_000;
    ms -= minutes * 60_000;
    let seconds = ms / 1000;
    ms -= seconds * 1000;
    let milliseconds = ms;
    format!(
        "{}{}D {:02}:{:02}:{:02}.{:03}",
        sign, days, hours, minutes, seconds, milliseconds
    )
}