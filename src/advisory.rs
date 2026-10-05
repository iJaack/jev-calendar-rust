use crate::{
    epoch, number,
    providers::{MODEL, Providers, choice},
    text,
};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
fn questions(kind: &str) -> Value {
    let q: Value = serde_json::from_str(include_str!("../fixtures/jev-questions.json"))
        .expect("validated questions");
    q[kind].clone()
}
fn log(p: &Providers, e: &anyhow::Error) {
    if p.env("JEV_LOG_FALLBACK") == "1" {
        let reason = if e.to_string() == "unconfigured" {
            "unconfigured"
        } else if e.to_string() == "low_confidence" {
            "low_confidence"
        } else {
            "request_failed"
        };
        eprintln!("Jev fallback: {}", json!({"reason":reason}));
    }
}
pub async fn notice(p: &Providers, base: i64, baseline: f64, days: &[Value]) -> Value {
    let work = async {
        let state = json!({
            "schemaVersion": 1,
            "purpose": "booking_notice",
            "ownerMinimumNoticeMinutes": base,
            "typicalDailyBusyMinutes": baseline,
            "upcomingDays": days,
        });
        let response = p.jev(&state, &questions("notice")).await?;
        let (selected, confidence) = choice(
            &response["answers"]["booking_notice"],
            &["BASE", "EXTEND_50", "DOUBLE"],
        )
        .ok_or_else(|| anyhow::anyhow!("invalid_answer"))?;
        ensure!(confidence >= 0.55, "low_confidence");
        let minutes = match selected.as_str() {
            "EXTEND_50" => ((base as f64 * 1.5).ceil() as i64).min(10080),
            "DOUBLE" => (base * 2).min(10080),
            _ => base,
        };
        Ok::<_, anyhow::Error>(json!({"source": "jev", "minutes": minutes}))
    }
    .await;
    match work {
        Ok(value) => value,
        Err(error) => {
            log(p, &error);
            json!({"source": "fallback", "minutes": base})
        }
    }
}

pub async fn week(p: &Providers, alert: &Value) -> Value {
    let work = async {
        let state = json!({
            "schemaVersion": 1,
            "purpose": "weekly_workload_suggestion",
            "heavyDays": alert["heavyDays"],
            "busyMinutes": alert["busyMinutes"],
            "typicalBusyMinutes": alert["typicalBusyMinutes"],
            "typeMinutes": alert["typeMinutes"],
        });
        let response = p.jev(&state, &questions("week")).await?;
        let (selected, confidence) = choice(
            &response["answers"]["weekly_adjustment"],
            &["cluster_meetings", "protect_focus", "protect_deadline"],
        )
        .ok_or_else(|| anyhow::anyhow!("invalid_answer"))?;
        ensure!(confidence >= 0.55, "low_confidence");
        Ok::<_, anyhow::Error>(json!({"source": "jev", "suggestion": selected}))
    }
    .await;
    match work {
        Ok(value) => value,
        Err(error) => {
            log(p, &error);
            json!({"source": "fallback", "suggestion": alert["suggestion"]})
        }
    }
}

pub async fn travel(
    p: &Providers,
    route: &Value,
    direction: &str,
    candidates: &[Value],
    minimum: f64,
) -> Value {
    let work = async {
        let state = json!({
            "schemaVersion": 1,
            "model": MODEL,
            "routeDurationBucketMinutes": (number(route, "seconds") / 300.0).round() * 5.0,
            "routeDistanceBucketKm": route["distanceMeters"].as_f64()
                .filter(|distance| *distance >= 0.0)
                .map(|distance| (distance / 1000.0).round()),
            "tripDirection": direction,
            "ownerMinimumBufferMinutes": minimum.max(0.0).round(),
            "candidateWindows": candidates.iter().take(12).cloned().collect::<Vec<_>>(),
        });
        let response = p.jev(&state, &questions("travel")).await?;
        let (mode, mode_confidence) = choice(
            &response["answers"]["travel_mode"],
            &["DRIVE", "WALK", "BICYCLE", "TRANSIT"],
        )
        .ok_or_else(|| anyhow::anyhow!("invalid_answer"))?;
        let (buffer, buffer_confidence) = choice(
            &response["answers"]["extra_buffer"],
            &["MIN_0", "MIN_5", "MIN_10", "MIN_15"],
        )
        .ok_or_else(|| anyhow::anyhow!("invalid_answer"))?;
        ensure!(
            mode_confidence >= 0.55 || buffer_confidence >= 0.55,
            "low_confidence"
        );
        let mode = if mode_confidence >= 0.55 {
            mode
        } else {
            "DRIVE".into()
        };
        let extra_buffer = if buffer_confidence >= 0.55 {
            buffer
                .strip_prefix("MIN_")
                .unwrap_or("0")
                .parse::<i64>()
                .unwrap_or(0)
        } else {
            0
        };
        Ok::<_, anyhow::Error>(json!({
            "source": "jev", "mode": mode, "extraBufferMinutes": extra_buffer,
        }))
    }
    .await;
    match work {
        Ok(value) => value,
        Err(error) => {
            log(p, &error);
            json!({"source": "fallback", "mode": "DRIVE", "extraBufferMinutes": 0})
        }
    }
}

