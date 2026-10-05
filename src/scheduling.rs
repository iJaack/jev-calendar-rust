use crate::{array, epoch, iso, number, text};
use anyhow::{Result, anyhow};
use chrono::{Datelike, Duration, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use serde_json::{Value, json};

pub fn valid_zone(z: &str) -> bool {
    z.parse::<Tz>().is_ok()
}
pub fn date(ms: i64, z: &str) -> Result<String> {
    let zone: Tz = z.parse().map_err(|_| anyhow!("Invalid time zone"))?;
    Ok(Utc
        .timestamp_millis_opt(ms)
        .single()
        .ok_or_else(|| anyhow!("Invalid date"))?
        .with_timezone(&zone)
        .format("%Y-%m-%d")
        .to_string())
}
pub fn shift(day: &str, n: i64) -> Result<String> {
    Ok(
        (chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")? + Duration::days(n))
            .format("%Y-%m-%d")
            .to_string(),
    )
}
pub fn weekday(day: &str) -> Result<u32> {
    Ok(chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")?
        .weekday()
        .num_days_from_sunday())
}
pub fn monday(day: &str) -> Result<String> {
    shift(day, -i64::from((weekday(day)? + 6) % 7))
}
pub fn zoned(day: &str, hour: f64, z: &str) -> Result<i64> {
    let zone: Tz = z.parse().map_err(|_| anyhow!("Invalid time zone"))?;
    let target = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")?
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow!("Invalid date"))?
        + Duration::milliseconds((hour * 3_600_000.0) as i64);
    // Match the existing ICU offset iteration, including ambiguous local hours.
    let target_ms = target.and_utc().timestamp_millis();
    let mut guess = target_ms;
    for _ in 0..3 {
        let local = Utc
            .timestamp_millis_opt(guess)
            .single()
            .ok_or_else(|| anyhow!("Invalid date"))?
            .with_timezone(&zone);
        let represented = local.naive_local().and_utc().timestamp_millis();
        guess += target_ms - represented;
    }
    Ok(guess)
}
pub fn overlaps(a: &str, b: &str, v: &Value) -> bool {
    match (
        epoch(a),
        epoch(b),
        epoch(&text(v, "start")),
        epoch(&text(v, "end")),
    ) {
        (Some(a), Some(b), Some(x), Some(y)) => a < y && b > x,
        _ => false,
    }
}
pub fn constraint(start: &str, end: &str, busy: &[Value], buffer: f64) -> Option<&'static str> {
    let (a, b) = match (epoch(start), epoch(end)) {
        (Some(a), Some(b)) if b > a => (a, b),
        _ => return Some("invalid_interval"),
    };
    for item in busy {
        if let (Some(x), Some(y)) = (epoch(&text(item, "start")), epoch(&text(item, "end"))) {
            let before = buffer.max(0.0).max(number(item, "beforeBufferMinutes")) * 60_000.0;
            let after = buffer.max(0.0).max(number(item, "afterBufferMinutes")) * 60_000.0;
            if (a as f64) < y as f64 + after && (b as f64) > x as f64 - before {
                return Some(if a < y && b > x {
                    "busy"
                } else {
                    "required_buffer"
                });
            }
        }
    }
    None
}
pub fn candidates(v: &Value) -> Result<Vec<Value>> {
    let day = text(v, "day");
    let zone = if v["zone"].is_string() {
        text(v, "zone")
    } else {
        "UTC".into()
    };
    let duration = (number(v, "durationMinutes") * 60_000.0) as i64;
    let buffer = (number(v, "bufferMinutes").max(0.0) * 60_000.0) as i64;
    if duration <= 0 {
        return Ok(vec![]);
    }
    let mut cursor = zoned(&day, number(&v["hours"], "start"), &zone)?;
    let end = zoned(&day, number(&v["hours"], "end"), &zone)?;
    let cutoff = v["notBefore"].as_i64().unwrap_or(i64::MIN);
    let limit = v["limit"].as_u64().unwrap_or(12) as usize;
    let mut intervals: Vec<_> = array(&v["busy"])
        .iter()
        .filter_map(|x| Some((epoch(&text(x, "start"))?, epoch(&text(x, "end"))?)))
        .collect();
    intervals.sort_by_key(|x| x.0);
    let mut out = vec![];
    for (a, b) in intervals
        .into_iter()
        .chain(std::iter::once((end + buffer, i64::MIN)))
    {
        while cursor + duration <= (a - buffer).min(end) && out.len() < limit {
            if cursor >= cutoff {
                out.push(json!({"start":iso(cursor),"end":iso(cursor+duration)}))
            }
            cursor += duration;
        }
        cursor = cursor.max(b.saturating_add(buffer));
    }
    Ok(out)
}
pub fn bucket(start: &str, z: &str) -> Result<String> {
    let zone: Tz = z.parse()?;
    let d = Utc
        .timestamp_millis_opt(epoch(start).ok_or_else(|| anyhow!("Invalid date"))?)
        .single()
        .ok_or_else(|| anyhow!("Invalid date"))?
        .with_timezone(&zone);
    Ok(format!("{}:{}", d.format("%a"), d.hour() / 2))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clamps_to_workday_end() {
        let v = json!({"day":"2030-10-07","zone":"Europe/Rome","durationMinutes":30,"bufferMinutes":10,"hours":{"start":9,"end":18},"limit":96,"busy":[{"start":"2030-10-07T20:00:00.000Z","end":"2030-10-07T21:00:00.000Z"}]});
        let c = candidates(&v).unwrap();
        assert!(c.iter().all(|s| epoch(&text(s, "end")).unwrap()
            <= zoned("2030-10-07", 18.0, "Europe/Rome").unwrap()));
    }
    #[test]
    fn boundary_includes_exact_notice() {
        let t = zoned("2030-10-07", 9.0, "Europe/Rome").unwrap();
        assert_eq!(date(t, "Asia/Tokyo").unwrap(), "2030-10-07");
        assert!(constraint(&iso(t), &iso(t), &[], 0.0).is_some());
    }
}
