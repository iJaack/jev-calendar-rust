use crate::providers::{Providers, choice};
use anyhow::ensure;
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
