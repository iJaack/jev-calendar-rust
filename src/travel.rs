//! Pure, bounded location inference from normalized TripIt events.

use crate::{array, epoch, iso, number, scheduling, text};
use anyhow::Result;
use serde_json::{Value, json};
use std::sync::LazyLock;

const DAY: i64 = 86_400_000;

// Embedded airport/city data: no geocoding or network access. Cities may have
// several airports; matching a hotel city is accepted only for a unique zone.
const AIRPORTS: &[(&str, &str, &str)] = &[
    ("FCO CIA", "Rome", "Europe/Rome"),
    ("MXP LIN BGY", "Milan", "Europe/Rome"),
    ("VCE", "Venice", "Europe/Rome"),
    ("NAP", "Naples", "Europe/Rome"),
    ("BLQ", "Bologna", "Europe/Rome"),
    ("FLR", "Florence", "Europe/Rome"),
    ("PSA", "Pisa", "Europe/Rome"),
    ("TRN", "Turin", "Europe/Rome"),
    ("CTA", "Catania", "Europe/Rome"),
    ("PMO", "Palermo", "Europe/Rome"),
    ("BRI", "Bari", "Europe/Rome"),
    ("CAG", "Cagliari", "Europe/Rome"),
    ("OLB", "Olbia", "Europe/Rome"),
    ("ATL", "Atlanta", "America/New_York"),
    ("JFK EWR LGA", "New York", "America/New_York"),
    ("BOS", "Boston", "America/New_York"),
    ("MIA FLL", "Miami", "America/New_York"),
    ("MCO", "Orlando", "America/New_York"),
    ("TPA", "Tampa", "America/New_York"),
    ("CLT", "Charlotte", "America/New_York"),
    ("PHL", "Philadelphia", "America/New_York"),
    ("IAD DCA BWI", "Washington", "America/New_York"),
    ("DTW", "Detroit", "America/Detroit"),
    ("SFO OAK SJC", "San Francisco", "America/Los_Angeles"),
    ("LAX BUR SNA ONT", "Los Angeles", "America/Los_Angeles"),
    ("SEA", "Seattle", "America/Los_Angeles"),
    ("LAS", "Las Vegas", "America/Los_Angeles"),
    ("SAN", "San Diego", "America/Los_Angeles"),
    ("PDX", "Portland", "America/Los_Angeles"),
    ("SMF", "Sacramento", "America/Los_Angeles"),
    ("ORD MDW", "Chicago", "America/Chicago"),
    ("DFW DAL", "Dallas", "America/Chicago"),
    ("IAH HOU", "Houston", "America/Chicago"),
    ("MSP", "Minneapolis", "America/Chicago"),
    ("AUS", "Austin", "America/Chicago"),
    ("BNA", "Nashville", "America/Chicago"),
    ("STL", "St Louis", "America/Chicago"),
    ("MCI", "Kansas City", "America/Chicago"),
    ("DEN", "Denver", "America/Denver"),
    ("SLC", "Salt Lake City", "America/Denver"),
    ("PHX", "Phoenix", "America/Phoenix"),
    ("HNL OGG", "Honolulu", "Pacific/Honolulu"),
    ("ANC", "Anchorage", "America/Anchorage"),
    ("CUN", "Cancun", "America/Cancun"),
    ("MEX NLU", "Mexico City", "America/Mexico_City"),
    ("GDL", "Guadalajara", "America/Mexico_City"),
    ("MTY", "Monterrey", "America/Monterrey"),
    ("YYZ YTZ", "Toronto", "America/Toronto"),
    ("YUL", "Montreal", "America/Toronto"),
    ("YVR", "Vancouver", "America/Vancouver"),
    ("YYC", "Calgary", "America/Edmonton"),
    ("YEG", "Edmonton", "America/Edmonton"),
    ("YOW", "Ottawa", "America/Toronto"),
    ("GRU CGH VCP", "Sao Paulo", "America/Sao_Paulo"),
    ("GIG SDU", "Rio de Janeiro", "America/Sao_Paulo"),
    ("BSB", "Brasilia", "America/Sao_Paulo"),
    ("BOG", "Bogota", "America/Bogota"),
    ("LIM", "Lima", "America/Lima"),
    ("SCL", "Santiago", "America/Santiago"),
    ("EZE AEP", "Buenos Aires", "America/Argentina/Buenos_Aires"),
    ("PTY", "Panama City", "America/Panama"),
    ("SJO", "San Jose", "America/Costa_Rica"),
    ("PUJ", "Punta Cana", "America/Santo_Domingo"),
    ("SJU", "San Juan", "America/Puerto_Rico"),
    ("LHR LGW STN LTN LCY", "London", "Europe/London"),
    ("MAN", "Manchester", "Europe/London"),
    ("EDI", "Edinburgh", "Europe/London"),
    ("DUB", "Dublin", "Europe/Dublin"),
    ("CDG ORY", "Paris", "Europe/Paris"),
    ("NCE", "Nice", "Europe/Paris"),
    ("LYS", "Lyon", "Europe/Paris"),
    ("AMS", "Amsterdam", "Europe/Amsterdam"),
    ("FRA", "Frankfurt", "Europe/Berlin"),
    ("MUC", "Munich", "Europe/Berlin"),
    ("BER", "Berlin", "Europe/Berlin"),
    ("DUS", "Dusseldorf", "Europe/Berlin"),
    ("HAM", "Hamburg", "Europe/Berlin"),
    ("ZRH", "Zurich", "Europe/Zurich"),
    ("GVA", "Geneva", "Europe/Zurich"),
    ("MAD", "Madrid", "Europe/Madrid"),
    ("BCN", "Barcelona", "Europe/Madrid"),
    ("PMI", "Palma", "Europe/Madrid"),
    ("AGP", "Malaga", "Europe/Madrid"),
    ("ALC", "Alicante", "Europe/Madrid"),
    ("LIS", "Lisbon", "Europe/Lisbon"),
    ("OPO", "Porto", "Europe/Lisbon"),
    ("BRU", "Brussels", "Europe/Brussels"),
    ("VIE", "Vienna", "Europe/Vienna"),
    ("CPH", "Copenhagen", "Europe/Copenhagen"),
    ("OSL", "Oslo", "Europe/Oslo"),
    ("ARN", "Stockholm", "Europe/Stockholm"),
    ("HEL", "Helsinki", "Europe/Helsinki"),
    ("WAW", "Warsaw", "Europe/Warsaw"),
    ("PRG", "Prague", "Europe/Prague"),
    ("BUD", "Budapest", "Europe/Budapest"),
    ("ATH", "Athens", "Europe/Athens"),
    ("HER", "Heraklion", "Europe/Athens"),
    ("IST SAW", "Istanbul", "Europe/Istanbul"),
    ("AYT", "Antalya", "Europe/Istanbul"),
    ("SVO DME VKO", "Moscow", "Europe/Moscow"),
    ("LED", "Saint Petersburg", "Europe/Moscow"),
    ("DXB DWC", "Dubai", "Asia/Dubai"),
    ("AUH", "Abu Dhabi", "Asia/Dubai"),
    ("DOH", "Doha", "Asia/Qatar"),
    ("RUH", "Riyadh", "Asia/Riyadh"),
    ("JED", "Jeddah", "Asia/Riyadh"),
    ("KWI", "Kuwait", "Asia/Kuwait"),
    ("BAH", "Bahrain", "Asia/Bahrain"),
    ("MCT", "Muscat", "Asia/Muscat"),
    ("TLV", "Tel Aviv", "Asia/Jerusalem"),
    ("AMM", "Amman", "Asia/Amman"),
    ("DEL", "Delhi", "Asia/Kolkata"),
    ("BOM", "Mumbai", "Asia/Kolkata"),
    ("BLR", "Bengaluru", "Asia/Kolkata"),
    ("HYD", "Hyderabad", "Asia/Kolkata"),
    ("MAA", "Chennai", "Asia/Kolkata"),
    ("CCU", "Kolkata", "Asia/Kolkata"),
    ("CMB", "Colombo", "Asia/Colombo"),
    ("DAC", "Dhaka", "Asia/Dhaka"),
    ("SIN", "Singapore", "Asia/Singapore"),
    ("HKG", "Hong Kong", "Asia/Hong_Kong"),
    ("NRT HND", "Tokyo", "Asia/Tokyo"),
    ("KIX ITM", "Osaka", "Asia/Tokyo"),
    ("FUK", "Fukuoka", "Asia/Tokyo"),
    ("CTS", "Sapporo", "Asia/Tokyo"),
    ("ICN GMP", "Seoul", "Asia/Seoul"),
    ("PUS", "Busan", "Asia/Seoul"),
    ("PEK PKX", "Beijing", "Asia/Shanghai"),
    ("PVG SHA", "Shanghai", "Asia/Shanghai"),
    ("CAN", "Guangzhou", "Asia/Shanghai"),
    ("SZX", "Shenzhen", "Asia/Shanghai"),
    ("CTU TFU", "Chengdu", "Asia/Shanghai"),
    ("CKG", "Chongqing", "Asia/Shanghai"),
    ("XIY", "Xian", "Asia/Shanghai"),
    ("KMG", "Kunming", "Asia/Shanghai"),
    ("HGH", "Hangzhou", "Asia/Shanghai"),
    ("WUH", "Wuhan", "Asia/Shanghai"),
    ("NKG", "Nanjing", "Asia/Shanghai"),
    ("CSX", "Changsha", "Asia/Shanghai"),
    ("CGO", "Zhengzhou", "Asia/Shanghai"),
    ("XMN", "Xiamen", "Asia/Shanghai"),
    ("TAO", "Qingdao", "Asia/Shanghai"),
    ("TSN", "Tianjin", "Asia/Shanghai"),
    ("HAK", "Haikou", "Asia/Shanghai"),
    ("SYX", "Sanya", "Asia/Shanghai"),
    ("TPE TSA", "Taipei", "Asia/Taipei"),
    ("MFM", "Macau", "Asia/Macau"),
    ("BKK DMK", "Bangkok", "Asia/Bangkok"),
    ("HKT", "Phuket", "Asia/Bangkok"),
    ("KUL", "Kuala Lumpur", "Asia/Kuala_Lumpur"),
    ("CGK", "Jakarta", "Asia/Jakarta"),
    ("DPS", "Denpasar", "Asia/Makassar"),
    ("MNL", "Manila", "Asia/Manila"),
    ("CEB", "Cebu", "Asia/Manila"),
    ("SGN", "Ho Chi Minh City", "Asia/Ho_Chi_Minh"),
    ("HAN", "Hanoi", "Asia/Ho_Chi_Minh"),
    ("SYD", "Sydney", "Australia/Sydney"),
    ("MEL", "Melbourne", "Australia/Melbourne"),
    ("BNE", "Brisbane", "Australia/Brisbane"),
    ("PER", "Perth", "Australia/Perth"),
    ("ADL", "Adelaide", "Australia/Adelaide"),
    ("AKL", "Auckland", "Pacific/Auckland"),
    ("CHC", "Christchurch", "Pacific/Auckland"),
    ("NAN", "Nadi", "Pacific/Fiji"),
    ("CAI", "Cairo", "Africa/Cairo"),
    ("JNB", "Johannesburg", "Africa/Johannesburg"),
    ("CPT", "Cape Town", "Africa/Johannesburg"),
    ("ADD", "Addis Ababa", "Africa/Addis_Ababa"),
    ("NBO", "Nairobi", "Africa/Nairobi"),
    ("CMN", "Casablanca", "Africa/Casablanca"),
    ("RAK", "Marrakech", "Africa/Casablanca"),
    ("LOS", "Lagos", "Africa/Lagos"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Interval {
    pub start: i64,
    pub end: i64,
}

#[derive(Debug, PartialEq, Eq)]
struct Leg {
    origin: String,
    destination: String,
    origin_zone: String,
    destination_zone: String,
    interval: Interval,
}

#[derive(Debug, PartialEq, Eq)]
struct Stay {
    zone: String,
    interval: Interval,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DayPlan {
    pub zone: String,
    pub source: String,
    pub windows: Vec<Interval>,
    pub blocked: Vec<Interval>,
    pub zones: Vec<String>,
    pub affected: bool,
}

pub(crate) fn enabled(flag: Option<&str>) -> bool {
    !flag.is_some_and(|s| {
        matches!(
            s.trim().to_ascii_lowercase().as_str(),
            "off" | "0" | "false"
        )
    })
}

fn airport(code: &str) -> Option<&'static str> {
    AIRPORTS
        .iter()
        .find(|(codes, _, _)| codes.split_whitespace().any(|c| c == code))
        .map(|(_, _, z)| *z)
}
fn tripit(event: &Value) -> bool {
    text(event, "calendarName")
        .to_ascii_lowercase()
        .contains("tripit")
}
fn interval(event: &Value) -> Option<Interval> {
    let start = epoch(&text(event, "start"))?;
    let end = epoch(&text(event, "end"))?;
    (end > start).then_some(Interval { start, end })
}
fn flight_codes(event: &Value) -> Option<(String, String)> {
    static FLIGHT: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?i)^\s*[a-z0-9]{2,3}\s*\d+[a-z]?\s+([a-z]{3})\s+to\s+([a-z]{3})\s*$")
            .expect("static flight regex")
    });
    if !tripit(event) {
        return None;
    }
    let summary = text(event, "summary");
    let captures = FLIGHT.captures(&summary)?;
    Some((
        captures[1].to_ascii_uppercase(),
        captures[2].to_ascii_uppercase(),
    ))
}
fn parse_legs(events: &[Value]) -> Vec<Leg> {
    let mut legs: Vec<_> = events
        .iter()
        .filter_map(|event| {
            let (origin, destination) = flight_codes(event)?;
            Some(Leg {
                origin_zone: airport(&origin)?.into(),
                destination_zone: airport(&destination)?.into(),
                origin,
                destination,
                interval: interval(event)?,
            })
        })
        .collect();
    legs.sort_by_key(|leg| leg.interval.start);
    legs.dedup();
    legs
}
fn location_zone(location: &str) -> Option<String> {
    static CODE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\(([A-Z]{3})\)").expect("static airport regex"));
    let upper = location.to_ascii_uppercase();
    let mut zones: Vec<&str> = CODE
        .captures_iter(&upper)
        .filter_map(|c| airport(&c[1]))
        .collect();
    if zones.is_empty() {
        let words: Vec<_> = location
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .map(str::to_lowercase)
            .collect();
        for (_, city, zone) in AIRPORTS {
            let city: Vec<_> = city.split_whitespace().map(str::to_lowercase).collect();
            if words.windows(city.len()).any(|w| w == city) {
                zones.push(zone);
            }
        }
    }
    zones.sort();
    zones.dedup();
    (zones.len() == 1).then(|| zones[0].to_owned())
}
fn parse_stays(events: &[Value]) -> Vec<Stay> {
    let legs = parse_legs(events);
    let mut markers: Vec<_> = events
        .iter()
        .filter(|e| tripit(e))
        .filter_map(|e| {
            let summary = text(e, "summary").trim().to_lowercase();
            let (check_in, name) = if let Some(name) = summary.strip_prefix("check-in:") {
                (true, name.trim().to_owned())
            } else {
                (false, summary.strip_prefix("check-out:")?.trim().to_owned())
            };
            Some((interval(e)?.start, check_in, name, text(e, "location")))
        })
        .collect();
    markers.sort_by_key(|m| m.0);
    let mut stays = vec![];
    for (i, (start, check_in, name, location)) in markers.iter().enumerate() {
        if !check_in {
            continue;
        }
        let Some((end, next_is_check_in, _, _)) = markers[i + 1..].iter().find(|m| &m.2 == name)
        else {
            continue;
        };
        // A second check-in before check-out makes the pair ambiguous.
        if *next_is_check_in {
            continue;
        }
        let zone = location_zone(location).or_else(|| {
            legs.iter()
                .filter(|l| l.interval.end <= *start)
                .max_by_key(|l| l.interval.end)
                .map(|l| l.destination_zone.clone())
        });
        if let Some(zone) = zone {
            stays.push(Stay {
                zone,
                interval: Interval {
                    start: *start,
                    end: *end,
                },
            });
        }
    }
    stays
}
fn intersects(a: &Interval, b: &Interval) -> bool {
    a.start < b.end && a.end > b.start
}
fn day_interval(day: &str, zone: &str) -> Result<Interval> {
    Ok(Interval {
        start: scheduling::zoned(day, 0.0, zone)?,
        end: scheduling::zoned(day, 24.0, zone)?,
    })
}

