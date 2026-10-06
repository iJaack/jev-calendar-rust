use crate::{
    advisory, array, crypto, epoch, hash, iso, now, number,
    providers::{self, Calendar, Providers},
    scheduling as s,
    store::Store,
    text, workload as w,
};
use anyhow::{Result, anyhow, ensure};
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::State,
    http::{HeaderMap, Method, Request, Response},
    routing::any,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;

const CALENDAR_REFRESH_MS: u64 = 60_000;
const PREVIEW_MAX_AGE_MS: u64 = 120_000;

struct CalendarSnapshot {
    value: Value,
    fingerprint: String,
    started: Instant,
}
#[derive(Default)]
struct PreviewState {
    snapshot: Option<CalendarSnapshot>,
    generation: u64,
}

const WORKLOAD_REFRESH_MS: i64 = 6 * 60 * 60 * 1000;

fn workload_refresh_remaining(saved: &Value, time: i64) -> i64 {
    epoch(&text(saved, "updatedAt"))
        .filter(|updated| *updated <= time)
        .map(|updated| (WORKLOAD_REFRESH_MS - (time - updated)).max(0))
        .unwrap_or(0)
}

pub struct App {
    pub db: Arc<Store>,
    pub providers: Providers,
    pub env: HashMap<String, String>,
    pub demo: bool,
    pub secret: String,
    pub base: String,
    pub origin: String,
    pub operations: tokio::sync::Mutex<()>,
    refresh: tokio::sync::Mutex<()>,
    previews: Mutex<PreviewState>,
    preview_changed: tokio::sync::Notify,
    pub(crate) demo_state: Mutex<Value>,
    limits: Mutex<HashMap<String, (i64, u32)>>,
}
fn error(status: u16, message: &str) -> Response<Body> {
    response(status, json!({"error":message}))
}
fn response(status: u16, v: Value) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json; charset=utf-8")
        .header("Cache-Control", "no-store")
        .body(Body::from(v.to_string()))
        .expect("valid response")
}
fn header<'a>(h: &'a HeaderMap, k: &str) -> &'a str {
    h.get(k).and_then(|v| v.to_str().ok()).unwrap_or("")
}
fn cookie(h: &HeaderMap, k: &str) -> String {
    header(h, "cookie")
        .split(';')
        .find_map(|c| {
            let (a, b) = c.trim().split_once('=')?;
            (a == k).then(|| {
                url::form_urlencoded::parse(format!("x={b}").as_bytes())
                    .next()
                    .map(|(_, v)| v.into_owned())
                    .unwrap_or_default()
            })
        })
        .unwrap_or_default()
}
fn merge(a: &Value, b: &Value) -> Value {
    let mut result = a.clone();
    if let Some(b) = b.as_object() {
        for (k, v) in b {
            result[k] = v.clone()
        }
    }
    result
}
pub fn default_settings() -> Value {
    json!({"hours":{"start":9,"end":18},"timezone":"Europe/Rome","timezoneSource":"settings","timezoneUpdatedAt":null,"durationMinutes":30,"baseLocation":"","baseLocationProvider":"","baseLocationProviderId":"","baseLocationCoordinates":null,"minimumTravelBufferMinutes":10,"minimumBookingNoticeMinutes":0})
}
impl App {
    pub(crate) fn env(&self, k: &str) -> String {
        self.env.get(k).cloned().unwrap_or_default()
    }
    pub fn settings(&self) -> Result<Value> {
        Ok(merge(
            &default_settings(),
            &self.db.setting("settings", default_settings())?,
        ))
    }
    fn rules(&self) -> Result<Vec<Value>> {
        w::normalize(&self.db.setting("projectRules", json!(w::defaults()))?)
    }
    fn signature(settings: &Value, rules: &[Value], zone: &str) -> String {
        crate::canonical(&json!([
            "week-intelligence-v1",
            settings["hours"],
            settings["minimumBookingNoticeMinutes"],
            rules,
            zone
        ]))
        .to_string()
    }
    fn learning(&self, settings: &Value, rules: &[Value], zone: &str) -> Result<Value> {
        let saved = self.db.setting("workloadLearning", Value::Null)?;
        Ok(
            if saved["signature"] == Self::signature(settings, rules, zone) {
                saved
            } else {
                Value::Null
            },
        )
    }
    fn notice(settings: &Value, learning: &Value) -> f64 {
        number(settings, "minimumBookingNoticeMinutes").max(
            learning["bookingNotice"]["minutes"]
                .as_i64()
                .map(|v| v as f64)
                .unwrap_or(number(settings, "minimumBookingNoticeMinutes")),
        )
    }
    fn owner(&self, h: &HeaderMap) -> Result<bool> {
        Ok(self.demo || self.db.valid_session(&cookie(h, "jev_session"))?)
    }
    fn bearer(&self, h: &HeaderMap, write: bool) -> bool {
        let secret = self.env(if write {
            "JEV_JAACK_OS_WRITE_TOKEN"
        } else {
            "JEV_JAACK_OS_READ_TOKEN"
        });
        let raw = header(h, "authorization")
            .strip_prefix("Bearer ")
            .unwrap_or("");
        secret.len() >= 32
            && (!write || secret != self.env("JEV_JAACK_OS_READ_TOKEN"))
            && !raw.is_empty()
            && raw
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            && bool::from(hash(&secret).as_bytes().ct_eq(hash(raw).as_bytes()))
    }
    fn mutation(&self, h: &HeaderMap) -> Result<bool> {
        Ok(self.owner(h)?
            && header(h, "x-jev-owner") == "1"
            && header(h, "origin") == url::Url::parse(&self.base)?.origin().ascii_serialization())
    }
    async fn host(&self, settings: &Value, calendar: Option<&Calendar>) -> Result<Value> {
        let zone = text(settings, "timezone");
        if settings["timezoneSource"] == "owner-device"
            && epoch(&text(settings, "timezoneUpdatedAt")).is_some_and(|t| now() - t < 600_000)
            && s::valid_zone(&zone)
        {
            return Ok(json!({"timezone":zone,"source":"owner-device"}));
        }
        let from = if self.demo {
            Ok("Europe/Rome".into())
        } else {
            match calendar {
                Some(c) => self.providers.timezone(c).await,
                None => Err(anyhow!("Calendar unavailable")),
            }
        };
        if let Ok(z) = from
            && s::valid_zone(&z)
        {
            if z != zone || settings["timezoneSource"] != "google-calendar" {
                self.db.put("settings",&merge(settings,&json!({"timezone":z,"timezoneSource":"google-calendar","timezoneUpdatedAt":iso(now())})))?;
            }
            return Ok(json!({"timezone":z,"source":"google-calendar"}));
        }
        Ok(
            json!({"timezone":if s::valid_zone(&zone){zone}else{"Europe/Rome".into()},"source":"settings"}),
        )
    }
    async fn calendar_data(
        &self,
        calendar: Option<&Calendar>,
        start: &str,
        end: &str,
    ) -> Result<(Vec<Value>, Vec<Value>)> {
        if self.demo {
            let day = start.get(..10).unwrap_or("");
            let busy = vec![
                json!({"start":format!("{day}T10:30:00.000Z"),"end":format!("{day}T11:15:00.000Z")}),
                json!({"start":format!("{day}T14:00:00.000Z"),"end":format!("{day}T15:00:00.000Z")}),
            ];
            return Ok((
                busy.into_iter()
                    .filter(|b| s::overlaps(start, end, b))
                    .collect(),
                vec![],
            ));
        }
        let c = calendar.ok_or_else(|| anyhow!("Google Calendar is not connected"))?;
        tokio::try_join!(
            self.providers.free_busy(c, start, end),
            self.providers.events(c, start, end, true)
        )
    }
    fn preview_max_age_ms(&self) -> u64 {
        if self.env("JEV_TEST_MODE") == "true" && self.env("JEV_ALLOW_LIVE_STATE") != "1" {
            self.env("JEV_TEST_PREVIEW_MAX_AGE_MS")
                .parse::<u64>()
                .unwrap_or(PREVIEW_MAX_AGE_MS)
                .clamp(100, PREVIEW_MAX_AGE_MS)
        } else {
            PREVIEW_MAX_AGE_MS
        }
    }
    fn preview_fingerprint(&self) -> Result<String> {
        let settings = self.settings()?;
        Ok(crate::canonical(&json!([
            self.db.accounts()?,
            self.db.setting("googleBookingAccountId", Value::Null)?,
            settings["timezone"],
            settings["timezoneSource"],
            settings["timezoneUpdatedAt"]
        ]))
        .to_string())
    }
    fn invalidate_preview(&self) {
        if let Ok(mut state) = self.previews.lock() {
            state.generation += 1;
            state.snapshot = None;
        }
        self.preview_changed.notify_one();
    }
    async fn refresh_preview(&self) -> Result<()> {
        let generation = self
            .previews
            .lock()
            .map_err(|_| anyhow!("Calendar snapshot unavailable"))?
            .generation;
        let started = Instant::now();
        let calendar = self.providers.calendar()?;
        let settings = self.settings()?;
        let host = self.host(&settings, Some(&calendar)).await?;
        let fingerprint = self.preview_fingerprint()?;
        let zone = text(&host, "timezone");
        let today = s::date(now(), &zone)?;
        // Include adjacent viewer days and full weekly workload context for the 90-day horizon.
        let start = iso(s::zoned(&s::monday(&s::shift(&today, -2)?)?, 0.0, &zone)?);
        let end = iso(s::zoned(&s::shift(&today, 110)?, 0.0, &zone)?);
        let (busy, events) = self.calendar_data(Some(&calendar), &start, &end).await?;
        ensure!(
            fingerprint == self.preview_fingerprint()?,
            "Calendar configuration changed during refresh"
        );
        ensure!(
            started.elapsed().as_millis() < PREVIEW_MAX_AGE_MS as u128,
            "Calendar refresh took too long"
        );
        let mut state = self
            .previews
            .lock()
            .map_err(|_| anyhow!("Calendar snapshot unavailable"))?;
        ensure!(
            generation == state.generation,
            "Calendar changed during refresh"
        );
        state.snapshot = Some(CalendarSnapshot {
            value: json!({"host":host,"busy":busy,"events":events,"start":start,"end":end}),
            fingerprint,
            started,
        });
        Ok(())
    }
    fn preview_snapshot(&self) -> Result<Value> {
        let fingerprint = self.preview_fingerprint()?;
        let state = self
            .previews
            .lock()
            .map_err(|_| anyhow!("Calendar snapshot unavailable"))?;
        let saved = state
            .snapshot
            .as_ref()
            .ok_or_else(|| anyhow!("Calendar is refreshing. Please try again shortly."))?;
        let age = saved.started.elapsed().as_millis() as u64;
        ensure!(
            age < self.preview_max_age_ms() && saved.fingerprint == fingerprint,
            "Calendar is refreshing. Please try again shortly."
        );
        let mut snapshot = saved.value.clone();
        snapshot["settings"] = self.settings()?;
        snapshot["rules"] = json!(self.rules()?);
        snapshot["feedback"] = json!(self.db.feedback()?);
        snapshot["snapshotAgeMs"] = json!(age);
        snapshot["previewMaxAgeMs"] = json!(self.preview_max_age_ms());
        Ok(snapshot)
    }
    async fn preview_availability(&self, input: &Value) -> Result<Value> {
        if self.demo {
            return self.availability(input, None, None, 12).await;
        }
        let settings = self.settings()?;
        let viewer = if s::valid_zone(&text(input, "viewerTimeZone")) {
            text(input, "viewerTimeZone")
        } else {
            text(&settings, "timezone")
        };
        let day = text(input, "date");
        // Preserve explicit-date reads outside the public page's 90-day horizon.
        if !day.is_empty() {
            let today = s::date(now(), &viewer)?;
            if day < today || day > s::shift(&today, 90)? {
                return self.availability(input, None, None, 12).await;
            }
        }
        let snapshot = self.preview_snapshot()?;
        let mut result = self.availability(input, None, Some(&snapshot), 12).await?;
        result["snapshotAgeMs"] = snapshot["snapshotAgeMs"].clone();
        result["previewMaxAgeMs"] = snapshot["previewMaxAgeMs"].clone();
        Ok(result)
    }
    pub async fn availability(
        &self,
        input: &Value,
        calendar: Option<&Calendar>,
        snapshot: Option<&Value>,
        limit: usize,
    ) -> Result<Value> {
        let owned = if !self.demo && calendar.is_none() && snapshot.is_none() {
            Some(self.providers.calendar()?)
        } else {
            None
        };
        let calendar = calendar.or(owned.as_ref());
        let settings = if let Some(s) = snapshot {
            s["settings"].clone()
        } else {
            self.settings()?
        };
        let duration = input["durationMinutes"]
            .as_f64()
            .filter(|v| *v != 0.0)
            .unwrap_or(number(&settings, "durationMinutes"))
            .clamp(15.0, 180.0);
        let host = if let Some(s) = snapshot {
            s["host"].clone()
        } else {
            self.host(&settings, calendar).await?
        };
        let zone = text(&host, "timezone");
        let viewer = if s::valid_zone(&text(input, "viewerTimeZone")) {
            text(input, "viewerTimeZone")
        } else {
            zone.clone()
        };
        let date = if regex::Regex::new(r"^\d{4}-\d{2}-\d{2}$")
            .expect("static regex")
            .is_match(&text(input, "date"))
        {
            text(input, "date")
        } else {
            s::date(now() + 86_400_000, &viewer)?
        };
        let va = s::zoned(&date, 0.0, &viewer)?;
        let vb = s::zoned(&date, 24.0, &viewer)?;
        let mut days = vec![s::date(va, &zone)?, s::date(vb - 1, &zone)?];
        days.sort();
        days.dedup();
        days.retain(|d| s::weekday(d).is_ok_and(|n| n > 0 && n < 6));
        let mut result = json!({"date":date,"durationMinutes":duration,"candidates":[],"travel":{"inPersonEventCount":0,"maximumBufferMinutes":settings["minimumTravelBufferMinutes"],"routeProvider":"none","adviceSource":"none"},"hostTimezone":zone,"hostTimezoneSource":host["source"],"viewerTimeZone":viewer,"demo":self.demo});
        if days.is_empty() {
            return Ok(result);
        }
        let start = iso(s::zoned(&days[0], 0.0, &zone)?);
        let end = iso(s::zoned(days.last().expect("nonempty"), 24.0, &zone)?);
        let ca = iso(s::zoned(&s::monday(&days[0])?, 0.0, &zone)?);
        let cb = iso(s::zoned(
            &s::shift(&s::monday(days.last().expect("nonempty"))?, 12)?,
            0.0,
            &zone,
        )?);
        let (context_busy, context_events) = if let Some(snapshot) = snapshot {
            (
                array(&snapshot["busy"])
                    .into_iter()
                    .filter(|v| s::overlaps(&ca, &cb, v))
                    .collect(),
                array(&snapshot["events"])
                    .into_iter()
                    .filter(|v| s::overlaps(&ca, &cb, v))
                    .collect(),
            )
        } else {
            self.calendar_data(calendar, &ca, &cb).await?
        };
        let busy: Vec<_> = context_busy
            .iter()
            .filter(|v| s::overlaps(&start, &end, v))
            .cloned()
            .collect();
        let events: Vec<_> = context_events
            .iter()
            .filter(|v| s::overlaps(&start, &end, v))
            .cloned()
            .collect();
        let rules = if let Some(snapshot) = snapshot {
            array(&snapshot["rules"])
        } else {
            self.rules()?
        };
        let learning = self.learning(&settings, &rules, &zone)?;
        let cutoff = now() + (Self::notice(&settings, &learning) * 60_000.0) as i64;
        let mut preliminary = vec![];
        for day in &days {
            preliminary.extend(s::candidates(&json!({"day":day,"busy":busy,"durationMinutes":duration,"bufferMinutes":settings["minimumTravelBufferMinutes"],"hours":settings["hours"],"zone":zone,"limit":96,"notBefore":cutoff}))?)
        }
        preliminary.retain(|v| epoch(&text(v, "start")).is_some_and(|t| t >= va && t < vb));
        preliminary.sort_by_key(|v| epoch(&text(v, "start")));
        let travel = json!({
            "eventCount": events.iter().filter(|event| event["inPerson"] == true).count(),
            "maximumBufferMinutes": settings["minimumTravelBufferMinutes"],
            "routeProvider": "none",
            "adviceSource": "none",
        });
        let safe = preliminary;
        let feedback = if let Some(snapshot) = snapshot {
            array(&snapshot["feedback"])
        } else {
            self.db.feedback()?
        };
        let mut candidates = vec![];
        for day in &days {
            let da = iso(s::zoned(day, 0.0, &zone)?);
            let db = iso(s::zoned(day, 24.0, &zone)?);
            let day_busy: Vec<_> = busy
                .iter()
                .filter(|v| s::overlaps(&da, &db, v))
                .cloned()
                .collect();
            let day_events: Vec<_> = events
                .iter()
                .filter(|v| s::overlaps(&da, &db, v))
                .cloned()
                .collect();
            let c: Vec<_> = safe
                .iter()
                .filter(|v| {
                    epoch(&text(v, "start"))
                        .and_then(|t| s::date(t, &zone).ok())
                        .as_ref()
                        == Some(day)
                })
                .cloned()
                .collect();
            let input = json!({"day":day,"zone":zone,"hours":settings["hours"],"busy":day_busy,"events":day_events,"rules":rules,"learning":learning,"weekBusy":context_busy,"weekEvents":context_events,"candidates":c});
            let load = w::day(&input)?;
            let arranged = w::arrange(&input)?;
            candidates.extend(w::rank(&json!({"candidates":arranged,"zone":zone,"busy":day_busy,"feedback":feedback,"meetingDay":number(&w::types(&load),"meetings")>=60.0||number(&load,"meetings")>=2.0}))?);
        }
        candidates.truncate(limit);
        result["candidates"] = json!(candidates);
        result["travel"] = json!({"inPersonEventCount":travel["eventCount"],"maximumBufferMinutes":travel["maximumBufferMinutes"],"routeProvider":travel["routeProvider"],"adviceSource":travel["adviceSource"]});
        Ok(result)
    }
    async fn dates(&self, input: &Value) -> Result<Value> {
        let month = text(input, "month");
        ensure!(
            regex::Regex::new(r"^\d{4}-(0[1-9]|1[0-2])$")
                .expect("static regex")
                .is_match(&month),
            "Invalid month"
        );
        let mut snapshot = if self.demo {
            None
        } else {
            Some(self.preview_snapshot()?)
        };
        let settings = self.settings()?;
        let host = if let Some(snapshot) = &snapshot {
            snapshot["host"].clone()
        } else {
            self.host(&settings, None).await?
        };
        let zone = text(&host, "timezone");
        let viewer = if s::valid_zone(&text(input, "viewerTimeZone")) {
            text(input, "viewerTimeZone")
        } else {
            zone.clone()
        };
        let duration = input["durationMinutes"]
            .as_f64()
            .filter(|v| *v != 0.0)
            .unwrap_or(number(&settings, "durationMinutes"))
            .clamp(15.0, 180.0);
        let today = s::date(now(), &viewer)?;
        let max = s::shift(&today, 90)?;
        let mut days = vec![];
        for n in 1..=31 {
            let d = format!("{month}-{n:02}");
            if chrono::NaiveDate::parse_from_str(&d, "%Y-%m-%d").is_ok() && d >= today && d <= max {
                days.push(d)
            }
        }
        let mut result = json!({"month":month,"durationMinutes":duration,"viewerTimeZone":viewer,"dates":[],"recommendedDate":null});
        if days.is_empty() {
            return Ok(result);
        }
        if let Some(snapshot) = snapshot.as_mut() {
            let first = s::date(s::zoned(&days[0], 0.0, &viewer)?, &zone)?;
            let last = s::date(
                s::zoned(days.last().expect("nonempty"), 24.0, &viewer)? - 1,
                &zone,
            )?;
            let start = iso(s::zoned(&s::monday(&first)?, 0.0, &zone)?);
            let end = iso(s::zoned(&s::shift(&s::monday(&last)?, 12)?, 0.0, &zone)?);
            for field in ["busy", "events"] {
                snapshot[field] = json!(
                    array(&snapshot[field])
                        .into_iter()
                        .filter(|v| s::overlaps(&start, &end, v))
                        .collect::<Vec<_>>()
                );
            }
        }
        let rules = self.rules()?;
        let mut dates = vec![];
        let mut preferences = vec![];
        let mut first_availability = Value::Null;
        let mut availability_by_date = serde_json::Map::new();
        for day in days {
            let mut avail = self
                .availability(
                    &json!({"date": day, "durationMinutes": duration, "viewerTimeZone": viewer}),
                    None,
                    snapshot.as_ref(),
                    12,
                )
                .await?;
            if let Some(snapshot) = &snapshot {
                avail["snapshotAgeMs"] = snapshot["snapshotAgeMs"].clone();
                avail["previewMaxAgeMs"] = snapshot["previewMaxAgeMs"].clone();
            }
            if !array(&avail["candidates"]).is_empty() {
                // Recommendation uses only the first seven eligible dates.
                if preferences.len() < 7 {
                    let date = s::date(
                        epoch(&text(&avail["candidates"][0], "start"))
                            .ok_or_else(|| anyhow!("Invalid candidate"))?,
                        &zone,
                    )?;
                    let score = w::preference(
                        &json!({"day":date,"zone":zone,"hours":settings["hours"],"busy":snapshot.as_ref().map(|s|s["busy"].clone()).unwrap_or(json!([])),"events":snapshot.as_ref().map(|s|s["events"].clone()).unwrap_or(json!([])),"rules":rules}),
                    )?;
                    preferences.push((day.clone(), score));
                }
                if first_availability.is_null() {
                    first_availability = avail.clone();
                }
                availability_by_date.insert(day.clone(), avail);
                dates.push(day.clone());
            }
        }
        preferences.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        result["dates"] = json!(dates);
        result["recommendedDate"] = json!(preferences.first().map(|p| &p.0));
        result["partial"] = json!(false);
        result["firstDateAvailability"] = first_availability;
        result["availabilityByDate"] = Value::Object(availability_by_date);
        Ok(result)
    }
    async fn refresh_workload(&self) -> Result<Value> {
        if self.demo {
            return Ok(Value::Null);
        }
        let _guard = self.refresh.lock().await;
        let calendar = self.providers.calendar()?;
        let settings = self.settings()?;
        let rules = self.rules()?;
        let host = self.host(&settings, Some(&calendar)).await?;
        let zone = text(&host, "timezone");
        let today = s::date(now(), &zone)?;
        let start_day = s::shift(&today, -28)?;
        let (busy, events) = self
            .calendar_data(
                Some(&calendar),
                &iso(s::zoned(&start_day, 0.0, &zone)?),
                &iso(s::zoned(&today, 0.0, &zone)?),
            )
            .await?;
        let mut learned = w::learn(
            &json!({"startDay":start_day,"days":28,"zone":zone,"hours":settings["hours"],"busy":busy,"events":events,"rules":rules}),
        )?;
        let mut upcoming = vec![];
        let mut d = today.clone();
        while upcoming.len() < 5 {
            if s::weekday(&d)? % 6 != 0 {
                upcoming.push(d.clone())
            }
            d = s::shift(&d, 1)?
        }
        let (busy, events) = self
            .calendar_data(
                Some(&calendar),
                &iso(s::zoned(&upcoming[0], 0.0, &zone)?),
                &iso(s::zoned(upcoming.last().expect("nonempty"), 24.0, &zone)?),
            )
            .await?;
        let mut notice_days = vec![];
        for day in upcoming.iter().take(2) {
            let load = w::day(
                &json!({"day":day,"zone":zone,"hours":settings["hours"],"busy":busy,"events":events,"rules":rules}),
            )?;
            notice_days
                .push(json!({"busyMinutes":load["busyMinutes"],"typeMinutes":w::types(&load)}))
        }
        learned["bookingNotice"] = advisory::notice(
            &self.providers,
            settings["minimumBookingNoticeMinutes"]
                .as_i64()
                .unwrap_or(0),
            number(&learned, "baselineBusyMinutes"),
            &notice_days,
        )
        .await;
        let signal = w::alert(
            &json!({"days":upcoming,"zone":zone,"hours":settings["hours"],"busy":busy,"events":events,"rules":rules,"learning":learned}),
        )?;
        learned["weekAlert"] = if signal.is_null() {
            Value::Null
        } else {
            merge(
                &json!({"heavyDays":signal["heavyDays"],"busyMinutes":signal["busyMinutes"],"typicalBusyMinutes":signal["typicalBusyMinutes"]}),
                &advisory::week(&self.providers, &signal).await,
            )
        };
        learned["signature"] = json!(Self::signature(&settings, &rules, &zone));
        self.db.put("workloadLearning", &learned)?;
        Ok(learned)
    }
    async fn refresh_due(&self) -> Result<()> {
        if self.demo || self.db.accounts()?.is_empty() {
            return Ok(());
        }
        let settings = self.settings()?;
        let rules = self.rules()?;
        let saved = self.db.setting("workloadLearning", Value::Null)?;
        let zone = text(&settings, "timezone");
        if saved["signature"] == Self::signature(&settings, &rules, &zone)
            && workload_refresh_remaining(&saved, now()) > 0
        {
            return Ok(());
        }
        self.refresh_workload().await?;
        Ok(())
    }
}

