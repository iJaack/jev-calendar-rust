use crate::{array, epoch, iso, now, number, scheduling as s, text};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::LazyLock,
};
type Intervals = Vec<(i64, i64)>;
type ProjectIntervals = (Intervals, HashMap<String, Intervals>);
pub fn defaults() -> Vec<Value> {
    serde_json::from_str(include_str!("../fixtures/default-project-rules.json"))
        .expect("validated rules")
}
pub fn normalize(v: &Value) -> Result<Vec<Value>> {
    ensure!(
        v.is_array() && array(v).len() <= 40,
        "Use at most 40 projects"
    );
    let mut seen = HashSet::new();
    array(v).iter().map(|v|{let name=text(v,"name").trim().chars().take(80).collect::<String>();ensure!(!name.is_empty()&&seen.insert(name.to_lowercase()),"Each project needs a unique name");ensure!(["meetings","focus","deadline"].contains(&text(v,"type").as_str()),"Choose a workload type for every project");let mut lists=json!({});for (key,max)in[("keywords",80),("calendarIds",300)]{let mut used=HashSet::new();let list:Vec<_>=array(&v[key]).iter().map(|x|x.as_str().unwrap_or("").trim().to_owned()).filter(|x|!x.is_empty()&&used.insert(x.clone())).take(20).collect();ensure!(list.iter().all(|x|x.chars().count()<=max),"Project match is too long");lists[key]=json!(list);}Ok(json!({"name":name,"type":v["type"],"keywords":lists["keywords"],"calendarIds":lists["calendarIds"]}))}).collect()
}
fn project<'a>(event: &Value, rules: &'a [Value]) -> Option<&'a Value> {
    let title = text(event, "summary").to_lowercase();
    let mut best = None;
    let mut length = 0;
    for r in rules {
        for word in array(&r["keywords"]) {
            let w = word.as_str().unwrap_or("");
            if title.contains(&w.to_lowercase()) && w.len() > length {
                best = Some(r);
                length = w.len()
            }
        }
    }
    best.or_else(|| {
        let name = text(event, "calendarName").to_lowercase();
        rules.iter().find(|r| {
            array(&r["calendarIds"]).contains(&event["calendarId"])
                || array(&r["keywords"])
                    .iter()
                    .any(|w| name.contains(&w.as_str().unwrap_or("").to_lowercase()))
        })
    })
}
fn kind(event: &Value, fallback: &str) -> String {
    static MEETING: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\b(meeting|call|sync|1:1)\b").expect("static regex"));
    let t = text(event, "summary").to_lowercase();
    if t.contains("[deadline]")
        || t.starts_with("deadline:")
        || t.contains("submission due")
        || t.contains("launch day")
    {
        return "deadline".into();
    }
    if t.contains("[focus]") || t.contains("deep work") || t.contains("focus block") {
        return "focus".into();
    }
    if t.contains("[meeting]") || MEETING.is_match(&t) {
        return "meetings".into();
    }
    fallback.into()
}
fn bounds(v: &Value) -> Result<(i64, i64)> {
    let day = text(v, "day");
    let z = text(v, "zone");
    Ok((
        s::zoned(&day, number(&v["hours"], "start"), &z)?,
        s::zoned(&day, number(&v["hours"], "end"), &z)?,
    ))
}
fn clip(items: &[Value], (a, b): (i64, i64)) -> Vec<(i64, i64)> {
    let mut r: Vec<_> = items
        .iter()
        .filter_map(|v| {
            let x = epoch(&text(v, "start"))?.max(a);
            let y = epoch(&text(v, "end"))?.min(b);
            (y > x).then_some((x, y))
        })
        .collect();
    r.sort_by_key(|x| x.0);
    r
}
fn minutes(mut intervals: Vec<(i64, i64)>) -> f64 {
    intervals.sort_by_key(|x| x.0);
    let (mut sum, mut end) = (0i64, i64::MIN);
    for (a, b) in intervals {
        sum += (b - a.max(end)).max(0);
        end = end.max(b);
    }
    ((sum as f64) / 60_000.0).round()
}
pub fn day(v: &Value) -> Result<Value> {
    let bounds = bounds(v)?;
    let busy = clip(&array(&v["busy"]), bounds);
    let rules = array(&v["rules"]);
    let mut projects: HashMap<String, ProjectIntervals> = HashMap::new();
    for event in array(&v["events"]) {
        let intervals = clip(std::slice::from_ref(&event), bounds);
        if intervals.is_empty() {
            continue;
        }
        if let Some(r) = project(&event, &rules) {
            let entry = projects.entry(text(r, "name")).or_default();
            entry.0.extend(&intervals);
            entry
                .1
                .entry(kind(&event, &text(r, "type")))
                .or_default()
                .extend(intervals);
        }
    }
    let mut result = json!({});
    for (name, (all, types)) in projects {
        result[&name] = json!({"minutes":minutes(all),"typeMinutes":{"meetings":minutes(types.get("meetings").cloned().unwrap_or_default()),"focus":minutes(types.get("focus").cloned().unwrap_or_default()),"deadline":minutes(types.get("deadline").cloned().unwrap_or_default())}})
    }
    Ok(
        json!({"day":v["day"],"busyMinutes":minutes(busy.clone()),"meetings":busy.len(),"projects":result}),
    )
}
pub fn types(load: &Value) -> Value {
    let mut totals = json!({"meetings":0.0,"focus":0.0,"deadline":0.0});
    if let Some(projects) = load["projects"].as_object() {
        for p in projects.values() {
            for k in ["meetings", "focus", "deadline"] {
                totals[k] = json!(number(&totals, k) + number(&p["typeMinutes"], k));
            }
        }
    }
    totals
}
fn week(v: &Value) -> Result<Vec<Value>> {
    let monday = s::monday(&text(v, "day"))?;
    (0..5)
        .map(|n| {
            let mut input = v.clone();
            input["day"] = json!(s::shift(&monday, n)?);
            day(&input)
        })
        .collect()
}
fn meeting(load: &Value) -> bool {
    let t = types(load);
    number(&t, "meetings") >= 60.0
        || (number(load, "meetings") >= 2.0
            && number(load, "busyMinutes") >= 120.0
            && number(&t, "focus") < 60.0)
}
pub fn deadline(v: &Value) -> Result<Option<i64>> {
    let current = text(v, "day");
    let rules = array(&v["rules"]);
    let mut stage = None;
    for event in array(&v["events"]) {
        let r = project(&event, &rules);
        if kind(&event, &r.map(|r| text(r, "type")).unwrap_or_default()) != "deadline" {
            continue;
        }
        let start = epoch(&text(&event, "start"));
        let end = epoch(&text(&event, "end"));
        if let (Some(start), Some(end)) = (start, end) {
            let title = text(&event, "summary").to_lowercase();
            if end - start < 90 * 60_000
                && !title.contains("[deadline]")
                && !title.starts_with("deadline:")
            {
                continue;
            }
            let future = s::date(start, &text(v, "zone"))?;
            if future < current {
                continue;
            }
            let mut count = 0;
            let mut d = s::shift(&current, 1)?;
            while d <= future {
                if s::weekday(&d)? % 6 != 0 {
                    count += 1
                }
                d = s::shift(&d, 1)?;
            }
            if count <= 2 {
                stage = Some(stage.map_or(count, |n: i64| n.min(count)))
            }
        }
    }
    Ok(stage)
}
pub fn preference(v: &Value) -> Result<f64> {
    let loads = week(v)?;
    let Some(load) = loads.iter().find(|x| x["day"] == v["day"]) else {
        return Ok(0.0);
    };
    let t = types(load);
    let strength = (number(&t, "meetings") / 60.0).floor()
        + if number(load, "meetings") >= 2.0 && number(&t, "focus") < 60.0 {
            1.0
        } else {
            0.0
        };
    Ok(strength.min(3.0)
        - if number(&t, "focus") >= 90.0 {
            2.0
        } else {
            0.0
        }
        - if loads.iter().any(meeting) && strength == 0.0 {
            1.0
        } else {
            0.0
        }
        - if deadline(v)?.is_some() { 3.0 } else { 0.0 })
}
fn edges(mut c: Vec<Value>, count: usize) -> Vec<Value> {
    if c.len() <= count {
        return c;
    }
    c.sort_by_key(|x| epoch(&text(x, "start")));
    if count == 1 {
        vec![c.remove(0)]
    } else {
        vec![c[0].clone(), c.last().expect("nonempty").clone()]
    }
}

