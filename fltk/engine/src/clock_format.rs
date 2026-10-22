// SPDX-License-Identifier: GPL-3.0-or-later
//! Read DomeD's current clock choice when formatting a reminder.
pub fn time_text(at: chrono::NaiveTime) -> String {
    use mailrs_domain::calendar::clock::{ClockFormat, format_time};
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        });
    let twelve_hour = std::fs::read(config.join("dome/clock.json"))
        .ok()
        .and_then(|v| serde_json::from_slice::<serde_json::Value>(&v).ok())
        .and_then(|v| v["twelve_hour"].as_bool())
        .unwrap_or(false);
    format_time(
        at,
        if twelve_hour {
            ClockFormat::Hour12
        } else {
            ClockFormat::Hour24
        },
        mailrs_domain::translate::date_locale(),
    )
}