// Each ground segment is bounded by actual flight instants. A date may have
// multiple segments/zones, including both sides of a same-day journey.
pub(crate) fn infer_day_plan(request: &Value, events: &[Value]) -> Result<DayPlan> {
    let day = text(request, "day");
    let fallback = text(request, "zone");
    let legs = parse_legs(events);
    let stays = parse_stays(events);
    let mut blocked: Vec<_> = events
        .iter()
        .filter(|e| flight_codes(e).is_some())
        .filter_map(interval)
        .collect();
    let mut ground: Vec<(String, Interval)> = vec![];
    let mut doubts: Vec<(Interval, Vec<String>)> = vec![];
    if let Some(first) = legs.first() {
        let departure_day = scheduling::date(first.interval.start, &first.origin_zone)?;
        ground.push((
            first.origin_zone.clone(),
            Interval {
                start: scheduling::zoned(&departure_day, 0.0, &first.origin_zone)?,
                end: first.interval.start,
            },
        ));
    }
    for (index, leg) in legs.iter().enumerate() {
        let next = legs.get(index + 1);
        let end = next.map_or(leg.interval.end + 30 * DAY, |n| n.interval.start);
        let connection = next.is_some_and(|n| {
            n.origin == leg.destination
                && end >= leg.interval.end
                && end - leg.interval.end <= DAY
                && !events.iter().any(|event| {
                    // Rule 4 needs a check-in, even when the checkout marker
                    // is outside the fetched range or missing altogether.
                    tripit(event)
                        && text(event, "summary")
                            .trim()
                            .to_ascii_lowercase()
                            .starts_with("check-in:")
                        && interval(event)
                            .is_some_and(|i| i.start >= leg.interval.end && i.start < end)
                        && location_zone(&text(event, "location"))
                            .unwrap_or_else(|| leg.destination_zone.clone())
                            == leg.destination_zone
                })
        });
        if connection {
            blocked.push(Interval {
                start: leg.interval.start,
                end: next.expect("connection").interval.end,
            });
        } else if end > leg.interval.end {
            ground.push((
                leg.destination_zone.clone(),
                Interval {
                    start: leg.interval.end,
                    end,
                },
            ));
        }
        if let Some(next) = next
            && (next.interval.start < leg.interval.end || next.origin != leg.destination)
        {
            doubts.push((
                Interval {
                    start: leg.interval.start.min(next.interval.start),
                    end: leg.interval.end.max(next.interval.end),
                },
                vec![
                    leg.origin_zone.clone(),
                    leg.destination_zone.clone(),
                    next.origin_zone.clone(),
                    next.destination_zone.clone(),
                ],
            ));
        }
    }
    // An unrecognized airport breaks the location chain. Its duration remains
    // blocked and its unknown destination cannot inherit an earlier arrival.
    for event in events {
        let Some((origin, destination)) = flight_codes(event) else {
            continue;
        };
        if airport(&origin).is_some() && airport(&destination).is_some() {
            continue;
        }
        if let Some(interval) = interval(event) {
            let end = legs
                .iter()
                .filter(|l| l.interval.start >= interval.end)
                .map(|l| l.interval.end)
                .min()
                .unwrap_or(interval.end + 30 * DAY);
            doubts.push((
                Interval {
                    start: interval.start,
                    end,
                },
                vec![fallback.clone()],
            ));
        }
    }
    for stay in &stays {
        // Flights prevail in transit, and contrary hotel evidence makes the
        // entire overlapping local date conservative rather than blending zones.
        for leg in &legs {
            if intersects(&stay.interval, &leg.interval)
                && stay.zone != leg.origin_zone
                && stay.zone != leg.destination_zone
            {
                doubts.push((
                    Interval {
                        start: stay.interval.start.max(leg.interval.start),
                        end: stay.interval.end.min(leg.interval.end),
                    },
                    vec![
                        stay.zone.clone(),
                        leg.origin_zone.clone(),
                        leg.destination_zone.clone(),
                    ],
                ));
            }
        }
        for (zone, interval) in &ground {
            if zone != &stay.zone && intersects(interval, &stay.interval) {
                doubts.push((
                    Interval {
                        start: interval.start.max(stay.interval.start),
                        end: interval.end.min(stay.interval.end),
                    },
                    vec![zone.clone(), stay.zone.clone()],
                ));
            }
        }
    }
    for (i, stay) in stays.iter().enumerate() {
        for other in &stays[i + 1..] {
            if stay.zone != other.zone && intersects(&stay.interval, &other.interval) {
                doubts.push((
                    Interval {
                        start: stay.interval.start.max(other.interval.start),
                        end: stay.interval.end.min(other.interval.end),
                    },
                    vec![stay.zone.clone(), other.zone.clone()],
                ));
            }
        }
        ground.push((stay.zone.clone(), stay.interval.clone()));
    }
    let fallback_day = day_interval(&day, &fallback)?;
    let ambiguous = doubts.iter().any(|(interval, zones)| {
        intersects(interval, &fallback_day)
            || zones
                .iter()
                .any(|z| day_interval(&day, z).is_ok_and(|d| intersects(interval, &d)))
    });
    let mut windows = vec![];
    let mut zones = vec![];
    let mut primary = None;
    for (zone, interval) in &ground {
        if !intersects(interval, &day_interval(&day, zone)?) {
            continue;
        }
        if !zones.contains(zone) {
            zones.push(zone.clone());
        }
        let window = Interval {
            start: scheduling::zoned(&day, number(&request["hours"], "start"), zone)?
                .max(interval.start),
            end: scheduling::zoned(&day, number(&request["hours"], "end"), zone)?.min(interval.end),
        };
        if window.start < window.end {
            if primary.is_none() {
                primary = Some(zone.clone());
            }
            windows.push(window);
        }
    }
    let affected =
        !zones.is_empty() || blocked.iter().any(|b| intersects(b, &fallback_day)) || ambiguous;
    let inferred = !ambiguous && !zones.is_empty();
    let zone = if inferred {
        primary
            .or_else(|| zones.last().cloned())
            .expect("ground zone")
    } else {
        fallback.clone()
    };
    if !inferred {
        zones = vec![fallback];
        windows = vec![Interval {
            start: scheduling::zoned(&day, number(&request["hours"], "start"), &zone)?,
            end: scheduling::zoned(&day, number(&request["hours"], "end"), &zone)?,
        }];
    }
    // No ground location during a journey: fallback metadata, no transit slots.
    // Every transit instant is still explicitly blocked, even without free/busy.
    if affected && !(1..=5).contains(&scheduling::weekday(&day)?) {
        windows.clear();
    }
    windows.sort_by_key(|w| w.start);
    let mut merged: Vec<Interval> = vec![];
    for window in windows {
        if let Some(last) = merged.last_mut()
            && window.start <= last.end
        {
            last.end = last.end.max(window.end);
            continue;
        }
        merged.push(window);
    }
    if affected {
        // Remove transit itself from the plan; the slot generator also applies
        // the configured buffer to these blocks on both sides.
        for block in &blocked {
            merged = merged
                .into_iter()
                .flat_map(|w| {
                    if !intersects(&w, block) {
                        return vec![w];
                    }
                    let mut parts = vec![];
                    if w.start < block.start {
                        parts.push(Interval {
                            start: w.start,
                            end: block.start,
                        });
                    }
                    if w.end > block.end {
                        parts.push(Interval {
                            start: block.end,
                            end: w.end,
                        });
                    }
                    parts
                })
                .collect();
        }
    }
    Ok(DayPlan {
        zone,
        source: if inferred {
            "travel".into()
        } else {
            text(request, "source")
        },
        windows: merged,
        blocked,
        zones,
        affected,
    })
}