impl App {
    fn set_cookie(
        &self,
        response: &mut Response<Body>,
        key: &str,
        value: &str,
        max_age: i64,
    ) -> Result<()> {
        let encoded: String = url::form_urlencoded::byte_serialize(value.as_bytes()).collect();
        let secure = if self.env("NODE_ENV") == "production" {
            "Secure; "
        } else {
            ""
        };
        let v =
            format!("{key}={encoded}; Path=/; HttpOnly; SameSite=Lax; {secure}Max-Age={max_age};");
        response.headers_mut().insert("Set-Cookie", v.parse()?);
        Ok(())
    }
    async fn asset(&self, path: &str) -> Result<Response<Body>> {
        let root = std::path::PathBuf::from(
            self.env
                .get("JEV_ASSET_ROOT")
                .cloned()
                .unwrap_or_else(|| "public".into()),
        )
        .canonicalize()?;
        let candidate = root.join(path);
        let clean = tokio::fs::canonicalize(candidate).await?;
        ensure!(clean.starts_with(&root) && clean.is_file(), "Forbidden");
        let body = tokio::fs::read(&clean).await?;
        let mime = match clean.extension().and_then(|x| x.to_str()).unwrap_or("") {
            "html" => "text/html; charset=utf-8",
            "js" => "text/javascript; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "svg" => "image/svg+xml",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            _ => "application/octet-stream",
        };
        Ok(Response::builder()
            .status(200)
            .header("Content-Type", mime)
            .header("Cache-Control", "no-store")
            .header("X-Content-Type-Options", "nosniff")
            .body(Body::from(body))?)
    }
    async fn route(
        &self,
        method: &Method,
        path: &str,
        query: &HashMap<String, String>,
        headers: &HeaderMap,
        input: &Value,
    ) -> Result<Response<Body>> {
        if self.demo && self.env("JEV_TEST_MODE") == "true" {
            match (method.as_str(), path) {
                ("GET", "/__test/provider-state") => {
                    let state = self
                        .demo_state
                        .lock()
                        .map_err(|_| anyhow!("Demo state unavailable"))?;
                    return Ok(response(
                        200,
                        json!({"mutationCount":state["mutationCount"],"creationCount":state["creationCount"]}),
                    ));
                }
                ("POST", "/__test/owner-change-pending") => {
                    let id = text(input, "requestId");
                    if self.db.journal(&id)?.is_null() {
                        return Ok(error(404, "Owner change not found."));
                    }
                    self.db.execute("UPDATE jaack_os_booking_changes SET state='pending',response=NULL,updated_at=? WHERE request_id=?",&[&iso(now()),&id])?;
                    return Ok(response(200, json!({"ok":true})));
                }
                ("POST", "/__test/external-change-after-read") => {
                    let record = self.db.by_source(&text(input, "sourceId"))?;
                    if !record["eventId"].is_string()
                        || epoch(&text(input, "start")).is_none()
                        || epoch(&text(input, "end")).is_none()
                    {
                        return Ok(error(400, "Invalid test event change."));
                    }
                    let mut state = self
                        .demo_state
                        .lock()
                        .map_err(|_| anyhow!("Demo state unavailable"))?;
                    state["external"][text(&record, "eventId")] = json!({"id":record["eventId"],"etag":uuid::Uuid::new_v4().to_string(),"start":{"dateTime":input["start"]},"end":{"dateTime":input["end"]}});
                    return Ok(response(200, json!({"ok":true})));
                }
                _ => (),
            }
        }
        if method == Method::GET && path == "/health" {
            return Ok(response(
                200,
                json!({"ok":true,"demo":self.demo,"calendar":if self.demo{"demo"}else{"google"},"previewReady":self.demo || self.preview_snapshot().is_ok()}),
            ));
        }
        if method == Method::GET && path == "/api/config" {
            let accounts = self.providers.list()?;
            return Ok(response(
                200,
                json!({"demo":self.demo,"googleConnected":!accounts.is_empty(),"googleAllCalendarsAuthorized":!accounts.is_empty()&&accounts.iter().all(|a|a["allCalendarsAuthorized"]==true),"hostTimezone":self.settings()?["timezone"]}),
            ));
        }
        if path == "/api/integrations/jaack-os/bookings"
            || path.starts_with("/api/integrations/jaack-os/bookings/")
            || path.starts_with("/api/integrations/jaack-os/booking-changes/")
        {
            if let Some(id) = path.strip_prefix("/api/integrations/jaack-os/booking-changes/")
                && uuid::Uuid::parse_str(id).is_ok()
            {
                if method != Method::GET {
                    return Ok(error(405, "Method not allowed"));
                }
                if !self.bearer(headers, false) {
                    return Ok(error(401, "Integration authentication required"));
                }
                let source = query.get("sourceId").cloned().unwrap_or_default();
                let request_hash = query.get("requestHash").cloned().unwrap_or_default();
                if !hex_id(&source)
                    || !hex_id(&request_hash)
                    || query.keys().any(|k| k != "sourceId" && k != "requestHash")
                {
                    return Ok(error(400, "Exact sourceId and requestHash are required."));
                }
                let journal = self.db.journal(id)?;
                if journal.is_null()
                    || journal["sourceId"] != source
                    || journal["requestHash"] != request_hash
                {
                    return Ok(error(404, "Booking change request not found."));
                }
                return Ok(response(
                    200,
                    json!({"requestId":id,"sourceId":source,"requestHash":request_hash,"state":journal["state"]}),
                ));
            }
            if let Some(id) = path
                .strip_prefix("/api/integrations/jaack-os/bookings/")
                .and_then(|p| p.strip_suffix("/change"))
                && hex_id(id)
            {
                if method != Method::POST {
                    return Ok(error(405, "Method not allowed"));
                }
                if !self.bearer(headers, true) {
                    return Ok(error(
                        401,
                        "Owner write integration authentication required",
                    ));
                }
                let _guard = self.operations.lock().await;
                let (code, v) = self
                    .owner_change(id, header(headers, "idempotency-key"), input)
                    .await?;
                return Ok(response(code, v));
            }
            if !self.bearer(headers, false) {
                return Ok(error(401, "Integration authentication required"));
            }
            if method != Method::GET {
                return Ok(error(405, "Method not allowed"));
            }
            if path == "/api/integrations/jaack-os/bookings" {
                let limit = query
                    .get("limit")
                    .map(|s| s.parse::<i64>())
                    .transpose()
                    .ok()
                    .flatten()
                    .unwrap_or(if query.contains_key("limit") { -1 } else { 50 });
                let offset = query
                    .get("offset")
                    .map(|s| s.parse::<i64>())
                    .transpose()
                    .ok()
                    .flatten()
                    .unwrap_or(if query.contains_key("offset") { -1 } else { 0 });
                if !(1..=100).contains(&limit) || !(0..=10000).contains(&offset) {
                    return Ok(error(400, "Invalid pagination"));
                }
                return Ok(response(200, self.db.list_owner(limit, offset)?));
            }
            let id = path
                .strip_prefix("/api/integrations/jaack-os/bookings/")
                .unwrap_or("");
            if !hex_id(id) {
                return Ok(error(404, "Booking not found"));
            }
            let record = self.db.by_source(id)?;
            return Ok(if record.is_null() {
                error(404, "Booking not found")
            } else {
                response(200, json!({"booking":Store::owner(&record)}))
            });
        }
        if path == "/api/google/accounts" {
            if !self.owner(headers)? {
                return Ok(error(401, "Owner sign-in required"));
            }
            if method == Method::GET {
                return Ok(response(
                    200,
                    json!({"accounts":self.providers.list()?,"demo":self.demo}),
                ));
            }
            if !self.mutation(headers)? {
                return Ok(error(
                    403,
                    "Open the private dashboard to manage Google accounts",
                ));
            }
            if method == Method::DELETE || method == Method::PUT {
                let _guard = self.operations.lock().await;
                if method == Method::DELETE {
                    self.db.disconnect(&text(input, "accountId"))?
                } else {
                    self.db.select(&text(input, "accountId"))?
                }
                return Ok(response(
                    200,
                    json!({"accounts":self.providers.list()?,"demo":self.demo}),
                ));
            }
        }
        if method == Method::POST && path == "/api/google/connect" {
            if !self.owner(headers)? {
                return Ok(error(401, "Owner sign-in required"));
            }
            if !self.mutation(headers)? {
                return Ok(error(
                    403,
                    "Open the private dashboard to manage Google accounts",
                ));
            }
            ensure!(!self.demo, "Demo mode has no Google OAuth");
            let id = text(input, "accountId");
            let flow = self.providers.begin_oauth(
                if id.is_empty() { "add" } else { "reconnect" },
                &id,
                &cookie(headers, "jev_session"),
                &self.base,
            )?;
            let mut r = response(200, json!({"url":flow["url"]}));
            self.set_cookie(&mut r, "jev_oauth", &text(&flow, "nonce"), 600)?;
            return Ok(r);
        }
        if [
            "/api/settings",
            "/api/workload",
            "/api/workload/refresh",
            "/api/address-search",
            "/api/timezone",
        ]
        .contains(&path)
            && !self.owner(headers)?
        {
            return Ok(error(401, "Owner sign-in required"));
        }
        if method == Method::GET && path == "/api/settings" {
            return Ok(response(200, self.settings()?));
        }
        if path == "/api/workload" && method == Method::GET {
            let calendars = if self.demo {
                vec![]
            } else {
                self.providers.workload_calendars().await?
            };
            let learned = self.db.setting("workloadLearning", Value::Null)?;
            return Ok(response(
                200,
                json!({"rules":self.rules()?,"calendars":calendars,"learning":if learned.is_null(){Value::Null}else{json!({"updatedAt":learned["updatedAt"],"baselineBusyMinutes":learned["baselineBusyMinutes"],"typicalProjectMinutes":learned["typicalProjectMinutes"],"bookingNotice":learned["bookingNotice"],"weekAlert":learned["weekAlert"]})}}),
            ));
        }
        if (path == "/api/workload" && method == Method::PUT)
            || (path == "/api/workload/refresh" && method == Method::POST)
        {
            if !self.mutation(headers)? {
                return Ok(error(
                    403,
                    "Open the private dashboard to manage Google accounts",
                ));
            }
            if method == Method::PUT {
                let rules = w::normalize(&input["rules"])?;
                self.db.put("projectRules", &json!(rules))?;
                return Ok(response(200, json!({"ok":true,"rules":rules})));
            }
            let learned = self.refresh_workload().await?;
            return Ok(response(
                200,
                json!({"ok":true,"updatedAt":learned["updatedAt"]}),
            ));
        }
        if method == Method::POST && path == "/api/address-search" {
            let suggestions=self.providers.address(&text(input,"input"),&text(input,"languageCode")).await?.iter().map(|s|json!({"text":s["text"],"mainText":s["mainText"],"secondaryText":s["secondaryText"],"selectionToken":crypto::address_encode(if self.env("APP_ENCRYPTION_KEY").is_empty(){"demo-local-only"}else{&self.secret},s)})).collect::<Vec<_>>();
            return Ok(response(
                200,
                json!({"suggestions":suggestions,"provider":"photon"}),
            ));
        }
        if method == Method::PUT && path == "/api/settings" {
            let current = self.settings()?;
            let location = text(input, "baseLocation")
                .trim()
                .chars()
                .take(300)
                .collect::<String>();
            let verified = if location.is_empty() {
                json!({"formattedAddress":"","provider":"","providerId":"","location":null})
            } else if !text(input, "baseLocationProviderId").is_empty()
                && input["baseLocationProviderId"] == current["baseLocationProviderId"]
                && location == text(&current, "baseLocation")
            {
                json!({"formattedAddress":current["baseLocation"],"provider":current["baseLocationProvider"],"providerId":current["baseLocationProviderId"],"location":current["baseLocationCoordinates"]})
            } else {
                crypto::address_decode(
                    if self.env("APP_ENCRYPTION_KEY").is_empty() {
                        "demo-local-only"
                    } else {
                        &self.secret
                    },
                    &text(input, "addressSelectionToken"),
                )?
            };
            ensure!(
                location == text(&verified, "formattedAddress"),
                "Choose an address from the search results"
            );
            let notice = if input.get("minimumBookingNoticeMinutes").is_some() {
                input["minimumBookingNoticeMinutes"].clone()
            } else {
                current["minimumBookingNoticeMinutes"].clone()
            };
            ensure!(
                notice.as_i64().is_some_and(|n| (0..=10080).contains(&n)),
                "Advance notice must be between 0 and 168 hours"
            );
            let n = |v: &Value, k: &str, f: f64| {
                let n = number(v, k);
                if n == 0.0 { f } else { n }
            };
            let settings = merge(
                &current,
                &json!({"hours":{"start":n(&input["hours"],"start",9.0).clamp(0.0,23.0),"end":n(&input["hours"],"end",18.0).clamp(1.0,24.0)},"durationMinutes":n(input,"durationMinutes",30.0).clamp(15.0,180.0),"baseLocation":verified["formattedAddress"],"baseLocationProvider":verified["provider"],"baseLocationProviderId":verified["providerId"],"baseLocationCoordinates":verified["location"],"minimumTravelBufferMinutes":n(input,"minimumTravelBufferMinutes",10.0).clamp(0.0,60.0),"minimumBookingNoticeMinutes":notice}),
            );
            ensure!(
                number(&settings["hours"], "end") > number(&settings["hours"], "start"),
                "End hour must be after start hour"
            );
            self.db.put("settings", &settings)?;
            return Ok(response(200, json!({"ok":true,"settings":settings})));
        }
        if method == Method::PUT && path == "/api/timezone" {
            let zone = text(input, "timezone");
            ensure!(s::valid_zone(&zone), "Invalid time zone");
            let updated = iso(now());
            let settings = merge(
                &self.settings()?,
                &json!({"timezone":zone,"timezoneSource":"owner-device","timezoneUpdatedAt":updated}),
            );
            self.db.put("settings", &settings)?;
            return Ok(response(
                200,
                json!({"ok":true,"timezone":zone,"source":"owner-device","updatedAt":updated}),
            ));
        }
        if method == Method::POST && path == "/api/availability" {
            return Ok(response(200, self.preview_availability(input).await?));
        }
        if method == Method::POST && path == "/api/available-dates" {
            return Ok(response(200, self.dates(input).await?));
        }
        if method == Method::POST
            && [
                "/api/booking/view",
                "/api/booking/slots",
                "/api/booking/change",
            ]
            .contains(&path)
        {
            let _guard = self.operations.lock().await;
            let record = self.management_record(&text(input, "token"))?;
            let (record, event) = self.load(record).await?;
            let result = if path == "/api/booking/view" {
                self.summary(&record)
            } else if path == "/api/booking/slots" {
                ensure!(
                    record["status"] == "confirmed",
                    "This meeting has been cancelled."
                );
                ensure!(
                    epoch(&text(&record, "start")).is_some_and(|t| t > now()),
                    "This meeting has already started. Please contact Jaack for changes."
                );
                ensure!(
                    record["pendingChange"].is_null(),
                    "Finish the pending change before choosing another time."
                );
                let duration = (epoch(&text(&record, "end")).unwrap_or(0)
                    - epoch(&text(&record, "start")).unwrap_or(0))
                    / 60_000;
                self.availability(
                    &merge(input, &json!({"durationMinutes":duration})),
                    None,
                    None,
                    12,
                )
                .await?
            } else {
                self.change(record, event, input).await?
            };
            return Ok(response(200, result));
        }
        if method == Method::POST && path == "/api/book" {
            let _guard = self.operations.lock().await;
            return match self.book(input).await {
                Ok((status, v)) => Ok(response(status, v)),
                Err(e) => {
                    let pending =
                        self.db.get(&text(input, "idempotencyKey"))?["status"] == "pending";
                    Ok(error(if pending { 503 } else { 400 }, &e.to_string()))
                }
            };
        }
        if method == Method::GET && path == "/auth/google" {
            ensure!(!self.demo, "Demo mode has no Google OAuth");
            let flow = self.providers.begin_oauth("login", "", "", &self.base)?;
            let mut r = redirect(&text(&flow, "url"))?;
            self.set_cookie(&mut r, "jev_oauth", &text(&flow, "nonce"), 600)?;
            return Ok(r);
        }
        if method == Method::GET && path == "/auth/google/callback" {
            let _guard = self.operations.lock().await;
            let result = self
                .providers
                .finish_oauth(
                    query.get("state").map(String::as_str).unwrap_or(""),
                    &cookie(headers, "jev_oauth"),
                    query.get("code").map(String::as_str).unwrap_or(""),
                    &cookie(headers, "jev_session"),
                    &self.base,
                )
                .await;
            return match result {
                Ok(login) => {
                    let mut r = redirect("/owner")?;
                    if login {
                        self.set_cookie(&mut r, "jev_session", &self.db.session()?, 28800)?;
                    }
                    Ok(r)
                }
                Err(e) => {
                    if !self.owner(headers)? {
                        return Err(e);
                    }
                    Ok(redirect(&format!(
                        "/owner?connectionError={}",
                        providers::encode(&e.to_string())
                    ))?)
                }
            };
        }
        if method == Method::GET {
            match path {
                "/owner" => {
                    if !self.owner(headers)? {
                        return redirect("/auth/google");
                    }
                    return self.asset("owner.html").await;
                }
                "/" => return self.asset("index.html").await,
                "/manage" => return self.asset("manage.html").await,
                "/privacy" => return self.asset("privacy.html").await,
                _ => (),
            };
            if let Some(asset) = path.strip_prefix("/assets/") {
                return self.asset(&format!("assets/{asset}")).await;
            }
        }
        Ok(error(404, "Not found"))
    }
}
fn hex_id(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn redirect(location: &str) -> Result<Response<Body>> {
    Ok(Response::builder()
        .status(302)
        .header("Location", location)
        .header("Cache-Control", "no-store")
        .body(Body::empty())?)
}
fn security(r: &mut Response<Body>) {
    for (k, v) in [
        (
            "Content-Security-Policy",
            "default-src 'self'; base-uri 'none'; connect-src 'self'; form-action 'self'; frame-ancestors 'none'; img-src 'self' data:; object-src 'none'; script-src 'self'; style-src 'self'",
        ),
        (
            "Permissions-Policy",
            "camera=(), microphone=(), geolocation=(), payment=()",
        ),
        ("Referrer-Policy", "strict-origin-when-cross-origin"),
        (
            "Strict-Transport-Security",
            "max-age=31536000; includeSubDomains",
        ),
        ("X-Content-Type-Options", "nosniff"),
        ("X-Frame-Options", "DENY"),
    ] {
        r.headers_mut().insert(
            axum::http::HeaderName::from_bytes(k.as_bytes()).expect("valid header"),
            v.parse().expect("valid value"),
        );
    }
}
fn allow(method: &Method, path: &str) -> bool {
    ((method == Method::GET || method == Method::HEAD)
        && (["/", "/manage", "/privacy", "/api/config", "/health"].contains(&path)
            || path.starts_with("/assets/")))
        || (method == Method::POST
            && [
                "/api/availability",
                "/api/available-dates",
                "/api/book",
                "/api/booking/view",
                "/api/booking/slots",
                "/api/booking/change",
            ]
            .contains(&path))
}
async fn private_handler(State(app): State<Arc<App>>, request: Request<Body>) -> Response<Body> {
    let (method, path, query, headers, body) = parts(request);
    let body = match to_bytes(body, 100_000).await {
        Ok(b) => b,
        Err(_) => return error(400, "Request too large"),
    };
    let input: Value = if body.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(_) => return error(400, "Invalid JSON"),
        }
    };
    let mut r = match app.route(&method, &path, &query, &headers, &input).await {
        Ok(r) => r,
        Err(e) => {
            if path == "/api/availability" || path == "/api/available-dates" {
                response(400, json!({"error":e.to_string(),"invalidatePreview":true}))
            } else {
                error(400, &e.to_string())
            }
        }
    };
    let changes_calendar = path == "/api/book"
        || path == "/api/booking/change"
        || (method == Method::POST && path.starts_with("/api/integrations/jaack-os/bookings/"));
    let changes_configuration =
        (method == Method::DELETE || method == Method::PUT || path == "/auth/google/callback")
            && r.status().is_success()
            || path == "/auth/google/callback" && r.status().is_redirection();
    if !app.demo && (changes_calendar || changes_configuration) {
        app.invalidate_preview();
    }
    r.headers_mut()
        .insert("X-Jev-Backend", "rust".parse().expect("valid header"));
    r
}
fn parts(request: Request<Body>) -> (Method, String, HashMap<String, String>, HeaderMap, Body) {
    let (parts, body) = request.into_parts();
    let query = url::form_urlencoded::parse(parts.uri.query().unwrap_or("").as_bytes())
        .into_owned()
        .collect();
    (
        parts.method,
        parts.uri.path().to_owned(),
        query,
        parts.headers,
        body,
    )
}
async fn public_handler(State(app): State<Arc<App>>, request: Request<Body>) -> Response<Body> {
    let (method, path, query, headers, body) = parts(request);
    let host = app
        .env
        .get("PUBLIC_HOST")
        .cloned()
        .unwrap_or_else(|| "book.jaack.me".into());
    let client = hash(header(&headers, "cf-connecting-ip"));
    let result = async {
        if header(&headers, "host").split(':').next().unwrap_or("") != host {
            return Ok(error(421, "Misdirected request"));
        }
        if (method == Method::GET || method == Method::HEAD)
            && ["/owner", "/owner/"].contains(&path.as_str())
            && !app.env("APP_BASE_URL").is_empty()
        {
            let target = url::Url::parse(&app.base)?.join("/owner")?;
            if target.origin().ascii_serialization() != format!("https://{host}") {
                return redirect(target.as_str());
            }
        }
        if !allow(&method, &path) {
            return Ok(error(404, "Not found"));
        }
        if method == Method::POST {
            if !header(&headers, "content-type")
                .to_lowercase()
                .starts_with("application/json")
            {
                return Ok(error(415, "JSON required"));
            }
            if !header(&headers, "origin").is_empty()
                && header(&headers, "origin") != format!("https://{host}")
            {
                return Ok(error(403, "Origin denied"));
            }
            if !header(&headers, "sec-fetch-site").is_empty()
                && header(&headers, "sec-fetch-site") != "same-origin"
            {
                return Ok(error(403, "Cross-site request denied"));
            }
            if header(&headers, "x-jev-booking") != "1" {
                return Ok(error(403, "Request verification failed"));
            }
            let (category, limit, window) = if path.starts_with("/api/booking/") {
                ("management", 60, 900_000)
            } else if path == "/api/book" {
                ("booking", 5, 3_600_000)
            } else {
                ("availability", 30, 900_000)
            };
            let retry = {
                let mut limits = app
                    .limits
                    .lock()
                    .map_err(|_| anyhow!("Rate limiter unavailable"))?;
                if limits.len() > 10000 {
                    limits.retain(|_, (end, _)| *end > now());
                }
                let entry = limits
                    .entry(format!("{category}:{client}"))
                    .or_insert((now() + window, 0));
                if entry.0 <= now() {
                    *entry = (now() + window, 0)
                }
                entry.1 += 1;
                if entry.1 > limit {
                    Some(((entry.0 - now() + 999) / 1000).max(1))
                } else {
                    None
                }
            };
            if let Some(retry) = retry {
                let mut r = error(429, "Too many requests. Please try again later.");
                r.headers_mut()
                    .insert("Retry-After", retry.to_string().parse()?);
                return Ok(r);
            }
        }
        let bytes = if method == Method::POST {
            match to_bytes(body, 100_000).await {
                Ok(b) => b,
                Err(_) => return Ok(error(413, "Request too large")),
            }
        } else {
            axum::body::Bytes::new()
        };
        if method == Method::POST {
            let input: Value = match serde_json::from_slice(&bytes) {
                Ok(v) => v,
                Err(_) => return Ok(error(400, "Invalid JSON")),
            };
            if !input.is_object() {
                return Ok(error(400, "Invalid request"));
            }
            if path == "/api/book" {
                let email = text(&input, "guestEmail").trim().to_owned();
                if text(&input, "guestName").trim().is_empty()
                    || !regex::Regex::new(r"^[^\s@]+@[^\s@]+\.[^\s@]+$")
                        .expect("static regex")
                        .is_match(&email)
                {
                    return Ok(error(400, "A valid name and email are required"));
                }
            }
        }
        let upstream = app
            .env
            .get("INTERNAL_ORIGIN")
            .cloned()
            .unwrap_or_else(|| "http://127.0.0.1:3030".into());
        let mut url = url::Url::parse(&format!("{upstream}{path}"))?;
        for (k, v) in &query {
            url.query_pairs_mut().append_pair(k, v);
        }
        let mut req = app
            .providers
            .client
            .request(method.clone(), url)
            .timeout(Duration::from_secs(30));
        if method == Method::POST {
            req = req.header("Content-Type", "application/json").body(bytes)
        }
        let upstream = req.send().await?;
        let code = upstream.status().as_u16();
        let content = upstream.headers().get("Content-Type").cloned();
        let bytes = if method == Method::HEAD {
            axum::body::Bytes::new()
        } else {
            upstream.bytes().await?
        };
        let mut r = Response::builder()
            .status(code)
            .header(
                "Cache-Control",
                if path.starts_with("/assets/") {
                    "public, max-age=300"
                } else {
                    "no-store"
                },
            )
            .body(Body::from(bytes))?;
        if let Some(v) = content {
            r.headers_mut().insert("Content-Type", v);
        }
        Ok::<_, anyhow::Error>(r)
    }
    .await;
    let mut result =
        result.unwrap_or_else(|_| error(502, "Booking service temporarily unavailable"));
    security(&mut result);
    result
        .headers_mut()
        .insert("X-Jev-Backend", "rust".parse().expect("valid header"));
    println!(
        "{}",
        json!({"at":iso(now()),"method":method.as_str(),"path":path,"status":result.status().as_u16(),"client":client.get(..16).unwrap_or("")})
    );
    result
}
async fn shutdown() {
    let ctrl = tokio::signal::ctrl_c();
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {_=ctrl=>(),_=term=>()}
}
pub async fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("--private");
    if mode == "--evaluate" {
        return evaluate();
    }
    let env: HashMap<_, _> = std::env::vars().collect();
    let public = mode == "--public";
    ensure!(
        ["--public", "--private", "--backup"].contains(&mode),
        "Use --private, --public, --evaluate or --backup"
    );
    let port = env
        .get(if public { "PUBLIC_PORT" } else { "PORT" })
        .map(|v| v.parse::<u16>())
        .transpose()?
        .unwrap_or(if public { 3031 } else { 3030 });
    let live = env.get("JEV_ALLOW_LIVE_STATE").is_some_and(|v| v == "1");
    ensure!(
        live || ![3030, 3031].contains(&port) || mode == "--backup",
        "Production ports require JEV_ALLOW_LIVE_STATE=1; use an isolated port for testing"
    );
    let demo = env.get("DEMO_MODE").is_none_or(|v| v != "false");
    let secret = env
        .get("APP_ENCRYPTION_KEY")
        .cloned()
        .unwrap_or_else(|| crypto::random(32));
    ensure!(
        public || demo || env.get("APP_ENCRYPTION_KEY").is_some_and(|s| !s.is_empty()),
        "APP_ENCRYPTION_KEY is required when DEMO_MODE=false"
    );
    let path = if public {
        ":memory:".into()
    } else {
        env.get("DATABASE_PATH")
            .cloned()
            .unwrap_or_else(|| "./data/jev-calendar.sqlite".into())
    };
    if !live && let Ok(p) = std::path::Path::new(&path).canonicalize() {
        ensure!(
            !p.starts_with("/Users/jaack/clawd/apps/jev-calendar/data"),
            "Live state requires JEV_ALLOW_LIVE_STATE=1"
        )
    }
    let db = Arc::new(Store::open(&path, &secret)?);
    if mode == "--backup" {
        db.backup(
            args.get(2)
                .ok_or_else(|| anyhow!("Supply backup destination"))?,
        )?;
        println!("Backup complete");
        return Ok(());
    }
    let providers = Providers::new(db.clone(), env.clone())?;
    let base = env
        .get("APP_BASE_URL")
        .cloned()
        .unwrap_or_else(|| format!("http://localhost:{port}"));
    let origin = if demo {
        url::Url::parse(&base)?.origin().ascii_serialization()
    } else {
        format!(
            "https://{}",
            env.get("PUBLIC_HOST")
                .map(String::as_str)
                .unwrap_or("book.jaack.me")
        )
    };
    let state = json!({"events":{},"external":{},"mutationCount":0,"creationCount":0,"ambiguousReads":0,"failAfterCancel":demo&&env.get("JEV_TEST_AMBIGUOUS_CANCEL").is_some_and(|s|s=="true"),"failBeforeReschedule":demo&&env.get("JEV_TEST_FAIL_BEFORE_RESCHEDULE").is_some_and(|s|s=="true"),"ambiguousCreate":demo&&env.get("JEV_TEST_AMBIGUOUS_CREATE").is_some_and(|s|s=="true")});
    let app = Arc::new(App {
        db,
        providers,
        env,
        demo,
        secret,
        base: base.clone(),
        origin,
        operations: tokio::sync::Mutex::new(()),
        refresh: tokio::sync::Mutex::new(()),
        previews: Mutex::new(PreviewState::default()),
        preview_changed: tokio::sync::Notify::new(),
        demo_state: Mutex::new(state),
        limits: Mutex::new(HashMap::new()),
    });
    if !public && !demo {
        app.providers.migrate().await?;
        let started = Instant::now();
        if let Err(error) = app.refresh_preview().await {
            eprintln!("Initial calendar snapshot unavailable: {error}");
        } else {
            eprintln!(
                "Initial calendar snapshot ready in {} ms",
                started.elapsed().as_millis()
            );
        }
    }
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    let router = if public {
        Router::new()
            .fallback(any(public_handler))
            .with_state(app.clone())
    } else {
        Router::new()
            .fallback(any(private_handler))
            .with_state(app.clone())
    };
    let background = if !public && !demo {
        let app = app.clone();
        Some(tokio::spawn(async move {
            loop {
                if app.refresh_due().await.is_err() {
                    eprintln!("Background workload refresh failed");
                }
                let saved = app
                    .db
                    .setting("workloadLearning", Value::Null)
                    .unwrap_or(Value::Null);
                let wait = workload_refresh_remaining(&saved, now()).max(60_000);
                tokio::time::sleep(Duration::from_millis(wait as u64)).await;
            }
        }))
    } else {
        None
    };
    let calendar_background = if !public && !demo {
        let app = app.clone();
        Some(tokio::spawn(async move {
            let interval =
                if app.env("JEV_TEST_MODE") == "true" && app.env("JEV_ALLOW_LIVE_STATE") != "1" {
                    app.env("JEV_TEST_CALENDAR_REFRESH_MS")
                        .parse::<u64>()
                        .unwrap_or(CALENDAR_REFRESH_MS)
                        .max(50)
                } else {
                    CALENDAR_REFRESH_MS
                };
            let duration = Duration::from_millis(interval);
            let mut timer =
                tokio::time::interval_at(tokio::time::Instant::now() + duration, duration);
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = timer.tick() => {},
                    _ = app.preview_changed.notified() => {},
                }
                if let Err(error) = app.refresh_preview().await {
                    eprintln!("Background calendar snapshot refresh failed: {error}");
                }
            }
        }))
    } else {
        None
    };
    println!(
        "Jev Calendar Rust listening at {base} ({})",
        if demo { "demo" } else { "Google" }
    );
    let result = axum::serve(listener, router)
        .with_graceful_shutdown(shutdown())
        .await;
    if let Some(task) = background {
        task.abort();
    }
    if let Some(task) = calendar_background {
        task.abort();
    }
    result?;
    Ok(())
}
fn evaluate() -> Result<()> {
    use std::io::{BufRead, Write};
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let v: Value = serde_json::from_str(&line?)?;
        let input = &v["input"];
        let result = match text(&v, "operation").as_str() {
            "generateCandidates" => json!(s::candidates(input)?),
            "hardConstraint" => json!(s::constraint(
                &text(input, "start"),
                &text(input, "end"),
                &array(&input["busy"]),
                number(input, "bufferMinutes")
            )),
            "dayWorkload" => w::day(input)?,
            "arrangeWorkloadSlots" => json!(w::arrange(input)?),
            "rankCandidateSlots" => json!(w::rank(input)?),
            "weeklyDatePreference" => json!(w::preference(input)?),
            "weeklyWorkloadAlert" => w::alert(input)?,
            "learnWorkload" => {
                let mut result = w::learn(input)?;
                result
                    .as_object_mut()
                    .expect("learning object")
                    .remove("updatedAt");
                result
            }
            "normalizeProjectRules" => json!(w::normalize(input)?),
            "zonedEpoch" => json!(s::zoned(
                &text(input, "day"),
                number(input, "hour"),
                &text(input, "zone")
            )?),
            "dateKeyInZone" => json!(s::date(
                input["epoch"]
                    .as_i64()
                    .ok_or_else(|| anyhow!("Epoch required"))?,
                &text(input, "zone")
            )?),
            "decrypt" => json!(
                crypto::SecretBox::new(&text(input, "secret"))
                    .decrypt(&text(input, "ciphertext"))?
            ),
            "encrypt" => json!(
                crypto::SecretBox::new(&text(input, "secret"))
                    .encrypt(&text(input, "plaintext"))?
            ),
            "managementToken" => json!(crypto::management_token(
                &text(input, "secret"),
                &text(input, "key")
            )),
            "managementKey" => json!(crypto::management_key(
                &text(input, "secret"),
                &text(input, "token")
            )?),
            "addressDecode" => {
                crypto::address_decode(&text(input, "secret"), &text(input, "token"))?
            }
            _ => return Err(anyhow!("Unknown evaluation operation")),
        };
        writeln!(out, "{result}")?;
    }
    Ok(())
}

#[cfg(test)]
mod background_tests {
    use super::*;

    #[test]
    fn refresh_deadline_survives_midnight_and_restarts() {
        let updated = epoch("2030-10-07T23:00:00Z").unwrap();
        let saved = json!({"updatedAt": iso(updated)});
        assert_eq!(
            workload_refresh_remaining(&saved, updated),
            WORKLOAD_REFRESH_MS
        );
        assert_eq!(
            workload_refresh_remaining(&saved, updated + 2 * 60 * 60 * 1000),
            4 * 60 * 60 * 1000
        );
        assert_eq!(
            workload_refresh_remaining(&saved, updated + WORKLOAD_REFRESH_MS - 1),
            1
        );
        assert_eq!(
            workload_refresh_remaining(&saved, updated + WORKLOAD_REFRESH_MS),
            0
        );
        assert_eq!(
            workload_refresh_remaining(&saved, updated + WORKLOAD_REFRESH_MS + 1),
            0
        );
        assert_eq!(workload_refresh_remaining(&saved, updated - 1), 0);
        assert_eq!(workload_refresh_remaining(&Value::Null, updated), 0);
        assert_eq!(
            workload_refresh_remaining(&json!({"updatedAt": "invalid"}), updated),
            0
        );
    }
}
