// src/formatting.rs

use crate::model::LogLevel;

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

pub fn short_severity(sev: &LogLevel) -> &'static str {
    match sev {
        LogLevel::Critical => "CRI",
        LogLevel::Error => "ERR",
        LogLevel::Warning => "WRN",
        LogLevel::Info => "INF",
        LogLevel::Debug => "DBG",
        LogLevel::Trace => "TRC",
    }
}

// Timezone information structure
#[derive(Clone)]
pub struct TimezoneInfo {
    pub name: &'static str,
    pub offset: chrono::Duration,
}

// Common timezones with their UTC offsets (standard time, not DST)
// Ordered with negative offsets first, UTC in the middle, then positive offsets
pub fn get_timezones() -> Vec<TimezoneInfo> {
    vec![
        TimezoneInfo { name: "UTC-12:00 (Baker Island)", offset: chrono::Duration::hours(-12) },
        TimezoneInfo { name: "UTC-11:00 (SST)", offset: chrono::Duration::hours(-11) },
        TimezoneInfo { name: "UTC-10:00 (HST)", offset: chrono::Duration::hours(-10) },
        TimezoneInfo { name: "UTC-09:00 (AKST)", offset: chrono::Duration::hours(-9) },
        TimezoneInfo { name: "UTC-08:00 (PST)", offset: chrono::Duration::hours(-8) },
        TimezoneInfo { name: "UTC-07:00 (MST/PDT)", offset: chrono::Duration::hours(-7) },
        TimezoneInfo { name: "UTC-06:00 (CST/MDT)", offset: chrono::Duration::hours(-6) },
        TimezoneInfo { name: "UTC-05:00 (EST/CDT)", offset: chrono::Duration::hours(-5) },
        TimezoneInfo { name: "UTC-04:00 (AST/EDT)", offset: chrono::Duration::hours(-4) },
        TimezoneInfo { name: "UTC-03:00 (ART/ADT)", offset: chrono::Duration::hours(-3) },
        TimezoneInfo { name: "UTC-02:00 (BRST)", offset: chrono::Duration::hours(-2) },
        TimezoneInfo { name: "UTC-01:00 (AZOST)", offset: chrono::Duration::hours(-1) },
        TimezoneInfo { name: "UTC", offset: chrono::Duration::zero() },
        TimezoneInfo { name: "UTC+01:00 (CET/WAT)", offset: chrono::Duration::hours(1) },
        TimezoneInfo { name: "UTC+02:00 (CEST/EET)", offset: chrono::Duration::hours(2) },
        TimezoneInfo { name: "UTC+03:00 (EEST/MSK)", offset: chrono::Duration::hours(3) },
        TimezoneInfo { name: "UTC+04:00 (GST)", offset: chrono::Duration::hours(4) },
        TimezoneInfo { name: "UTC+05:00 (PKT)", offset: chrono::Duration::hours(5) },
        TimezoneInfo { name: "UTC+05:30 (IST)", offset: chrono::Duration::hours(5) + chrono::Duration::minutes(30) },
        TimezoneInfo { name: "UTC+06:00 (BST)", offset: chrono::Duration::hours(6) },
        TimezoneInfo { name: "UTC+07:00 (ICT)", offset: chrono::Duration::hours(7) },
        TimezoneInfo { name: "UTC+08:00 (CST/AWST)", offset: chrono::Duration::hours(8) },
        TimezoneInfo { name: "UTC+09:00 (JST)", offset: chrono::Duration::hours(9) },
        TimezoneInfo { name: "UTC+09:30 (ACST)", offset: chrono::Duration::hours(9) + chrono::Duration::minutes(30) },
        TimezoneInfo { name: "UTC+10:00 (AEST)", offset: chrono::Duration::hours(10) },
        TimezoneInfo { name: "UTC+11:00 (AEDT/SBT)", offset: chrono::Duration::hours(11) },
        TimezoneInfo { name: "UTC+12:00 (NZST)", offset: chrono::Duration::hours(12) },
        TimezoneInfo { name: "UTC+13:00 (NZDT/TOT)", offset: chrono::Duration::hours(13) },
        TimezoneInfo { name: "UTC+14:00 (LINT)", offset: chrono::Duration::hours(14) },
    ]
}