pub(crate) fn slot_path(request: &Value, events: &[Value], flag: Option<&str>) -> Result<Value> {
    let plan = if enabled(flag) {
        Some(infer_day_plan(request, events)?)
    } else {
        None
    };
    let mut candidates = vec![];
    let (zone, source) = if let Some(plan) = plan.filter(|p| p.affected) {
        let mut input = request.clone();
        let mut busy = array(&input["busy"]);
        busy.extend(
            plan.blocked
                .iter()
                .map(|b| json!({"start":iso(b.start),"end":iso(b.end)})),
        );
        input["busy"] = json!(busy);
        for window in &plan.windows {
            candidates.extend(scheduling::window_candidates(
                &input,
                window.start,
                window.end,
            ));
        }
        candidates.sort_by_key(|v| epoch(&text(v, "start")));
        candidates.dedup();
        candidates.truncate(request["limit"].as_u64().unwrap_or(12) as usize);
        (plan.zone, plan.source)
    } else {
        candidates = scheduling::candidates(request)?;
        (text(request, "zone"), text(request, "source"))
    };
    Ok(json!({"candidates":candidates,"hostTimezone":zone,"hostTimezoneSource":source}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{array, epoch, iso};

    const MINUTE: i64 = 60_000;
    const DAY: i64 = 86_400_000;
    const CANCUN: &str = "America/Cancun";
    const ROME: &str = "Europe/Rome";

    fn ms(value: &str) -> i64 {
        epoch(value).expect("synthetic RFC3339 timestamp")
    }

    fn interval(start: &str, end: &str) -> Interval {
        Interval {
            start: ms(start),
            end: ms(end),
        }
    }

    // Match Providers::events' normalized, flat shape, not Google's raw shape.
    fn event(summary: &str, location: &str, start: &str, end: &str) -> Value {
        json!({
            "calendarId": "synthetic-travel-calendar",
            "calendarName": "Synthetic (TripIt)",
            "summary": summary,
            "location": location,
            "start": start,
            "end": end,
            "inPerson": true,
        })
    }

    fn cancun_journey() -> Vec<Value> {
        vec![
            event(
                "XX100 FCO to ATL",
                "Synthetic terminal (FCO)",
                "2026-10-26T09:25:00Z",
                "2026-10-26T20:24:00Z",
            ),
            event(
                "XX101 ATL to CUN",
                "Synthetic terminal (ATL)",
                "2026-10-26T22:35:00Z",
                "2026-10-27T00:55:00Z",
            ),
            event(
                "XX102 CUN to ATL",
                "Synthetic terminal (CUN)",
                "2026-10-30T17:35:00Z",
                "2026-10-30T20:14:00Z",
            ),
            event(
                "XX103 ATL to FCO",
                "Synthetic terminal (ATL)",
                "2026-10-30T21:55:00Z",
                "2026-10-31T07:15:00Z",
            ),
        ]
    }

    fn flight_busy(events: &[Value]) -> Vec<Value> {
        events
            .iter()
            .map(|e| json!({"start": e["start"], "end": e["end"]}))
            .collect()
    }

    fn request(day: &str, busy: &[Value]) -> Value {
        json!({
            "day": day, "zone": ROME, "source": "google-calendar",
            "hours": {"start": 9, "end": 18},
            "durationMinutes": 30, "bufferMinutes": 10,
            "notBefore": ms("2000-01-01T00:00:00Z"), "limit": 96,
            "busy": busy,
        })
    }

    fn plan(day: &str, events: &[Value]) -> DayPlan {
        infer_day_plan(&request(day, &flight_busy(events)), events).unwrap()
    }

    fn slots(day: &str, events: &[Value]) -> Value {
        slot_path(&request(day, &flight_busy(events)), events, None).unwrap()
    }

    fn assert_zone(result: &Value, zone: &str, source: &str) {
        assert_eq!(result["hostTimezone"], zone, "host zone");
        assert_eq!(result["hostTimezoneSource"], source, "host zone source");
    }

    fn assert_workday(result: &Value, start: &str, end: &str) {
        let candidates = array(&result["candidates"]);
        assert!(!candidates.is_empty(), "expected a nonempty working window");
        assert_eq!(ms(&text(&candidates[0], "start")), ms(start), "first slot");
        for slot in &candidates {
            let a = ms(&text(slot, "start"));
            let b = ms(&text(slot, "end"));
            assert!(
                a >= ms(start) && b <= ms(end),
                "slot outside workday: {slot}"
            );
            assert_eq!(b - a, 30 * MINUTE);
        }
        assert!(
            candidates
                .windows(2)
                .all(|pair| ms(&text(&pair[0], "end")) <= ms(&text(&pair[1], "start")))
        );
    }

    fn assert_safe(result: &Value, busy: &[Value]) {
        for slot in array(&result["candidates"]) {
            assert_eq!(
                scheduling::constraint(&text(&slot, "start"), &text(&slot, "end"), busy, 10.0),
                None,
                "unsafe slot: {slot}"
            );
        }
    }

    // Accept either merged transit blocks or separate adjacent blocks. This
    // checks continuous coverage rather than prescribing the representation.
    fn assert_covered(blocks: &[Interval], start: &str, end: &str) {
        let mut blocks = blocks.to_vec();
        blocks.sort_by_key(|b| b.start);
        let mut cursor = ms(start);
        for block in blocks {
            if block.start <= cursor {
                cursor = cursor.max(block.end);
            }
        }
        assert!(
            cursor >= ms(end),
            "unblocked transit begins at {}",
            iso(cursor)
        );
    }

    fn all_blocks(plan: &DayPlan, busy: &[Value]) -> Vec<Interval> {
        let mut blocks = plan.blocked.clone();
        blocks.extend(
            busy.iter()
                .map(|b| interval(&text(b, "start"), &text(b, "end"))),
        );
        blocks
    }

    #[test]
    fn acceptance_cancun_oct_27_28_29_use_local_working_hours() {
        let events = cancun_journey();
        let busy = flight_busy(&events);
        assert_eq!(busy.len(), 4, "only flights belong in free/busy");
        for day in ["2026-10-27", "2026-10-28", "2026-10-29"] {
            let result = slots(day, &events);
            assert_workday(
                &result,
                &format!("{day}T14:00:00Z"),
                &format!("{day}T23:00:00Z"),
            );
            assert_eq!(
                array(&result["candidates"]).len(),
                18,
                "complete, unlimited workday"
            );
            assert_eq!(
                ms(&text(
                    result["candidates"].as_array().unwrap().last().unwrap(),
                    "end"
                )),
                ms(&format!("{day}T23:00:00Z"))
            );
            assert_zone(&result, CANCUN, "travel");
            let plan = plan(day, &events);
            assert_eq!(
                (plan.zone.as_str(), plan.source.as_str()),
                (CANCUN, "travel")
            );
            assert_eq!(
                plan.windows,
                vec![interval(
                    &format!("{day}T14:00:00Z"),
                    &format!("{day}T23:00:00Z")
                )],
                "working windows use UTC epoch milliseconds"
            );
            assert_safe(&result, &busy);
        }
    }

    #[test]
    fn acceptance_oct_30_stops_before_cancun_departure_and_connection() {
        let events = cancun_journey();
        let result = slots("2026-10-30", &events);
        // 12:35 Cancun departure minus ten minutes = 17:25Z.
        assert_workday(&result, "2026-10-30T14:00:00Z", "2026-10-30T17:25:00Z");
        assert_eq!(array(&result["candidates"]).len(), 6);
        assert_zone(&result, CANCUN, "travel");
        assert_eq!(plan("2026-10-30", &events).zone, CANCUN);
        let mut blocked = flight_busy(&events);
        blocked.push(json!({"start": "2026-10-30T20:14:00Z", "end": "2026-10-30T21:55:00Z"}));
        assert_safe(&result, &blocked);
        assert!(
            array(&result["candidates"])
                .iter()
                .all(|s| ms(&text(s, "end")) <= ms("2026-10-30T17:25:00Z"))
        );
    }

    #[test]
    fn acceptance_oct_31_returns_to_rome_but_saturday_has_no_slots() {
        let events = cancun_journey();
        let plan = plan("2026-10-31", &events);
        assert_eq!(plan.zone, ROME);
        let result = slots("2026-10-31", &events);
        assert!(
            array(&result["candidates"]).is_empty(),
            "Saturday must stay closed"
        );
        assert_zone(&result, ROME, "travel");
        assert!(plan.windows.is_empty());
    }

    #[test]
    fn acceptance_return_shifted_48h_opens_monday_rome_after_landing() {
        let mut events = cancun_journey();
        for e in &mut events[2..] {
            for key in ["start", "end"] {
                e[key] = json!(iso(ms(&text(e, key)) + 2 * DAY));
            }
        }
        // UTC instants +48h: Cancun stays UTC-5 as US DST ends Nov 1.
        assert_eq!(ms(&text(&events[2], "start")), ms("2026-11-01T17:35:00Z"));
        assert_eq!(
            scheduling::zoned("2026-11-01", 12.0 + 35.0 / 60.0, CANCUN).unwrap(),
            ms("2026-11-01T17:35:00Z")
        );
        assert_eq!(ms(&text(&events[3], "end")), ms("2026-11-02T07:15:00Z"));
        let result = slots("2026-11-02", &events);
        assert_workday(&result, "2026-11-02T08:00:00Z", "2026-11-02T17:00:00Z");
        assert_zone(&result, ROME, "travel");
        assert_eq!(plan("2026-11-02", &events).zone, ROME);
        assert_eq!(array(&result["candidates"]).len(), 18);
        assert!(
            array(&result["candidates"])
                .iter()
                .all(|s| ms(&text(s, "start")) >= ms("2026-11-02T07:15:00Z") + 10 * MINUTE)
        );
        assert_safe(&result, &flight_busy(&events));
    }

    #[test]
    fn parses_summary_and_location_with_lowercase_and_odd_spacing() {
        let mut e = event(
            "  xx100   fco  To   atl  ",
            "  Synthetic terminal (fco) ",
            "2026-10-26T09:25:00Z",
            "2026-10-26T20:24:00Z",
        );
        e["calendarName"] = json!("Synthetic tRiPiT import");
        assert_eq!(
            parse_legs(&[e]),
            vec![Leg {
                origin: "FCO".into(),
                destination: "ATL".into(),
                origin_zone: ROME.into(),
                destination_zone: "America/New_York".into(),
                interval: interval("2026-10-26T09:25:00Z", "2026-10-26T20:24:00Z"),
            }]
        );
    }

    #[test]
    fn parses_multileg_events_in_chronological_order() {
        let mut events = cancun_journey();
        events.reverse();
        let legs = parse_legs(&events);
        assert_eq!(legs.len(), 4);
        assert_eq!(
            legs.iter()
                .map(|l| (l.origin.as_str(), l.destination.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("FCO", "ATL"),
                ("ATL", "CUN"),
                ("CUN", "ATL"),
                ("ATL", "FCO")
            ]
        );
        assert!(
            legs.windows(2)
                .all(|pair| pair[0].interval.start < pair[1].interval.start)
        );
    }

    #[test]
    fn non_tripit_calendars_are_ignored_even_when_the_id_or_title_mentions_tripit() {
        let mut events = cancun_journey();
        for e in &mut events {
            e["calendarName"] = json!("Synthetic personal calendar");
            e["calendarId"] = json!("synthetic-tripit-calendar");
        }
        assert!(parse_legs(&events).is_empty());
        assert!(parse_stays(&events).is_empty());
        assert_eq!(plan("2026-10-28", &events).zone, ROME);
    }

    #[test]
    fn unknown_airports_give_no_location_but_freebusy_still_blocks() {
        for summary in ["XX200 ZZZ to CUN", "XX201 FCO to ZZZ"] {
            let events = vec![event(
                summary,
                "Synthetic unknown terminal",
                "2026-10-27T10:00:00Z",
                "2026-10-27T15:00:00Z",
            )];
            assert!(parse_legs(&events).is_empty());
            let result = slots("2026-10-27", &events);
            assert_zone(&result, ROME, "google-calendar");
            assert!(!array(&result["candidates"]).is_empty());
            assert_safe(&result, &flight_busy(&events));
        }
    }

    #[test]
    fn required_airports_resolve_to_iana_zones() {
        let groups: &[(&str, &[&str])] = &[
            (
                ROME,
                &[
                    "FCO", "CIA", "MXP", "LIN", "BGY", "VCE", "NAP", "BLQ", "FLR", "PSA", "TRN",
                    "CTA", "PMO", "BRI", "CAG", "OLB",
                ],
            ),
            (CANCUN, &["CUN"]),
            (
                "America/New_York",
                &["ATL", "JFK", "EWR", "LGA", "BOS", "MIA"],
            ),
            ("America/Los_Angeles", &["SFO", "LAX", "SEA"]),
            ("America/Chicago", &["ORD", "DFW"]),
            ("America/Denver", &["DEN"]),
            ("Europe/London", &["LHR", "LGW"]),
            ("Europe/Paris", &["CDG"]),
            ("Europe/Amsterdam", &["AMS"]),
            ("Europe/Berlin", &["FRA", "MUC"]),
            ("Europe/Zurich", &["ZRH", "GVA"]),
            ("Europe/Madrid", &["MAD", "BCN"]),
            ("Europe/Lisbon", &["LIS"]),
            ("Asia/Dubai", &["DXB"]),
            ("Asia/Qatar", &["DOH"]),
            ("Europe/Istanbul", &["IST"]),
            ("Asia/Singapore", &["SIN"]),
            ("Asia/Hong_Kong", &["HKG"]),
            ("Asia/Tokyo", &["NRT", "HND"]),
            ("Asia/Seoul", &["ICN"]),
            ("Australia/Sydney", &["SYD"]),
            ("Pacific/Auckland", &["AKL"]),
            ("Pacific/Honolulu", &["HNL"]),
            ("America/Mexico_City", &["MEX"]),
            ("America/Sao_Paulo", &["GRU"]),
            ("America/Toronto", &["YYZ"]),
            ("America/Vancouver", &["YVR"]),
        ];
        for (zone, airports) in groups {
            for airport in *airports {
                let origin = if *airport == "FCO" { "JFK" } else { "FCO" };
                let events = vec![event(
                    &format!("XX200 {origin} to {airport}"),
                    &format!("Synthetic terminal ({origin})"),
                    "2026-10-20T06:00:00Z",
                    "2026-10-20T18:00:00Z",
                )];
                let legs = parse_legs(&events);
                assert_eq!(legs.len(), 1, "missing airport {airport}");
                assert_eq!(legs[0].destination_zone, *zone, "airport {airport}");
            }
        }
    }

    fn overnight_connection() -> Vec<Value> {
        vec![
            event(
                "XX200 FCO to ATL",
                "Synthetic terminal (FCO)",
                "2026-10-25T15:00:00Z",
                "2026-10-26T01:00:00Z",
            ),
            event(
                "XX201 ATL to CUN",
                "Synthetic terminal (ATL)",
                "2026-10-27T00:00:00Z",
                "2026-10-27T03:00:00Z",
            ),
        ]
    }

    #[test]
    fn connection_under_24h_never_opens_a_whole_day_in_atlanta() {
        let events = overnight_connection();
        let plan = plan("2026-10-26", &events);
        assert_ne!(plan.zone, "America/New_York");
        assert!(
            plan.windows.is_empty(),
            "fully in transit, despite a 23h connection"
        );
        assert!(array(&slots("2026-10-26", &events)["candidates"]).is_empty());
    }

    #[test]
    fn connection_gap_is_added_to_blocks_without_freebusy_help() {
        let events = overnight_connection();
        let plan = plan("2026-10-26", &events);
        assert_covered(
            &plan.blocked,
            "2026-10-26T01:00:00Z",
            "2026-10-27T00:00:00Z",
        );
        assert_covered(
            &all_blocks(&plan, &flight_busy(&events)),
            "2026-10-25T15:00:00Z",
            "2026-10-27T03:00:00Z",
        );
    }

    fn with_connection_hotel() -> Vec<Value> {
        let mut events = overnight_connection();
        events.extend([
            event(
                "Check-in: Synthetic Lodge",
                "Synthetic address, Atlanta (ATL)",
                "2026-10-26T02:00:00Z",
                "2026-10-26T02:30:00Z",
            ),
            event(
                "Check-out: Synthetic Lodge",
                "Synthetic address, Atlanta (ATL)",
                "2026-10-26T23:00:00Z",
                "2026-10-26T23:30:00Z",
            ),
        ]);
        events
    }

    #[test]
    fn hotel_at_connection_city_makes_it_the_location_for_those_nights() {
        let events = with_connection_hotel();
        // Hotel markers supply location, not an all-night free/busy interval.
        let busy = flight_busy(&events[..2]);
        let input = request("2026-10-26", &busy);
        let plan = infer_day_plan(&input, &events).unwrap();
        assert_eq!(
            (plan.zone.as_str(), plan.source.as_str()),
            ("America/New_York", "travel")
        );
        let result = slot_path(&input, &events, None).unwrap();
        assert_zone(&result, "America/New_York", "travel");
        assert_workday(&result, "2026-10-26T13:00:00Z", "2026-10-26T22:00:00Z");
        assert_eq!(array(&result["candidates"]).len(), 18);
        assert_safe(&result, &busy);
    }

    #[test]
    fn hotel_location_matches_airport_code_or_city_name() {
        for location in ["Synthetic address (ATL)", "Synthetic address, Atlanta"] {
            let mut events = with_connection_hotel();
            for e in &mut events[2..] {
                e["location"] = json!(location);
            }
            assert_eq!(
                parse_stays(&events),
                vec![Stay {
                    zone: "America/New_York".into(),
                    interval: interval("2026-10-26T02:00:00Z", "2026-10-26T23:00:00Z")
                }]
            );
        }
    }

    #[test]
    fn unrecognized_hotel_address_uses_last_flight_arrival() {
        let mut events = with_connection_hotel();
        for e in &mut events[2..] {
            e["location"] = json!("Synthetic unrecognized address");
        }
        assert_eq!(
            parse_stays(&events),
            vec![Stay {
                zone: "America/New_York".into(),
                interval: interval("2026-10-26T02:00:00Z", "2026-10-26T23:00:00Z")
            }]
        );
    }

    #[test]
    fn non_tripit_hotel_does_not_turn_a_connection_into_a_location() {
        let mut events = with_connection_hotel();
        for e in &mut events[2..] {
            e["calendarName"] = json!("Synthetic personal calendar");
        }
        assert!(parse_stays(&events).is_empty());
        assert_ne!(plan("2026-10-26", &events).zone, "America/New_York");
        assert_covered(
            &plan("2026-10-26", &events).blocked,
            "2026-10-26T01:00:00Z",
            "2026-10-27T00:00:00Z",
        );
    }

    #[test]
    fn overnight_eastbound_flight_has_no_slots_on_fully_in_transit_date() {
        let events = vec![event(
            "XX300 JFK to SIN",
            "Synthetic terminal (JFK)",
            "2026-10-26T23:00:00Z",
            "2026-10-28T00:00:00Z",
        )];
        let result = slots("2026-10-27", &events);
        assert!(
            array(&result["candidates"]).is_empty(),
            "all working hours fall inside the overnight flight"
        );
        assert!(plan("2026-10-27", &events).windows.is_empty());
        assert_safe(&result, &flight_busy(&events));
    }

    #[test]
    fn westbound_dateline_flight_arrives_on_previous_local_date() {
        let events = vec![event(
            "XX301 HND to HNL",
            "Synthetic terminal (HND)",
            "2026-10-27T00:00:00Z",
            "2026-10-27T07:00:00Z",
        )];
        assert_eq!(
            scheduling::date(ms("2026-10-27T00:00:00Z"), "Asia/Tokyo").unwrap(),
            "2026-10-27"
        );
        assert_eq!(
            scheduling::date(ms("2026-10-27T07:00:00Z"), "Pacific/Honolulu").unwrap(),
            "2026-10-26"
        );
        let result = slots("2026-10-27", &events);
        assert_workday(&result, "2026-10-27T19:00:00Z", "2026-10-28T04:00:00Z");
        assert_zone(&result, "Pacific/Honolulu", "travel");
        assert_eq!(plan("2026-10-27", &events).zone, "Pacific/Honolulu");
        assert_safe(&result, &flight_busy(&events));
    }

    fn three_leg_journey() -> Vec<Value> {
        vec![
            event(
                "XX400 FCO to ATL",
                "Synthetic terminal (FCO)",
                "2026-10-26T09:25:00Z",
                "2026-10-26T20:24:00Z",
            ),
            event(
                "XX401 ATL to DFW",
                "Synthetic terminal (ATL)",
                "2026-10-26T22:35:00Z",
                "2026-10-27T01:00:00Z",
            ),
            event(
                "XX402 DFW to CUN",
                "Synthetic terminal (DFW)",
                "2026-10-27T13:00:00Z",
                "2026-10-27T16:00:00Z",
            ),
        ]
    }

    #[test]
    fn multileg_journey_blocks_both_layovers_and_uses_final_destination() {
        let events = three_leg_journey();
        let plan = plan("2026-10-27", &events);
        assert_covered(
            &plan.blocked,
            "2026-10-26T20:24:00Z",
            "2026-10-26T22:35:00Z",
        );
        assert_covered(
            &plan.blocked,
            "2026-10-27T01:00:00Z",
            "2026-10-27T13:00:00Z",
        );
        assert_covered(
            &all_blocks(&plan, &flight_busy(&events)),
            "2026-10-26T09:25:00Z",
            "2026-10-27T16:00:00Z",
        );
        assert_eq!(plan.zone, CANCUN);
    }

    #[test]
    fn arrival_during_workday_opens_only_after_final_landing_and_buffer() {
        let events = three_leg_journey();
        let result = slots("2026-10-27", &events);
        assert_workday(&result, "2026-10-27T16:10:00Z", "2026-10-27T23:00:00Z");
        assert_zone(&result, CANCUN, "travel");
        assert!(
            array(&result["candidates"])
                .iter()
                .all(|s| ms(&text(s, "start")) >= ms("2026-10-27T16:00:00Z") + 10 * MINUTE)
        );
        assert_safe(&result, &flight_busy(&events));
    }

    #[test]
    fn conflicting_hotel_falls_back_for_date_and_keeps_flight_blocks() {
        let mut events = cancun_journey();
        events.extend([
            event(
                "Check-in: Synthetic Lodge",
                "Synthetic address, Tokyo (HND)",
                "2026-10-29T00:00:00Z",
                "2026-10-29T00:30:00Z",
            ),
            event(
                "Check-out: Synthetic Lodge",
                "Synthetic address, Tokyo (HND)",
                "2026-10-31T12:00:00Z",
                "2026-10-31T12:30:00Z",
            ),
        ]);
        let busy = flight_busy(&events[..4]);
        let input = request("2026-10-30", &busy);
        let plan = infer_day_plan(&input, &events).unwrap();
        assert_eq!(
            (plan.zone.as_str(), plan.source.as_str()),
            (ROME, "google-calendar")
        );
        for b in &busy {
            assert_covered(
                &all_blocks(&plan, &busy),
                &text(b, "start"),
                &text(b, "end"),
            );
        }
        assert_covered(
            &plan.blocked,
            "2026-10-30T20:14:00Z",
            "2026-10-30T21:55:00Z",
        );
        let result = slot_path(&input, &events, None).unwrap();
        assert_zone(&result, ROME, "google-calendar");
        assert!(!array(&result["candidates"]).is_empty());
        let mut blocked = busy;
        blocked.push(json!({"start": "2026-10-30T17:35:00Z", "end": "2026-10-31T07:15:00Z"}));
        assert_safe(&result, &blocked);
    }

    #[test]
    fn overlapping_flights_use_fallback_but_keep_both_busy_intervals() {
        let events = vec![
            event(
                "XX500 FCO to CUN",
                "Synthetic terminal (FCO)",
                "2026-10-27T09:00:00Z",
                "2026-10-27T14:00:00Z",
            ),
            event(
                "XX501 FCO to JFK",
                "Synthetic terminal (FCO)",
                "2026-10-27T12:00:00Z",
                "2026-10-27T16:00:00Z",
            ),
        ];
        let plan = plan("2026-10-27", &events);
        assert_eq!(
            (plan.zone.as_str(), plan.source.as_str()),
            (ROME, "google-calendar")
        );
        let result = slots("2026-10-27", &events);
        assert_zone(&result, ROME, "google-calendar");
        assert!(!array(&result["candidates"]).is_empty());
        assert_safe(&result, &flight_busy(&events));
    }

    #[test]
    fn disconnected_leg_origins_use_fallback_without_losing_conflicts() {
        let events = vec![
            event(
                "XX510 FCO to CUN",
                "Synthetic terminal (FCO)",
                "2026-10-26T09:00:00Z",
                "2026-10-26T20:00:00Z",
            ),
            event(
                "XX511 JFK to ATL",
                "Synthetic terminal (JFK)",
                "2026-10-27T13:00:00Z",
                "2026-10-27T16:00:00Z",
            ),
        ];
        let plan = plan("2026-10-27", &events);
        assert_eq!(
            (plan.zone.as_str(), plan.source.as_str()),
            (ROME, "google-calendar")
        );
        let result = slots("2026-10-27", &events);
        assert_zone(&result, ROME, "google-calendar");
        assert!(!array(&result["candidates"]).is_empty());
        assert_safe(&result, &flight_busy(&events));
    }

    #[test]
    fn rome_dst_oct_25_uses_date_specific_offsets() {
        let events = vec![event(
            "XX600 JFK to FCO",
            "Synthetic terminal (JFK)",
            "2026-10-23T20:00:00Z",
            "2026-10-24T06:00:00Z",
        )];
        let mut input = request("2026-10-25", &flight_busy(&events));
        input["zone"] = json!(CANCUN);
        let plan = infer_day_plan(&input, &events).unwrap();
        assert_eq!((plan.zone.as_str(), plan.source.as_str()), (ROME, "travel"));
        assert_eq!(
            scheduling::zoned("2026-10-25", 0.0, &plan.zone).unwrap(),
            ms("2026-10-24T22:00:00Z")
        );
        assert_eq!(
            scheduling::zoned("2026-10-25", 24.0, &plan.zone).unwrap(),
            ms("2026-10-25T23:00:00Z")
        );
        assert_eq!(
            scheduling::zoned("2026-10-25", 9.0, &plan.zone).unwrap(),
            ms("2026-10-25T08:00:00Z")
        );
        assert!(
            array(&slot_path(&input, &events, None).unwrap()["candidates"]).is_empty(),
            "DST Sunday stays closed"
        );
    }

    #[test]
    fn new_york_dst_nov_1_uses_date_specific_offsets() {
        let events = vec![event(
            "XX601 FCO to JFK",
            "Synthetic terminal (FCO)",
            "2026-10-30T09:00:00Z",
            "2026-10-30T18:00:00Z",
        )];
        let input = request("2026-11-01", &flight_busy(&events));
        let plan = infer_day_plan(&input, &events).unwrap();
        assert_eq!(
            (plan.zone.as_str(), plan.source.as_str()),
            ("America/New_York", "travel")
        );
        assert_eq!(
            scheduling::zoned("2026-11-01", 0.0, &plan.zone).unwrap(),
            ms("2026-11-01T04:00:00Z")
        );
        assert_eq!(
            scheduling::zoned("2026-11-01", 24.0, &plan.zone).unwrap(),
            ms("2026-11-02T05:00:00Z")
        );
        assert_eq!(
            scheduling::zoned("2026-11-01", 9.0, &plan.zone).unwrap(),
            ms("2026-11-01T14:00:00Z")
        );
        assert!(
            array(&slot_path(&input, &events, None).unwrap()["candidates"]).is_empty(),
            "DST Sunday stays closed"
        );
    }

    #[test]
    fn first_flight_origin_applies_only_on_its_departure_date() {
        let events = vec![event(
            "XX700 FCO to CUN",
            "Synthetic terminal (FCO)",
            "2026-10-26T09:25:00Z",
            "2026-10-26T20:00:00Z",
        )];
        let mut input = request("2026-10-26", &flight_busy(&events));
        input["zone"] = json!("America/New_York");
        let plan = infer_day_plan(&input, &events).unwrap();
        assert_eq!((plan.zone.as_str(), plan.source.as_str()), (ROME, "travel"));
        let result = slot_path(&input, &events, None).unwrap();
        // Rome hours until the departure buffer, then Cancun hours after landing
        // (15:00 local) plus the buffer: 08:00-09:15Z and 20:10-23:00Z.
        let starts: Vec<i64> = array(&result["candidates"])
            .iter()
            .map(|s| ms(&text(s, "start")))
            .collect();
        assert_eq!(starts.first(), Some(&ms("2026-10-26T08:00:00Z")));
        assert!(starts.contains(&ms("2026-10-26T20:10:00Z")));
        for slot in array(&result["candidates"]) {
            let (a, b) = (ms(&text(&slot, "start")), ms(&text(&slot, "end")));
            let before = a >= ms("2026-10-26T08:00:00Z") && b <= ms("2026-10-26T09:15:00Z");
            let after = a >= ms("2026-10-26T20:10:00Z") && b <= ms("2026-10-26T23:00:00Z");
            assert!(
                before || after,
                "slot outside origin/destination windows: {slot}"
            );
        }
        assert_safe(&result, &flight_busy(&events));
        assert_zone(&result, ROME, "travel");
        input["day"] = json!("2026-10-23");
        let earlier = infer_day_plan(&input, &events).unwrap();
        assert_eq!(
            (earlier.zone.as_str(), earlier.source.as_str()),
            ("America/New_York", "google-calendar")
        );
    }

    fn one_way_arrival() -> Vec<Value> {
        // Arrival Sep 30 at 19:00 Cancun. Thirty days expires Oct 30 at 19:00.
        vec![event(
            "XX701 FCO to CUN",
            "Synthetic terminal (FCO)",
            "2026-09-30T12:00:00Z",
            "2026-10-01T00:00:00Z",
        )]
    }

    #[test]
    fn destination_persists_after_last_arrival_through_day_30_working_hours() {
        let events = one_way_arrival();
        for day in ["2026-10-02", "2026-10-15", "2026-10-30"] {
            let plan = plan(day, &events);
            assert_eq!(
                (plan.zone.as_str(), plan.source.as_str()),
                (CANCUN, "travel")
            );
            assert_workday(
                &slots(day, &events),
                &format!("{day}T14:00:00Z"),
                &format!("{day}T23:00:00Z"),
            );
        }
    }

    #[test]
    fn destination_does_not_persist_beyond_30_days() {
        let events = one_way_arrival();
        for day in ["2026-10-31", "2026-11-02"] {
            let plan = plan(day, &events);
            assert_eq!(
                (plan.zone.as_str(), plan.source.as_str()),
                (ROME, "google-calendar")
            );
        }
        let input = request("2026-11-02", &flight_busy(&events));
        assert_fallback(&input, &events, None);
    }

    fn assert_fallback(input: &Value, events: &[Value], flag: Option<&str>) {
        // Compare the entire pure slot response, including the unchanged source,
        // and its serialized bytes. DST Sundays deliberately exercise candidates
        // directly; App retains its separate weekday gate on the legacy path.
        let expected = json!({
            "candidates": scheduling::candidates(input).unwrap(),
            "hostTimezone": input["zone"], "hostTimezoneSource": input["source"],
        });
        let actual = slot_path(input, events, flag).unwrap();
        assert_eq!(actual, expected, "fallback response changed");
        assert_eq!(
            actual.to_string(),
            expected.to_string(),
            "serialized fallback changed"
        );
    }

    fn fallback_requests() -> Vec<Value> {
        let mut requests = vec![];
        for day in [
            "2026-10-23",
            "2026-10-25",
            "2026-10-26",
            "2026-10-28",
            "2026-10-30",
            "2026-11-01",
            "2026-11-02",
        ] {
            for zone in [ROME, "America/New_York", CANCUN] {
                for source in ["google-calendar", "owner-device", "settings"] {
                    let busy = vec![json!({
                        "start": iso(scheduling::zoned(day, 11.0, zone).unwrap()),
                        "end": iso(scheduling::zoned(day, 12.25, zone).unwrap()),
                    })];
                    let mut input = request(day, &busy);
                    input["zone"] = json!(zone);
                    input["source"] = json!(source);
                    input["limit"] = json!(12);
                    input["notBefore"] = json!(scheduling::zoned(day, 9.5, zone).unwrap());
                    requests.push(input);
                }
            }
        }
        requests
    }

    #[test]
    fn no_travel_matches_existing_candidates_full_json_including_dst() {
        for input in fallback_requests() {
            assert_fallback(&input, &[], None);
        }
    }

    #[test]
    fn non_tripit_flight_titles_match_existing_candidates_full_json() {
        for input in fallback_requests() {
            let day = text(&input, "day");
            let mut e = event(
                "AB12 FCO to JFK",
                "Synthetic terminal (FCO)",
                &format!("{day}T10:00:00Z"),
                &format!("{day}T20:00:00Z"),
            );
            e["calendarName"] = json!("Synthetic personal calendar");
            let busy = flight_busy(&[e.clone()]);
            let mut input = input;
            input["busy"] = json!(busy);
            assert_fallback(&input, &[e], None);
        }
    }

    #[test]
    fn kill_switch_off_matches_fallback_even_with_travel_data() {
        let events = cancun_journey();
        for input in fallback_requests() {
            let mut input = input;
            input["busy"] = json!(flight_busy(&events));
            assert_fallback(&input, &events, Some("off"));
        }
    }

    #[test]
    fn kill_switch_zero_and_false_match_fallback_even_with_travel_data() {
        let events = cancun_journey();
        for flag in ["0", "false"] {
            for input in fallback_requests() {
                let mut input = input;
                input["busy"] = json!(flight_busy(&events));
                assert_fallback(&input, &events, Some(flag));
            }
        }
    }
}

#[cfg(test)]
mod regression_tests {
    use super::*;

    fn flight(summary: &str, start: &str, end: &str) -> Value {
        json!({"calendarName":"Synthetic (TripIt)","summary":summary,"start":start,"end":end})
    }
    fn request(day: &str) -> Value {
        json!({"day":day,"zone":"Europe/Rome","source":"settings","hours":{"start":9,"end":18},"durationMinutes":30,"bufferMinutes":10,"limit":96})
    }
    #[test]
    fn embedded_airports_are_unique_and_have_valid_zones() {
        let mut codes = std::collections::HashSet::new();
        for (airports, _, zone) in AIRPORTS {
            assert!(scheduling::valid_zone(zone), "invalid embedded zone {zone}");
            for code in airports.split_whitespace() {
                assert_eq!(code.len(), 3);
                assert!(codes.insert(code), "duplicate airport {code}");
                assert_eq!(airport(code), Some(*zone));
            }
        }
        assert!(codes.len() >= 150);
    }
    #[test]
    fn unknown_destination_breaks_a_previously_inferred_location() {
        let events = [
            flight(
                "XX100 FCO to CUN",
                "2026-10-20T09:00:00Z",
                "2026-10-20T20:00:00Z",
            ),
            flight(
                "XX101 CUN to ZZZ",
                "2026-10-26T10:00:00Z",
                "2026-10-26T20:00:00Z",
            ),
        ];
        let plan = infer_day_plan(&request("2026-10-27"), &events).unwrap();
        assert_eq!(
            (plan.zone.as_str(), plan.source.as_str()),
            ("Europe/Rome", "settings")
        );
        assert!(plan.blocked.contains(&Interval {
            start: epoch("2026-10-26T10:00:00Z").unwrap(),
            end: epoch("2026-10-26T20:00:00Z").unwrap()
        }));
    }
    #[test]
    fn connection_checkin_does_not_require_a_checkout_marker() {
        let events = [
            flight(
                "XX100 FCO to ATL",
                "2026-10-25T15:00:00Z",
                "2026-10-26T01:00:00Z",
            ),
            flight(
                "XX101 ATL to CUN",
                "2026-10-27T00:00:00Z",
                "2026-10-27T03:00:00Z",
            ),
            json!({"calendarName":"Synthetic (TripIt)","summary":"Check-in: Synthetic Lodge","location":"Atlanta","start":"2026-10-26T02:00:00Z","end":"2026-10-26T02:30:00Z"}),
        ];
        let plan = infer_day_plan(&request("2026-10-26"), &events).unwrap();
        assert_eq!(plan.zone, "America/New_York");
        let result = slot_path(&request("2026-10-26"), &events, None).unwrap();
        assert_eq!(result["candidates"][0]["start"], "2026-10-26T13:00:00.000Z");
    }
    #[test]
    fn hotel_without_flights_uses_explicit_city_and_closes_at_checkout() {
        let events = [
            json!({"calendarName":"Synthetic (TripIt)","summary":"Check-in: Synthetic Lodge","location":"Atlanta","start":"2026-10-25T20:00:00Z","end":"2026-10-25T21:00:00Z"}),
            json!({"calendarName":"Synthetic (TripIt)","summary":"Check-out: Synthetic Lodge","location":"Atlanta","start":"2026-10-28T20:00:00Z","end":"2026-10-28T21:00:00Z"}),
        ];
        let day = infer_day_plan(&request("2026-10-26"), &events).unwrap();
        assert_eq!(day.zone, "America/New_York");
        assert_eq!(
            day.windows,
            vec![Interval {
                start: epoch("2026-10-26T13:00:00Z").unwrap(),
                end: epoch("2026-10-26T22:00:00Z").unwrap()
            }]
        );
        assert!(
            !infer_day_plan(&request("2026-10-29"), &events)
                .unwrap()
                .affected
        );
    }
}