pub fn default_load_mode() -> Value {
    json!({
        "mode": "ultraHeavy",
        "until": Value::Null,
        "tiers": {
            "heavy": {"maxPerDay": 2, "maxPerWeek": 6},
            "ultraHeavy": {"maxPerDay": 1, "maxPerWeek": 3}
        }
    })
}

pub fn default_off_calendar_load() -> Value {
    json!({"weeklyMinutes": 0, "byDate": {}, "byWeek": {}})
}

fn is_date_key(s: &str) -> bool {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

fn whole_cap(v: &Value, key: &str, default: i64, max: i64) -> Result<i64> {
    let raw = v.get(key).unwrap_or(&Value::Null);
    let n = if raw.is_null() {
        default
    } else {
        raw.as_i64()
            .or_else(|| {
                raw.as_f64()
                    .and_then(|f| (f.fract() == 0.0).then_some(f as i64))
            })
            .ok_or_else(|| anyhow::anyhow!("Load caps must be whole numbers"))?
    };
    ensure!(
        (0..=max).contains(&n),
        "Load caps must be between 0 and {max}"
    );
    Ok(n)
}

pub fn normalize_load_mode(v: &Value) -> Result<Value> {
    let defaults = default_load_mode();
    if v.is_null() {
        return Ok(defaults);
    }
    let mode = if let Some(s) = v.as_str() {
        s.to_owned()
    } else {
        let m = text(v, "mode");
        if m.is_empty() {
            text(&defaults, "mode")
        } else {
            m
        }
    };
    ensure!(
        matches!(mode.as_str(), "normal" | "heavy" | "ultraHeavy"),
        "Load mode must be normal, heavy, or ultraHeavy"
    );
    let until = if v.is_string() {
        Value::Null
    } else if v["until"].is_null() || text(v, "until").is_empty() {
        Value::Null
    } else {
        let u = text(v, "until");
        ensure!(is_date_key(&u), "Load mode until date must be YYYY-MM-DD");
        json!(u)
    };
    let src_tiers = if v.is_object() {
        &v["tiers"]
    } else {
        &Value::Null
    };
    let mut tiers = json!({});
    for (name, def_day, def_week) in [("heavy", 2i64, 6i64), ("ultraHeavy", 1, 3)] {
        let src = if src_tiers[name].is_object() {
            &src_tiers[name]
        } else {
            &defaults["tiers"][name]
        };
        tiers[name] = json!({
            "maxPerDay": whole_cap(src, "maxPerDay", def_day, 24)?,
            "maxPerWeek": whole_cap(src, "maxPerWeek", def_week, 120)?
        });
    }
    Ok(json!({"mode": mode, "until": until, "tiers": tiers}))
}

pub fn normalize_off_calendar_load(v: &Value) -> Result<Value> {
    if v.is_null() {
        return Ok(default_off_calendar_load());
    }
    if let Some(n) = v.as_f64() {
        ensure!(
            (0.0..=10_000.0).contains(&n),
            "Off-calendar load must be between 0 and 10000 minutes"
        );
        return Ok(json!({"weeklyMinutes": n, "byDate": {}, "byWeek": {}}));
    }
    let weekly = v["weeklyMinutes"].as_f64().unwrap_or(0.0);
    ensure!(
        (0.0..=10_000.0).contains(&weekly),
        "Off-calendar load must be between 0 and 10000 minutes"
    );
    Ok(json!({
        "weeklyMinutes": weekly,
        "byDate": minute_map(&v["byDate"], false)?,
        "byWeek": minute_map(&v["byWeek"], true)?
    }))
}

fn minute_map(v: &Value, as_week: bool) -> Result<Value> {
    if v.is_null() {
        return Ok(json!({}));
    }
    let obj = v
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Off-calendar entries must be a date map"))?;
    ensure!(obj.len() <= 400, "Use at most 400 off-calendar entries");
    let mut out = serde_json::Map::new();
    for (k, val) in obj {
        ensure!(is_date_key(k), "Off-calendar keys must be YYYY-MM-DD");
        let key = if as_week { s::monday(k)? } else { k.clone() };
        let n = val
            .as_f64()
            .ok_or_else(|| anyhow::anyhow!("Off-calendar minutes must be a number"))?;
        ensure!(
            (0.0..=10_000.0).contains(&n),
            "Off-calendar load must be between 0 and 10000 minutes"
        );
        let prev = out.get(&key).and_then(|x| x.as_f64()).unwrap_or(0.0);
        out.insert(key, json!(prev + n));
    }
    Ok(Value::Object(out))
}

pub fn off_calendar_minutes(load: &Value, day: &str) -> Result<f64> {
    let load = normalize_off_calendar_load(load)?;
    let weekday = s::weekday(day)? % 6 != 0;
    let monday = s::monday(day)?;
    let weekly = load["weeklyMinutes"].as_f64().unwrap_or(0.0);
    let week = load["byWeek"][&monday].as_f64().unwrap_or(0.0);
    let date = load["byDate"][day].as_f64().unwrap_or(0.0);
    let spread = if weekday { (weekly + week) / 5.0 } else { 0.0 };
    Ok(spread + date)
}

fn active_caps(load_mode: &Value, day: &str) -> Result<(String, Option<(i64, i64)>)> {
    let mode = normalize_load_mode(load_mode)?;
    let name = text(&mode, "mode");
    let until = text(&mode, "until");
    if name == "normal" || (!until.is_empty() && day > until.as_str()) {
        return Ok(("normal".into(), None));
    }
    let caps = &mode["tiers"][&name];
    Ok((
        name,
        Some((
            caps["maxPerDay"].as_i64().unwrap_or(0),
            caps["maxPerWeek"].as_i64().unwrap_or(0),
        )),
    ))
}

fn event_day(event: &Value, zone: &str) -> Option<String> {
    let start = text(event, "start");
    if let Some(ms) = epoch(&start) {
        return s::date(ms, zone).ok();
    }
    let day = start.chars().take(10).collect::<String>();
    is_date_key(&day).then_some(day)
}

fn overlaps_window(event: &Value, day: &str, zone: &str, start_hour: f64, end_hour: f64) -> bool {
    let (Ok(a), Ok(b)) = (
        s::zoned(day, start_hour, zone),
        s::zoned(day, end_hour, zone),
    ) else {
        return false;
    };
    match (epoch(&text(event, "start")), epoch(&text(event, "end"))) {
        (Some(x), Some(y)) => a < y && b > x,
        _ => event_day(event, zone).as_deref() == Some(day),
    }
}

fn event_fallback(event: &Value, rules: &[Value]) -> String {
    project(event, rules)
        .map(|r| text(r, "type"))
        .unwrap_or_default()
}

fn is_call(event: &Value, rules: &[Value]) -> bool {
    kind(event, &event_fallback(event, rules)) == "meetings"
}

fn is_deadline_event(event: &Value, rules: &[Value]) -> bool {
    kind(event, &event_fallback(event, rules)) == "deadline"
}

fn is_in_person(event: &Value) -> bool {
    event["inPerson"] == true
}

fn is_travel_or_in_person(event: &Value) -> bool {
    if is_in_person(event) {
        return true;
    }
    let cal = text(event, "calendarName").to_lowercase();
    let id = text(event, "calendarId").to_lowercase();
    cal.contains("tripit") || id.contains("tripit") || id.contains("synthetic-travel")
}

fn band_of(score: i64) -> (&'static str, usize) {
    if score <= 30 {
        ("light", 4)
    } else if score <= 55 {
        ("normal", 2)
    } else if score <= 75 {
        ("busy", 1)
    } else {
        ("heavy", 0)
    }
}

fn context_events(v: &Value) -> Vec<Value> {
    if v["weekEvents"].is_array() {
        array(&v["weekEvents"])
    } else {
        array(&v["events"])
    }
}

fn context_busy(v: &Value) -> Value {
    if v["weekBusy"].is_array() {
        v["weekBusy"].clone()
    } else {
        v["busy"].clone()
    }
}

fn score_input(v: &Value, day: &str) -> Value {
    let mut input = v.clone();
    input["day"] = json!(day);
    input["events"] = json!(context_events(v));
    input["busy"] = context_busy(v);
    input
}

pub fn score_components(v: &Value) -> Result<Value> {
    let day_key = text(v, "day");
    let zone = text(v, "zone");
    let start_h = number(&v["hours"], "start");
    let end_h = number(&v["hours"], "end");
    let capacity = ((end_h - start_h) * 60.0).max(0.0);
    let input = score_input(v, &day_key);
    let load = day(&input)?;
    let booked = number(&load, "busyMinutes");
    let off = off_calendar_minutes(&v["offCalendarLoad"], &day_key)?;
    let loaded = booked + off;
    let booked_score = if capacity <= 0.0 {
        if loaded > 0.0 { 40.0 } else { 0.0 }
    } else {
        (40.0 * (loaded / capacity)).min(40.0)
    };
    let rules = array(&v["rules"]);
    let events = context_events(v);
    let call_count = events
        .iter()
        .filter(|e| overlaps_window(e, &day_key, &zone, start_h, end_h) && is_call(e, &rules))
        .count();
    let call_score = ((call_count as i64) * 5).min(20) as f64;
    let project_count = load["projects"].as_object().map(|p| p.len()).unwrap_or(0);
    let switch_score = (project_count.saturating_sub(1) * 5).min(15) as f64;
    let day_events: Vec<_> = events
        .iter()
        .filter(|e| overlaps_window(e, &day_key, &zone, 0.0, 24.0))
        .collect();
    let in_person = day_events.iter().any(|e| is_in_person(e));
    let travel = day_events.iter().any(|e| is_travel_or_in_person(e));
    let travel_score = if travel { 15.0 } else { 0.0 };
    let tomorrow = s::shift(&day_key, 1)?;
    let deadline_count = events
        .iter()
        .filter(|e| {
            is_deadline_event(e, &rules)
                && event_day(e, &zone).is_some_and(|d| d == day_key || d == tomorrow)
        })
        .count();
    let deadline_score = ((deadline_count as i64) * 5).min(10) as f64;
    let raw = booked_score + call_score + switch_score + travel_score + deadline_score;
    let score = raw.clamp(0.0, 100.0).round() as i64;
    let (band, natural) = band_of(score);
    let band_slots = if in_person { 0 } else { natural };
    Ok(json!({
        "day": day_key,
        "score": score,
        "rawScore": raw,
        "band": band,
        "bandSlots": band_slots,
        "inPerson": in_person,
        "drivers": {
            "bookedMinutes": booked,
            "offCalendarMinutes": off,
            "capacityMinutes": capacity,
            "bookedTime": booked_score,
            "callCount": call_count,
            "calls": call_score,
            "projectCount": project_count,
            "projectSwitches": switch_score,
            "travelOrInPerson": travel_score,
            "deadlineCount": deadline_count,
            "deadlines": deadline_score
        }
    }))
}

fn booking_day(b: &Value, zone: &str) -> Option<String> {
    let start = if !text(&b["pendingChange"], "start").is_empty() {
        text(&b["pendingChange"], "start")
    } else {
        text(b, "start")
    };
    if let Some(ms) = epoch(&start) {
        return s::date(ms, zone).ok();
    }
    let day = start.chars().take(10).collect::<String>();
    is_date_key(&day).then_some(day)
}

pub fn week_bookings_count(v: &Value, monday: &str, zone: &str) -> Result<i64> {
    let friday = s::shift(monday, 4)?;
    let mut n = 0;
    for b in array(&v["weekBookings"]) {
        let status = text(&b, "status");
        if !status.is_empty() && status != "pending" && status != "confirmed" {
            continue;
        }
        if let Some(d) = booking_day(&b, zone) {
            if d.as_str() >= monday && d.as_str() <= friday.as_str() && s::weekday(&d)? % 6 != 0 {
                n += 1;
            }
        }
    }
    Ok(n)
}

pub fn week_plan(v: &Value) -> Result<Vec<Value>> {
    let monday = s::monday(&text(v, "day"))?;
    let zone = text(v, "zone");
    let bookings = week_bookings_count(v, &monday, &zone)?;
    let mut already = 0i64;
    let mut rows = vec![];
    for n in 0..5 {
        let d = s::shift(&monday, n)?;
        let report = score_components(&score_input(v, &d))?;
        let (mode, caps) = active_caps(&v["loadMode"], &d)?;
        let band = report["bandSlots"].as_i64().unwrap_or(0);
        let offered = if let Some((max_day, max_week)) = caps {
            let remaining = (max_week - bookings - already).max(0);
            let offered = band.min(max_day).min(remaining).max(0);
            already += offered;
            offered
        } else {
            band
        };
        rows.push(json!({
            "day": d,
            "score": report["score"],
            "rawScore": report["rawScore"],
            "band": report["band"],
            "bandSlots": report["bandSlots"],
            "offeredSlots": offered,
            "mode": mode,
            "inPerson": report["inPerson"],
            "weekBookings": bookings,
            "drivers": report["drivers"]
        }));
    }
    Ok(rows)
}

pub fn report(v: &Value) -> Result<Value> {
    let day = text(v, "day");
    week_plan(v)?
        .into_iter()
        .find(|r| text(r, "day") == day)
        .ok_or_else(|| anyhow::anyhow!("Score is only computed for Monday to Friday"))
}

pub fn offered_slots(v: &Value) -> Result<usize> {
    if s::weekday(&text(v, "day"))? % 6 == 0 {
        return Ok(0);
    }
    Ok(report(v)?["offeredSlots"].as_u64().unwrap_or(0) as usize)
}

fn driver_summary(r: &Value) -> String {
    let d = &r["drivers"];
    let mut parts = vec![format!(
        "booked time {:.1} ({}+{:.0}/{:.0} min)",
        d["bookedTime"].as_f64().unwrap_or(0.0),
        d["bookedMinutes"].as_f64().unwrap_or(0.0),
        d["offCalendarMinutes"].as_f64().unwrap_or(0.0),
        d["capacityMinutes"].as_f64().unwrap_or(0.0)
    )];
    let calls = d["callCount"].as_u64().unwrap_or(0);
    if calls > 0 {
        parts.push(format!(
            "{calls} calls {}",
            d["calls"].as_f64().unwrap_or(0.0)
        ));
    }
    let projects = d["projectCount"].as_u64().unwrap_or(0);
    if projects > 1 {
        parts.push(format!(
            "{} project switches {}",
            projects - 1,
            d["projectSwitches"].as_f64().unwrap_or(0.0)
        ));
    }
    if d["travelOrInPerson"].as_f64().unwrap_or(0.0) > 0.0 {
        parts.push(format!(
            "travel or in-person {}",
            d["travelOrInPerson"].as_f64().unwrap_or(0.0)
        ));
    }
    let deadlines = d["deadlineCount"].as_u64().unwrap_or(0);
    if deadlines > 0 {
        parts.push(format!(
            "{deadlines} deadlines {}",
            d["deadlines"].as_f64().unwrap_or(0.0)
        ));
    }
    if r["inPerson"] == true {
        parts.push("in-person forces 0 slots".into());
    }
    parts.join("; ")
}

pub fn preview(v: &Value) -> Result<Vec<Value>> {
    let mut rows = vec![];
    for d in array(&v["days"]) {
        let day = d.as_str().unwrap_or("").to_owned();
        ensure!(is_date_key(&day), "Preview days must be YYYY-MM-DD");
        let mut input = v.clone();
        input["day"] = json!(day);
        if s::weekday(&day)? % 6 == 0 {
            rows.push(json!({
                "date": day,
                "score": 0,
                "band": "light",
                "slots": 0,
                "drivers": "weekend",
                "mode": "normal",
                "inPerson": false,
                "weekBookings": 0
            }));
            continue;
        }
        let r = report(&input)?;
        rows.push(json!({
            "date": day,
            "score": r["score"],
            "band": r["band"],
            "slots": r["offeredSlots"],
            "drivers": driver_summary(&r),
            "driverParts": r["drivers"],
            "mode": r["mode"],
            "inPerson": r["inPerson"],
            "weekBookings": r["weekBookings"],
            "bandSlots": r["bandSlots"]
        }));
    }
    Ok(rows)
}

pub fn arrange(v: &Value) -> Result<Vec<Value>> {
    let mut c = array(&v["candidates"]);
    if c.is_empty() {
        return Ok(c);
    }
    let offered = offered_slots(v)?;
    if offered == 0 {
        return Ok(vec![]);
    }
    if c.len() <= offered {
        return Ok(c);
    }
    let load = day(v)?;
    let t = types(&load);
    let mut dominant = "meetings";
    for k in ["meetings", "focus", "deadline"] {
        if number(&t, k) > number(&t, dominant) {
            dominant = k
        }
    }
    if number(&t, dominant) < 60.0f64.max(number(&load, "busyMinutes") * 0.4) {
        dominant = "meetings"
    }
    if dominant == "focus" {
        return Ok(edges(c, offered));
    }
    let busy = clip(&array(&v["busy"]), bounds(v)?);
    let distance = |slot: &Value| {
        let a = epoch(&text(slot, "start")).unwrap_or(0);
        let b = epoch(&text(slot, "end")).unwrap_or(0);
        busy.iter()
            .map(|(x, y)| (a - y).abs().min((x - b).abs()))
            .min()
            .unwrap_or(0)
    };
    c.sort_by_key(|s| (distance(s), epoch(&text(s, "start"))));
    c.truncate(offered);
    c.sort_by_key(|s| epoch(&text(s, "start")));
    Ok(c)
}

pub fn rank(v: &Value) -> Result<Vec<Value>> {
    let zone = text(v, "zone");
    let mut history: HashMap<String, (f64, f64)> = HashMap::new();
    for item in array(&v["feedback"]) {
        let e = history
            .entry(s::bucket(&text(&item, "start"), &zone)?)
            .or_default();
        match text(&item, "outcome").as_str() {
            "confirmed" => e.0 += 1.0,
            "cancelled" | "rescheduled" => e.1 += 1.0,
            _ => (),
        }
    }
    let score = |slot: &Value| {
        let (a, b) = history
            .get(&s::bucket(&text(slot, "start"), &zone).unwrap_or_default())
            .copied()
            .unwrap_or_default();
        let learned = 3.0 * (a - b) / (a + b + 4.0);
        if v["meetingDay"] != true {
            return learned;
        }
        let x = epoch(&text(slot, "start")).unwrap_or(0);
        let y = epoch(&text(slot, "end")).unwrap_or(0);
        let distance = array(&v["busy"])
            .iter()
            .filter_map(|i| {
                Some(
                    (x - epoch(&text(i, "end"))?)
                        .abs()
                        .min((epoch(&text(i, "start"))? - y).abs()),
                )
            })
            .min()
            .unwrap_or(i64::MAX);
        learned
            + if distance <= 45 * 60_000 {
                1.0
            } else if distance <= 90 * 60_000 {
                0.5
            } else {
                0.0
            }
    };
    let mut c = array(&v["candidates"]);
    c.sort_by(|a, b| {
        score(b)
            .total_cmp(&score(a))
            .then_with(|| epoch(&text(a, "start")).cmp(&epoch(&text(b, "start"))))
    });
    Ok(c)
}
fn percentile(mut values: Vec<f64>, fraction: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    values[((values.len() as f64 * fraction).ceil() as usize).saturating_sub(1)]
}
pub fn learn(v: &Value) -> Result<Value> {
    let mut history = vec![];
    for n in 0..number(v, "days") as i64 {
        let d = s::shift(&text(v, "startDay"), n)?;
        if s::weekday(&d)? % 6 != 0 {
            let mut input = v.clone();
            input["day"] = json!(d);
            history.push(day(&input)?);
        }
    }
    let names: HashSet<_> = history
        .iter()
        .flat_map(|x| {
            x["projects"]
                .as_object()
                .map(|p| p.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default()
        })
        .collect();
    let mut typical = json!({});
    for name in names {
        typical[&name] = json!(percentile(
            history
                .iter()
                .filter_map(|x| x["projects"][&name]["minutes"].as_f64())
                .filter(|x| *x > 0.0)
                .collect(),
            0.5
        ));
    }
    Ok(
        json!({"updatedAt":iso(now()),"baselineBusyMinutes":percentile(history.iter().map(|x|number(x,"busyMinutes")).collect(),0.75),"typicalProjectMinutes":typical}),
    )
}
pub fn alert(v: &Value) -> Result<Value> {
    let mut loads = vec![];
    for d in array(&v["days"]) {
        let mut i = v.clone();
        i["day"] = d;
        loads.push(day(&i)?);
    }
    let baseline = 120.0f64.max(number(&v["learning"], "baselineBusyMinutes"));
    let heavy = loads
        .iter()
        .filter(|x| number(x, "busyMinutes") >= 180.0f64.max(baseline * 1.5))
        .count();
    let busy: f64 = loads.iter().map(|x| number(x, "busyMinutes")).sum();
    let mut t = json!({"meetings":0.0,"focus":0.0,"deadline":0.0});
    for x in &loads {
        let current = types(x);
        for k in ["meetings", "focus", "deadline"] {
            t[k] = json!(number(&t, k) + number(&current, k));
        }
    }
    if heavy < 3
        && busy < 900.0f64.max(baseline * 7.5)
        && !(number(&t, "deadline") >= 120.0 && busy >= baseline * 5.0)
    {
        return Ok(Value::Null);
    }
    let suggestion = if number(&t, "deadline") >= 120.0 {
        "protect_deadline"
    } else if number(&t, "focus") >= 60.0 && number(&t, "focus") > number(&t, "meetings") {
        "protect_focus"
    } else {
        "cluster_meetings"
    };
    Ok(
        json!({"heavyDays":heavy,"busyMinutes":busy,"typicalBusyMinutes":baseline*loads.len()as f64,"typeMinutes":t,"suggestion":suggestion}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iso;

    const ZONE: &str = "Europe/Rome";
    const MON: &str = "2026-11-16";
    const TUE: &str = "2026-11-17";
    const WED: &str = "2026-11-18";
    const THU: &str = "2026-11-19";
    const FRI: &str = "2026-11-20";

    fn rules() -> Value {
        json!([
            {"name":"Alpha","type":"meetings","keywords":["alpha"],"calendarIds":[]},
            {"name":"Beta","type":"focus","keywords":["beta"],"calendarIds":[]},
            {"name":"Gamma","type":"deadline","keywords":["gamma"],"calendarIds":[]},
            {"name":"Delta","type":"focus","keywords":["delta"],"calendarIds":[]}
        ])
    }

    fn ts(day: &str, hour: f64) -> String {
        iso(s::zoned(day, hour, ZONE).unwrap())
    }

    fn ev(day: &str, start: f64, end: f64, summary: &str) -> Value {
        json!({
            "start": ts(day, start),
            "end": ts(day, end),
            "summary": summary,
            "inPerson": false
        })
    }

    fn in_person(day: &str, start: f64, end: f64, summary: &str) -> Value {
        let mut e = ev(day, start, end, summary);
        e["inPerson"] = json!(true);
        e["location"] = json!("Via del Corso 1, Rome");
        e
    }

    fn cands(day: &str, n: usize) -> Vec<Value> {
        (0..n)
            .map(|i| {
                let start = 9.0 + i as f64 * 0.5;
                json!({"start": ts(day, start), "end": ts(day, start + 0.5)})
            })
            .collect()
    }

    fn base(day: &str, mode: &str) -> Value {
        json!({
            "day": day,
            "zone": ZONE,
            "hours": {"start": 9, "end": 18},
            "busy": [],
            "events": [],
            "weekEvents": [],
            "weekBusy": [],
            "weekBookings": [],
            "rules": rules(),
            "candidates": cands(day, 8),
            "loadMode": {"mode": mode},
            "offCalendarLoad": {"weeklyMinutes": 0, "byDate": {}, "byWeek": {}}
        })
    }

    fn with_events(mut v: Value, events: Vec<Value>) -> Value {
        let busy: Vec<Value> = events
            .iter()
            .map(|e| json!({"start": e["start"], "end": e["end"]}))
            .collect();
        v["events"] = json!(events.clone());
        v["weekEvents"] = json!(events);
        v["busy"] = json!(busy.clone());
        v["weekBusy"] = json!(busy);
        v
    }

    fn booking(day: &str, hour: f64, status: &str) -> Value {
        json!({"start": ts(day, hour), "end": ts(day, hour + 0.5), "status": status})
    }

    fn scored(v: &Value) -> Value {
        report(v).unwrap()
    }

    #[test]
    fn empty_day_is_light_four_slots_in_normal_mode() {
        let v = base(WED, "normal");
        let r = scored(&v);
        assert_eq!(r["score"], 0);
        assert_eq!(r["band"], "light");
        assert_eq!(r["bandSlots"], 4);
        assert_eq!(r["offeredSlots"], 4);
        assert_eq!(arrange(&v).unwrap().len(), 4);
    }

    #[test]
    fn default_and_ultra_heavy_cap_one_per_day() {
        let v = base(WED, "ultraHeavy");
        let r = scored(&v);
        assert_eq!(r["bandSlots"], 4);
        assert_eq!(r["offeredSlots"], 1);
        assert_eq!(r["mode"], "ultraHeavy");
        assert_eq!(arrange(&v).unwrap().len(), 1);
        let defaulted = {
            let mut x = v.clone();
            x["loadMode"] = Value::Null;
            scored(&x)
        };
        assert_eq!(defaulted["mode"], "ultraHeavy");
        assert_eq!(defaulted["offeredSlots"], 1);
    }

    #[test]
    fn heavy_caps_two_per_day_even_when_band_is_four() {
        let v = base(WED, "heavy");
        let r = scored(&v);
        assert_eq!(r["bandSlots"], 4);
        assert_eq!(r["offeredSlots"], 2);
        assert_eq!(arrange(&v).unwrap().len(), 2);
    }

    #[test]
    fn ultra_heavy_week_cap_distributes_three_days() {
        let v = base(MON, "ultraHeavy");
        let plan = week_plan(&v).unwrap();
        let offered: Vec<i64> = plan
            .iter()
            .map(|r| r["offeredSlots"].as_i64().unwrap())
            .collect();
        assert_eq!(offered, vec![1, 1, 1, 0, 0]);
    }

    #[test]
    fn heavy_week_cap_stops_at_six() {
        let v = base(MON, "heavy");
        let plan = week_plan(&v).unwrap();
        let offered: Vec<i64> = plan
            .iter()
            .map(|r| r["offeredSlots"].as_i64().unwrap())
            .collect();
        // 2 per day, 6 per week: Mon-Wed get 2, Thu-Fri get 0
        assert_eq!(offered, vec![2, 2, 2, 0, 0]);
    }

    #[test]
    fn week_cap_counts_confirmed_and_pending_bookings() {
        let mut v = base(MON, "ultraHeavy");
        v["weekBookings"] = json!([
            booking(MON, 10.0, "confirmed"),
            booking(TUE, 11.0, "pending"),
            booking(WED, 9.0, "cancelled")
        ]);
        let plan = week_plan(&v).unwrap();
        let offered: Vec<i64> = plan
            .iter()
            .map(|r| r["offeredSlots"].as_i64().unwrap())
            .collect();
        // 2 active bookings, max 3: one slot left, on Monday
        assert_eq!(plan[0]["weekBookings"], 2);
        assert_eq!(offered, vec![1, 0, 0, 0, 0]);
        // three already booked: nothing offered
        v["weekBookings"] = json!([
            booking(MON, 10.0, "confirmed"),
            booking(TUE, 11.0, "pending"),
            booking(FRI, 14.0, "confirmed")
        ]);
        let plan = week_plan(&v).unwrap();
        assert!(plan.iter().all(|r| r["offeredSlots"] == 0));
    }

    #[test]
    fn configurable_tier_caps() {
        let mut v = base(MON, "ultraHeavy");
        v["loadMode"] = json!({
            "mode": "ultraHeavy",
            "tiers": {"ultraHeavy": {"maxPerDay": 2, "maxPerWeek": 5}}
        });
        let offered: Vec<i64> = week_plan(&v)
            .unwrap()
            .iter()
            .map(|r| r["offeredSlots"].as_i64().unwrap())
            .collect();
        // band 4, day cap 2, week 5: 2+2+1+0+0
        assert_eq!(offered, vec![2, 2, 1, 0, 0]);
    }

    #[test]
    fn until_date_is_inclusive_then_reverts_to_normal() {
        let mut v = base(WED, "ultraHeavy");
        v["loadMode"] = json!({"mode": "ultraHeavy", "until": WED});
        assert_eq!(scored(&v)["mode"], "ultraHeavy");
        assert_eq!(scored(&v)["offeredSlots"], 1);
        v["day"] = json!(THU);
        v["candidates"] = json!(cands(THU, 8));
        assert_eq!(scored(&v)["mode"], "normal");
        assert_eq!(scored(&v)["offeredSlots"], 4);
    }

    #[test]
    fn booked_time_is_linear_against_workday_capacity() {
        // 270 of 540 minutes = 20 points
        let events = vec![ev(WED, 9.0, 13.5, "focus block")];
        let v = with_events(base(WED, "normal"), events);
        let r = scored(&v);
        assert_eq!(r["drivers"]["bookedMinutes"], 270.0);
        assert_eq!(r["drivers"]["capacityMinutes"], 540.0);
        assert_eq!(r["drivers"]["bookedTime"], 20.0);
        assert_eq!(r["score"], 20);
        assert_eq!(r["band"], "light");
    }

    #[test]
    fn off_calendar_weekly_minutes_spread_over_weekdays() {
        let mut v = base(WED, "normal");
        v["offCalendarLoad"] = json!({"weeklyMinutes": 2700, "byDate": {}, "byWeek": {}});
        // 2700/5 = 540, fills the day: 40 points
        let r = scored(&v);
        assert_eq!(r["drivers"]["offCalendarMinutes"], 540.0);
        assert_eq!(r["drivers"]["bookedTime"], 40.0);
        assert_eq!(r["score"], 40);
        assert_eq!(r["band"], "normal");
        // weekend gets no weekly spread
        assert_eq!(
            off_calendar_minutes(&v["offCalendarLoad"], "2026-11-21").unwrap(),
            0.0
        );
    }

    #[test]
    fn off_calendar_by_date_and_by_week_add() {
        let mut v = base(WED, "normal");
        v["offCalendarLoad"] = json!({
            "weeklyMinutes": 0,
            "byDate": {WED: 108},
            "byWeek": {MON: 540}
        });
        // week 540/5=108, plus 108 date = 216; 40*216/540=16
        let r = scored(&v);
        assert_eq!(r["drivers"]["offCalendarMinutes"], 216.0);
        assert_eq!(r["drivers"]["bookedTime"], 16.0);
        // another weekday only gets the weekly share
        v["day"] = json!(THU);
        assert_eq!(scored(&v)["drivers"]["offCalendarMinutes"], 108.0);
    }

    #[test]
    fn calls_are_five_each_capped_at_twenty() {
        let events = (0..5)
            .map(|i| ev(WED, 9.0 + i as f64, 9.5 + i as f64, "team call"))
            .collect();
        let v = with_events(base(WED, "normal"), events);
        let r = scored(&v);
        assert_eq!(r["drivers"]["callCount"], 5);
        assert_eq!(r["drivers"]["calls"], 20.0);
        // five 30-minute calls: 150 booked minutes → 40*150/540≈11.11, plus 20 → 31
        assert_eq!(r["score"], 31);
        // a sixth call stays capped
        let six = with_events(
            base(WED, "normal"),
            (0..6)
                .map(|i| ev(WED, 9.0 + i as f64, 9.5 + i as f64, "team call"))
                .collect(),
        );
        assert_eq!(scored(&six)["drivers"]["calls"], 20.0);
    }

    #[test]
    fn project_switches_after_the_first_capped_at_fifteen() {
        let events = vec![
            ev(WED, 9.0, 10.0, "alpha review"),
            ev(WED, 10.0, 11.0, "beta deep work"),
            ev(WED, 11.0, 12.0, "gamma [deadline]"),
            ev(WED, 13.0, 14.0, "delta focus block"),
        ];
        let v = with_events(base(WED, "normal"), events);
        let r = scored(&v);
        assert_eq!(r["drivers"]["projectCount"], 4);
        assert_eq!(r["drivers"]["projectSwitches"], 15.0);
    }

    #[test]
    fn travel_adds_fifteen_and_in_person_forces_zero_slots() {
        let v = with_events(
            base(WED, "normal"),
            vec![in_person(WED, 15.0, 16.0, "coffee in Rome")],
        );
        let r = scored(&v);
        assert_eq!(r["drivers"]["travelOrInPerson"], 15.0);
        assert_eq!(r["inPerson"], true);
        assert_eq!(r["bandSlots"], 0);
        assert_eq!(r["offeredSlots"], 0);
        // 60 booked minutes → 40*60/540≈4.44, plus travel 15 → 19
        assert_eq!(r["score"], 19);
        assert!(arrange(&v).unwrap().is_empty());
    }

    #[test]
    fn tripit_travel_without_in_person_adds_fifteen_but_keeps_slots() {
        let mut e = ev(WED, 7.0, 8.0, "UA 123 SFO-FCO");
        e["calendarName"] = json!("TripIt");
        e["calendarId"] = json!("tripit#family");
        let v = with_events(base(WED, "normal"), vec![e]);
        let r = scored(&v);
        assert_eq!(r["drivers"]["travelOrInPerson"], 15.0);
        assert_eq!(r["inPerson"], false);
        assert_eq!(r["bandSlots"], 4); // 15 points, light
        assert_eq!(r["offeredSlots"], 4);
    }

    #[test]
    fn deadlines_today_or_tomorrow_capped_at_ten() {
        let events = vec![
            ev(WED, 9.0, 9.5, "[deadline] ship alpha"),
            ev(THU, 9.0, 9.5, "deadline: beta review"),
            ev(THU, 14.0, 14.5, "gamma launch day"),
            ev(FRI, 9.0, 9.5, "[deadline] later"),
        ];
        let v = with_events(base(WED, "normal"), events);
        let r = scored(&v);
        assert_eq!(r["drivers"]["deadlineCount"], 3);
        assert_eq!(r["drivers"]["deadlines"], 10.0);
        // Friday-only deadline is not today or tomorrow from Wednesday
    }

    #[test]
    fn band_boundaries() {
        assert_eq!(band_of(0), ("light", 4));
        assert_eq!(band_of(30), ("light", 4));
        assert_eq!(band_of(31), ("normal", 2));
        assert_eq!(band_of(55), ("normal", 2));
        assert_eq!(band_of(56), ("busy", 1));
        assert_eq!(band_of(75), ("busy", 1));
        assert_eq!(band_of(76), ("heavy", 0));
        assert_eq!(band_of(100), ("heavy", 0));
    }

    #[test]
    fn heavy_band_offers_nothing_even_in_normal_mode() {
        // Full day 40 + 4 calls 20 + 3 switches 15 + travel 15 + 2 deadlines 10 = 100
        let mut travel = ev(WED, 8.0, 8.5, "flight");
        travel["calendarName"] = json!("TripIt");
        let events = vec![
            ev(WED, 9.0, 18.0, "alpha call"),
            ev(WED, 9.0, 9.5, "beta call"),
            ev(WED, 10.0, 10.5, "gamma [deadline]"),
            ev(WED, 11.0, 11.5, "delta call"),
            ev(WED, 12.0, 12.5, "standup call"),
            ev(THU, 9.0, 9.5, "[deadline] tomorrow"),
            travel,
        ];
        let v = with_events(base(WED, "normal"), events);
        let r = scored(&v);
        assert!(
            r["score"].as_i64().unwrap() >= 76,
            "expected heavy score, got {}",
            r
        );
        assert_eq!(r["band"], "heavy");
        assert_eq!(r["offeredSlots"], 0);
        assert!(arrange(&v).unwrap().is_empty());
    }

    #[test]
    fn normalize_rejects_unknown_mode_and_negative_caps() {
        assert!(normalize_load_mode(&json!("crazy")).is_err());
        assert!(
            normalize_load_mode(
                &json!({"mode":"heavy","tiers":{"heavy":{"maxPerDay":-1,"maxPerWeek":6}}})
            )
            .is_err()
        );
        assert!(normalize_off_calendar_load(&json!(-5)).is_err());
        let ok = normalize_load_mode(&json!("heavy")).unwrap();
        assert_eq!(ok["mode"], "heavy");
        assert_eq!(ok["tiers"]["heavy"]["maxPerDay"], 2);
    }

    #[test]
    fn preview_table_uses_ultra_heavy() {
        let mut v = base(MON, "ultraHeavy");
        v["days"] = json!([MON, TUE, WED, THU, FRI, "2026-11-23"]);
        let rows = preview(&v).unwrap();
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[0]["slots"], 1);
        assert_eq!(rows[1]["slots"], 1);
        assert_eq!(rows[2]["slots"], 1);
        assert_eq!(rows[3]["slots"], 0);
        assert_eq!(rows[4]["slots"], 0);
        assert_eq!(rows[5]["slots"], 1); // next Monday, new week
        assert_eq!(rows[0]["mode"], "ultraHeavy");
        assert!(rows[0]["drivers"].as_str().unwrap().contains("booked time"));
    }
}
