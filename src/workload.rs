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
pub fn arrange(v: &Value) -> Result<Vec<Value>> {
    let mut c = array(&v["candidates"]);
    if c.is_empty() {
        return Ok(c);
    }
    let load = day(v)?;
    let t = types(&load);
    let span = (number(&v["hours"], "end") - number(&v["hours"], "start")) * 60.0;
    let baseline = number(&v["learning"], "baselineBusyMinutes");
    let spike = load["projects"].as_object().is_some_and(|p| {
        p.iter().any(|(n, p)| {
            number(p, "minutes")
                >= 120.0f64.max(
                    v["learning"]["typicalProjectMinutes"][n]
                        .as_f64()
                        .unwrap_or(0.0)
                        * 1.5,
                )
        })
    });
    let heavy = number(&load, "busyMinutes") >= 180.0f64.max((span * 0.6).min(baseline * 1.25))
        || (spike && number(&load, "busyMinutes") >= 120.0);
    if heavy {
        if number(&t, "deadline") >= 120.0 {
            return Ok(vec![]);
        }
        let count = if number(&load, "busyMinutes") >= span * 0.65 {
            1
        } else {
            2
        };
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
            c = edges(c, count)
        } else {
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
            c.truncate(count);
            c.sort_by_key(|s| epoch(&text(s, "start")));
        }
    }
    let mut context = v.clone();
    if v["weekEvents"].is_array() {
        context["events"] = v["weekEvents"].clone()
    }
    if v["weekBusy"].is_array() {
        context["busy"] = v["weekBusy"].clone()
    }
    match deadline(&context)? {
        Some(0) => return Ok(vec![]),
        Some(1) => return Ok(edges(c, 1)),
        Some(2) => return Ok(edges(c, 2)),
        _ => (),
    };
    let elsewhere = week(&context)?
        .iter()
        .any(|x| x["day"] != v["day"] && meeting(x));
    if number(&t, "focus") >= 90.0
        || (elsewhere && number(&t, "meetings") < 30.0 && number(&load, "meetings") < 2.0)
    {
        return Ok(edges(c, 2));
    }
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