pub async fn constraints(
    p: &Providers,
    demo: bool,
    events: &[Value],
    base: &str,
    candidates: &[Value],
    minimum: f64,
) -> Result<Value> {
    let physical: Vec<_> = events
        .iter()
        .filter(|e| {
            e["inPerson"] == true
                && epoch(&text(e, "start")).is_some()
                && epoch(&text(e, "end")).is_some()
        })
        .collect();
    let count = physical.len();
    if count == 0 {
        return Ok(
            json!({"constraints":[],"eventCount":0,"maximumBufferMinutes":minimum.max(0.0),"routeProvider":"none","adviceSource":"none"}),
        );
    }
    ensure!(
        !base.trim().is_empty(),
        "Jaack must configure a base location before in-person travel buffers can be calculated"
    );
    let mut constraints = vec![];
    let mut providers = vec![];
    let mut sources = vec![];
    let mut maximum = minimum;
    for event in physical {
        let mut buffers = [minimum, minimum];
        for (i, direction) in ["TO_EVENT", "FROM_EVENT"].iter().enumerate() {
            let adjacent: Vec<_> = candidates
                .iter()
                .filter(|slot| {
                    if i == 0 {
                        epoch(&text(slot, "end")) <= epoch(&text(event, "start"))
                    } else {
                        epoch(&text(slot, "start")) >= epoch(&text(event, "end"))
                    }
                })
                .cloned()
                .collect();
            if adjacent.is_empty() {
                continue;
            }
            let location = text(event, "location");
            let (origin, destination) = if i == 0 {
                (base, location.as_str())
            } else {
                (location.as_str(), base)
            };
            let departure = if i == 0 {
                text(adjacent.last().expect("nonempty"), "end")
            } else {
                text(event, "end")
            };
            let get = |mode: String| {
                let departure = departure.clone();
                async move {
                    if demo {
                        let seed: u32 = format!("{origin}|{destination}|{mode}")
                            .encode_utf16()
                            .map(u32::from)
                            .sum();
                        Ok(json!({"seconds":720+(seed%24)*60,"provider":"demo-routes","mode":mode}))
                    } else {
                        p.route(origin, destination, &mode, &departure).await
                    }
                }
            };
            let mut route = get("DRIVE".into()).await?;
            let mut advice = travel(p, &route, direction, &adjacent, minimum).await;
            let mode = text(&advice, "mode");
            if mode != "DRIVE" {
                match get(mode).await {
                    Ok(r) => route = r,
                    Err(_) => {
                        advice = json!({"source":"fallback","mode":"DRIVE","extraBufferMinutes":0})
                    }
                }
            }
            buffers[i] = (number(&route, "seconds") / 60.0).ceil()
                + minimum.max(0.0)
                + number(&advice, "extraBufferMinutes");
            let provider = text(&route, "provider");
            if !providers.contains(&provider) {
                providers.push(provider)
            }
            sources.push(text(&advice, "source"));
        }
        maximum = maximum.max(buffers[0]).max(buffers[1]);
        constraints.push(json!({"start":event["start"],"end":event["end"],"beforeBufferMinutes":buffers[0],"afterBufferMinutes":buffers[1]}));
    }
    Ok(
        json!({"constraints":constraints,"eventCount":count,"maximumBufferMinutes":maximum,"routeProvider":if providers.is_empty(){"none".into()}else{providers.join(",")},"adviceSource":if sources.iter().any(|s|s=="fallback"){"fallback"}else if sources.iter().any(|s|s=="jev"){"jev"}else{"none"}}),
    )
}
