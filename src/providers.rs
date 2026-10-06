use crate::{array, crypto, epoch, hash, now, number, store::Store, text};
use anyhow::{Result, anyhow, ensure};
use futures_util::{StreamExt, TryStreamExt};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};
pub const MODEL: &str = "jev-1.13.0";
pub const SCOPES: [&str; 5] = [
    "calendar.events",
    "calendar.freebusy",
    "calendar.events.freebusy",
    "calendar.calendarlist.readonly",
    "calendar.settings.readonly",
];
#[derive(Debug)]
pub struct HttpError(pub u16);
impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Google Calendar failed ({})", self.0)
    }
}
impl std::error::Error for HttpError {}
pub fn status(e: &anyhow::Error) -> Option<u16> {
    e.downcast_ref::<HttpError>().map(|e| e.0)
}
pub fn authorized(token: &Value) -> bool {
    let scope = text(token, "scope");
    let has = |s: &str| {
        scope
            .split_whitespace()
            .any(|x| x == format!("https://www.googleapis.com/auth/{s}"))
    };
    has("calendar")
        || (has("calendar.events")
            && (has("calendar.calendarlist.readonly") || has("calendar.calendarlist"))
            && has("calendar.events.freebusy")
            && has("calendar.settings.readonly"))
}
pub struct Providers {
    pub db: Arc<Store>,
    pub client: reqwest::Client,
    pub env: HashMap<String, String>,
    refresh: tokio::sync::Mutex<()>,
    pub errors: Mutex<HashSet<String>>,
    flows: Mutex<HashMap<String, Value>>,
}
pub struct Calendar {
    pub accounts: Vec<Value>,
    pub destination: String,
    lists: tokio::sync::Mutex<HashMap<String, Vec<Value>>>,
}
impl Providers {
    pub fn new(db: Arc<Store>, env: HashMap<String, String>) -> Result<Self> {
        if let Some(origin) = env.get("JEV_TEST_PROVIDER_ORIGIN") {
            let url = url::Url::parse(origin)?;
            ensure!(
                env.get("JEV_TEST_MODE").is_some_and(|v| v == "true")
                    && env.get("JEV_ALLOW_LIVE_STATE").is_none_or(|v| v != "1")
                    && url.scheme() == "http"
                    && url.host_str() == Some("127.0.0.1")
                    && url.username().is_empty()
                    && url.password().is_none(),
                "Fake providers require isolated test mode and a loopback origin"
            );
        }
        Ok(Self {
            db,
            env,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            refresh: tokio::sync::Mutex::new(()),
            errors: Mutex::new(HashSet::new()),
            flows: Mutex::new(HashMap::new()),
        })
    }
    pub fn env(&self, k: &str) -> String {
        self.env.get(k).cloned().unwrap_or_default()
    }
    fn endpoint(&self, real: &str, fixture: &str) -> String {
        self.env
            .get("JEV_TEST_PROVIDER_ORIGIN")
            .map(|base| format!("{}{fixture}", base.trim_end_matches('/')))
            .unwrap_or_else(|| real.to_owned())
    }
    pub fn calendar(&self) -> Result<Calendar> {
        let accounts = self.db.accounts()?;
        ensure!(!accounts.is_empty(), "Google Calendar is not connected");
        let destination = self
            .db
            .setting("googleBookingAccountId", Value::Null)?
            .as_str()
            .unwrap_or("")
            .to_owned();
        ensure!(
            accounts.iter().any(|a| a["id"] == destination),
            "Choose an account for new bookings in the private dashboard"
        );
        Ok(Calendar {
            accounts,
            destination,
            lists: tokio::sync::Mutex::new(HashMap::new()),
        })
    }
    pub fn list(&self) -> Result<Vec<Value>> {
        let destination = self.db.setting("googleBookingAccountId", Value::Null)?;
        let errors = self
            .errors
            .lock()
            .map_err(|_| anyhow!("Provider state unavailable"))?;
        self.db.accounts()?.iter().map(|a|{let id=text(a,"id");let account=self.db.account(&id)?;Ok(json!({"id":id,"email":id,"receivesBookings":destination==id,"needsReconnect":errors.contains(&id),"allCalendarsAuthorized":authorized(&account["token"])}))}).collect()
    }
    async fn refresh_token(&self, token: &Value) -> Result<Value> {
        if token["access_token"].is_string()
            && (now() as f64) < number(token, "expires_at") - 60_000.0
        {
            return Ok(token.clone());
        }
        ensure!(
            !text(token, "refresh_token").is_empty(),
            "Reconnect Google in the private dashboard."
        );
        let response = self
            .client
            .post(self.endpoint("https://oauth2.googleapis.com/token", "/oauth/token"))
            .form(&[
                ("client_id", self.env("GOOGLE_CLIENT_ID")),
                ("client_secret", self.env("GOOGLE_CLIENT_SECRET")),
                ("refresh_token", text(token, "refresh_token")),
                ("grant_type", "refresh_token".into()),
            ])
            .send()
            .await?;
        ensure!(
            response.status().is_success(),
            "A Google connection needs attention. Reconnect it in the private dashboard."
        );
        let update: Value = response.json().await?;
        let mut result = token.clone();
        for (k, v) in update
            .as_object()
            .ok_or_else(|| anyhow!("Invalid Google token"))?
        {
            result[k] = v.clone()
        }
        result["expires_at"] =
            json!(now() + (update["expires_in"].as_i64().unwrap_or(3600) * 1000));
        Ok(result)
    }
    async fn access(&self, id: &str, version: &str) -> Result<String> {
        let current = self.db.account(id)?;
        ensure!(
            !current.is_null() && current["version"] == version,
            "Google connections changed. Please try again."
        );
        if current["token"]["access_token"].is_string()
            && (now() as f64) < number(&current["token"], "expires_at") - 60_000.0
        {
            return Ok(text(&current["token"], "access_token"));
        }
        let _guard = self.refresh.lock().await;
        let account = self.db.account(id)?;
        ensure!(
            !account.is_null() && account["version"] == version,
            "Google connections changed. Please try again."
        );
        let refreshed = self.refresh_token(&account["token"]).await;
        match refreshed {
            Ok(token) => {
                ensure!(
                    self.db.refresh(id, version, &token)?,
                    "Google connections changed. Please try again."
                );
                self.errors
                    .lock()
                    .map_err(|_| anyhow!("Provider state unavailable"))?
                    .remove(id);
                Ok(text(&token, "access_token"))
            }
            Err(e) => {
                self.errors
                    .lock()
                    .map_err(|_| anyhow!("Provider state unavailable"))?
                    .insert(id.to_owned());
                Err(e)
            }
        }
    }
    async fn request_token(
        &self,
        token: &str,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
        etag: Option<&str>,
    ) -> Result<Value> {
        let mut request = self
            .client
            .request(
                method,
                format!(
                    "{}{path}",
                    self.endpoint("https://www.googleapis.com/calendar/v3", "/calendar/v3")
                ),
            )
            .bearer_auth(token)
            .header("Content-Type", "application/json");
        if let Some(b) = body {
            request = request.json(b)
        }
        if let Some(e) = etag {
            request = request.header("If-Match", e)
        }
        let r = request.send().await?;
        let status = r.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(HttpError(status).into());
        }
        if status == 204 {
            return Ok(Value::Null);
        }
        Ok(r.json().await?)
    }
    pub async fn request(
        &self,
        account: &Value,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
        etag: Option<&str>,
    ) -> Result<Value> {
        let id = text(account, "id");
        let token = self.access(&id, &text(account, "version")).await?;
        let result = self.request_token(&token, method, path, body, etag).await;
        if result
            .as_ref()
            .err()
            .and_then(status)
            .is_some_and(|s| s == 401 || s == 403)
        {
            self.errors
                .lock()
                .map_err(|_| anyhow!("Provider state unavailable"))?
                .insert(id);
        }
        result
    }
    pub async fn calendars(&self, calendar: &Calendar, account: &Value) -> Result<Vec<Value>> {
        let id = text(account, "id");
        let mut lists = calendar.lists.lock().await;
        if let Some(v) = lists.get(&id) {
            return Ok(v.clone());
        }
        let mut result = Vec::<Value>::new();
        let mut page = String::new();
        loop {
            let mut url = url::Url::parse("https://local.invalid/users/me/calendarList")?;
            url.query_pairs_mut().extend_pairs([
                ("maxResults", "250"),
                ("showHidden", "true"),
                ("showDeleted", "false"),
            ]);
            if !page.is_empty() {
                url.query_pairs_mut().append_pair("pageToken", &page);
            }
            let v = self
                .request(
                    account,
                    reqwest::Method::GET,
                    &format!("{}?{}", url.path(), url.query().unwrap_or("")),
                    None,
                    None,
                )
                .await?;
            for item in array(&v["items"]) {
                if item["deleted"] != true && item["id"].is_string() {
                    if let Some(i) = result.iter().position(|r| r["id"] == item["id"]) {
                        result[i] = item
                    } else {
                        result.push(item)
                    }
                }
            }
            page = text(&v, "nextPageToken");
            if page.is_empty() {
                break;
            }
        }
        ensure!(
            !result.is_empty(),
            "No Google calendars are available. Reconnect Google Calendar."
        );
        lists.insert(id, result.clone());
        Ok(result)
    }
    pub async fn free_busy(
        &self,
        calendar: &Calendar,
        start: &str,
        end: &str,
    ) -> Result<Vec<Value>> {
        let mut result = vec![];
        for account in &calendar.accounts {
            let calendars = self.calendars(calendar, account).await?;
            for batch in calendars.chunks(50) {
                let items: Vec<_> = batch.iter().map(|v| json!({"id":v["id"]})).collect();
                let v=self.request(account,reqwest::Method::POST,"/freeBusy",Some(&json!({"timeMin":start,"timeMax":end,"calendarExpansionMax":50,"items":items})),None).await?;
                for c in batch {
                    let entry = &v["calendars"][text(c, "id")];
                    ensure!(
                        entry.is_object()
                            && array(&entry["errors"]).is_empty()
                            && entry["busy"].is_array(),
                        "Unable to check every Google calendar. Please try again later."
                    );
                    for busy in array(&entry["busy"]) {
                        result.push(json!({"start":busy["start"],"end":busy["end"]}))
                    }
                }
            }
        }
        Ok(result)
    }
    pub async fn events(
        &self,
        calendar: &Calendar,
        start: &str,
        end: &str,
        workload: bool,
    ) -> Result<Vec<Value>> {
        let mut events = Vec::<Value>::new();
        let mut keys = HashMap::new();
        for account in &calendar.accounts {
            let calendars = self.calendars(calendar, account).await?;
            let calendars = calendars
                .into_iter()
                .filter(|c| c["accessRole"] != "freeBusyReader");
            let reads = futures_util::stream::iter(calendars)
                .map(|c| async move {
                    let mut result = Vec::new();
                    let mut page = String::new();
                    loop {
                        let path = format!("/calendars/{}/events", encode(&text(&c, "id")));
                        let mut url = url::Url::parse(&format!("https://local.invalid{path}"))?;
                        url.query_pairs_mut().extend_pairs([
                            ("timeMin", start),
                            ("timeMax", end),
                            ("singleEvents", "true"),
                            ("orderBy", "startTime"),
                            ("showDeleted", "false"),
                            ("maxResults", "2500"),
                        ]);
                        if !page.is_empty() {
                            url.query_pairs_mut().append_pair("pageToken", &page);
                        }
                        let v = self
                            .request(
                                account,
                                reqwest::Method::GET,
                                &format!("{path}?{}", url.query().unwrap_or("")),
                                None,
                                None,
                            )
                            .await?;
                        for e in array(&v["items"]) {
                            if e["status"] == "cancelled"
                                || e["transparency"] == "transparent"
                                || array(&e["attendees"])
                                    .iter()
                                    .any(|a| a["self"] == true && a["responseStatus"] == "declined")
                                || !e["start"]["dateTime"].is_string()
                                || !e["end"]["dateTime"].is_string()
                            {
                                continue;
                            }
                            let location = text(&e, "location").trim().to_owned();
                            let mut event = json!({"start":e["start"]["dateTime"],"end":e["end"]["dateTime"],"location":location,"inPerson":physical(&location)});
                            if workload {
                                event["summary"] =
                                    json!(text(&e, "summary").chars().take(500).collect::<String>());
                                event["calendarId"] = c["id"].clone();
                                event["calendarName"] =
                                    json!(text(&c, "summary").chars().take(200).collect::<String>());
                            }
                            result.push(event);
                        }
                        page = text(&v, "nextPageToken");
                        if page.is_empty() {
                            break;
                        }
                    }
                    Ok::<_, anyhow::Error>(result)
                })
                .buffered(4)
                .try_collect::<Vec<_>>()
                .await?;
            for event in reads.into_iter().flatten() {
                let key = json!([
                    epoch(&text(&event, "start")),
                    epoch(&text(&event, "end")),
                    event["location"],
                    event["inPerson"],
                    event["calendarId"],
                    event["summary"]
                ])
                .to_string();
                if let Some(i) = keys.get(&key) {
                    events[*i] = event;
                } else {
                    keys.insert(key, events.len());
                    events.push(event);
                }
            }
        }
        Ok(events)
    }
    pub async fn workload_calendars(&self) -> Result<Vec<Value>> {
        let calendar = self.calendar()?;
        let mut out = Vec::<Value>::new();
        for account in &calendar.accounts {
            for c in self.calendars(&calendar, account).await? {
                if c["accessRole"] != "freeBusyReader" {
                    let item = json!({"id":c["id"],"name":c["summary"].as_str().unwrap_or(c["id"].as_str().unwrap_or("")),"primary":c["primary"]==true});
                    if let Some(i) = out.iter().position(|v| v["id"] == item["id"]) {
                        out[i] = item
                    } else {
                        out.push(item)
                    }
                }
            }
        }
        Ok(out)
    }
    pub async fn timezone(&self, calendar: &Calendar) -> Result<String> {
        let account = calendar
            .accounts
            .iter()
            .find(|a| a["id"] == calendar.destination)
            .ok_or_else(|| anyhow!("Google Calendar is not connected"))?;
        Ok(text(
            &self
                .request(
                    account,
                    reqwest::Method::GET,
                    "/users/me/settings/timezone",
                    None,
                    None,
                )
                .await?,
            "value",
        ))
    }
    pub fn account_for(&self, id: &str) -> Result<Value> {
        let a = self.db.account(id)?;
        ensure!(
            !a.is_null(),
            "The calendar for this meeting is disconnected. Please contact Jaack."
        );
        Ok(a)
    }
    pub async fn get_event(&self, id: &str, event: &str) -> Result<Value> {
        match self
            .request(
                &self.account_for(id)?,
                reqwest::Method::GET,
                &format!("/calendars/primary/events/{}", encode(event)),
                None,
                None,
            )
            .await
        {
            Err(e) if status(&e).is_some_and(|s| s == 404 || s == 410) => Ok(Value::Null),
            r => r,
        }
    }
    pub async fn create(&self, v: &Value) -> Result<Value> {
        let links = &v["managementLinks"];
        let mut body = json!({"id":text(v,"providerEventId"),"summary":v["meetingTitle"],"description":format!("Manage your meeting with Jaack:\n\nReschedule: {}\n\nCancel: {}\n\nKeep these links private. Changes require confirmation.",text(links,"reschedule"),text(links,"cancel")),"start":{"dateTime":v["start"]},"end":{"dateTime":v["end"]},"attendees":[{"email":v["guestEmail"],"displayName":v["guestName"]}],"conferenceData":{"createRequest":{"requestId":text(v,"providerEventId"),"conferenceSolutionKey":{"type":"hangoutsMeet"}}}});
        if text(v, "guestEmail").is_empty() {
            body["attendees"] = json!([]);
        }
        let event = self
            .request(
                &self.account_for(&text(v, "googleAccountId"))?,
                reqwest::Method::POST,
                "/calendars/primary/events?sendUpdates=all&conferenceDataVersion=1",
                Some(&body),
                None,
            )
            .await?;
        Ok(event)
    }
    pub async fn mutate(&self, record: &Value, change: &Value) -> Result<()> {
        let event = text(record, "eventId");
        let account = self.account_for(&text(record, "googleAccountId"))?;
        let etag = text(change, "etag");
        ensure!(
            !etag.is_empty(),
            "Google did not return an event version. Please try again."
        );
        let path = format!(
            "/calendars/primary/events/{}?sendUpdates=all{}",
            encode(&event),
            if change["action"] == "cancel" {
                ""
            } else {
                "&conferenceDataVersion=1"
            }
        );
        let body = json!({"start":{"dateTime":change["start"]},"end":{"dateTime":change["end"]}});
        self.request(
            &account,
            if change["action"] == "cancel" {
                reqwest::Method::DELETE
            } else {
                reqwest::Method::PATCH
            },
            &path,
            if change["action"] == "cancel" {
                None
            } else {
                Some(&body)
            },
            Some(&etag),
        )
        .await?;
        Ok(())
    }
    async fn identity(&self, token: &Value) -> Result<String> {
        let mut page = String::new();
        loop {
            let path = format!(
                "/users/me/calendarList?maxResults=250&showHidden=true&showDeleted=false{}",
                if page.is_empty() {
                    String::new()
                } else {
                    format!("&pageToken={}", encode(&page))
                }
            );
            let v = self
                .request_token(
                    &text(token, "access_token"),
                    reqwest::Method::GET,
                    &path,
                    None,
                    None,
                )
                .await?;
            if let Some(p) = array(&v["items"])
                .iter()
                .find(|p| p["primary"] == true && p["id"].is_string())
            {
                return Ok(text(p, "id"));
            }
            page = text(&v, "nextPageToken");
            ensure!(
                !page.is_empty(),
                "Google did not identify a primary calendar. No account was changed."
            );
        }
    }
    pub async fn migrate(&self) -> Result<()> {
        let row = self
            .db
            .query("SELECT value FROM tokens WHERE provider='google'", &[])?;
        if let Some(v) = row.first() {
            let token: Value = serde_json::from_str(
                &crypto::SecretBox::new(&self.env("APP_ENCRYPTION_KEY"))
                    .decrypt(&text(v, "value"))?,
            )?;
            let token = self.refresh_token(&token).await?;
            let id = self.identity(&token).await?;
            if self.db.account(&id)?.is_null() {
                self.db.connect(&id, &token)?;
            }
            self.db.with(|c| {
                c.execute_batch("BEGIN IMMEDIATE")?;
                let work = || -> Result<()> {
                    c.execute(
                        "UPDATE bookings SET google_account_id=? WHERE google_account_id IS NULL",
                        [&id],
                    )?;
                    c.execute("DELETE FROM tokens WHERE provider='google'", [])?;
                    Ok(())
                };
                match work() {
                    Ok(()) => c.execute_batch("COMMIT")?,
                    Err(e) => {
                        c.execute_batch("ROLLBACK")?;
                        return Err(e);
                    }
                }
                Ok(())
            })?;
        }
        Ok(())
    }
    pub fn begin_oauth(&self, intent: &str, id: &str, session: &str, base: &str) -> Result<Value> {
        ensure!(
            !self.env("GOOGLE_CLIENT_ID").is_empty()
                && !self.env("GOOGLE_CLIENT_SECRET").is_empty(),
            "Google OAuth is not configured"
        );
        ensure!(
            ["login", "add", "reconnect"].contains(&intent),
            "Invalid connection action"
        );
        if intent != "login" {
            ensure!(self.db.valid_session(session)?, "Owner sign-in required")
        }
        if intent == "reconnect" {
            ensure!(
                !self.db.account(id)?.is_null(),
                "Google account is not connected"
            )
        }
        let mut flows = self
            .flows
            .lock()
            .map_err(|_| anyhow!("OAuth state unavailable"))?;
        flows.retain(|_, f| number(f, "expires") >= now() as f64);
        ensure!(
            flows.len() < 100,
            "Too many connection attempts. Please try again later."
        );
        let nonce = crypto::random(32);
        flows.insert(nonce.clone(),json!({"intent":intent,"accountId":id,"sessionHash":hash(session),"expires":now()+600_000}));
        let mut u = url::Url::parse("https://accounts.google.com/o/oauth2/v2/auth")?;
        u.query_pairs_mut().extend_pairs([
            ("client_id", self.env("GOOGLE_CLIENT_ID")),
            ("redirect_uri", format!("{base}/auth/google/callback")),
            ("response_type", "code".into()),
            (
                "scope",
                SCOPES
                    .map(|s| format!("https://www.googleapis.com/auth/{s}"))
                    .join(" "),
            ),
            ("access_type", "offline".into()),
            ("include_granted_scopes", "true".into()),
            ("prompt", "consent select_account".into()),
            ("state", nonce.clone()),
        ]);
        let hint = if intent == "login" {
            self.db
                .setting("googleOwnerId", Value::Null)?
                .as_str()
                .unwrap_or("")
                .to_owned()
        } else {
            id.to_owned()
        };
        if !hint.is_empty() {
            u.query_pairs_mut().append_pair("login_hint", &hint);
        }
        Ok(json!({"nonce":nonce,"url":u.as_str()}))
    }
    pub async fn finish_oauth(
        &self,
        state: &str,
        cookie: &str,
        code: &str,
        session: &str,
        base: &str,
    ) -> Result<bool> {
        let flow = {
            let mut flows = self
                .flows
                .lock()
                .map_err(|_| anyhow!("OAuth state unavailable"))?;
            let f = flows.get(state).cloned().unwrap_or(Value::Null);
            ensure!(
                !f.is_null() && number(&f, "expires") >= now() as f64 && state == cookie,
                "Connection expired. Return to the dashboard and try again."
            );
            flows.remove(state);
            f
        };
        if flow["intent"] != "login" {
            ensure!(
                self.db.valid_session(session)? && flow["sessionHash"] == hash(session),
                "Owner sign-in required"
            )
        }
        ensure!(
            !code.is_empty(),
            "Google connection was cancelled. Your accounts were not changed."
        );
        let r = self
            .client
            .post(self.endpoint("https://oauth2.googleapis.com/token", "/oauth/token"))
            .form(&[
                ("code", code.to_owned()),
                ("client_id", self.env("GOOGLE_CLIENT_ID")),
                ("client_secret", self.env("GOOGLE_CLIENT_SECRET")),
                ("redirect_uri", format!("{base}/auth/google/callback")),
                ("grant_type", "authorization_code".into()),
            ])
            .send()
            .await?;
        ensure!(
            r.status().is_success(),
            "Google connection failed. Please try again."
        );
        let mut token: Value = r.json().await?;
        token["expires_at"] = json!(now() + token["expires_in"].as_i64().unwrap_or(3600) * 1000);
        ensure!(
            authorized(&token),
            "Allow all requested calendar permissions to connect. Your accounts were not changed."
        );
        let id = self.identity(&token).await?;
        let owner = self.db.setting("googleOwnerId", Value::Null)?;
        ensure!(
            flow["intent"] != "login" || owner.is_null() || owner == id,
            "Sign in with the original dashboard owner account."
        );
        ensure!(
            flow["intent"] != "reconnect" || flow["accountId"] == id,
            "Choose the same Google account when reconnecting."
        );
        if !token["refresh_token"].is_string() {
            token["refresh_token"] = self.db.account(&id)?["token"]["refresh_token"].clone()
        }
        ensure!(
            token["refresh_token"].is_string(),
            "Google did not grant ongoing access. Please reconnect and allow access."
        );
        self.db.connect(&id, &token)?;
        self.errors
            .lock()
            .map_err(|_| anyhow!("Provider state unavailable"))?
            .remove(&id);
        Ok(flow["intent"] == "login")
    }
    pub async fn route(
        &self,
        origin: &str,
        destination: &str,
        mode: &str,
        departure: &str,
    ) -> Result<Value> {
        ensure!(
            !self.env("GOOGLE_ROUTES_API_KEY").is_empty(),
            "Google Routes API key is not configured"
        );
        let mut body = json!({"origin":{"address":origin},"destination":{"address":destination},"travelMode":mode});
        if mode == "DRIVE" {
            body["routingPreference"] = json!("TRAFFIC_AWARE")
        }
        if epoch(departure).is_some_and(|t| t > now() + 60_000) {
            body["departureTime"] = json!(departure)
        }
        let r = self
            .client
            .post(self.endpoint(
                "https://routes.googleapis.com/directions/v2:computeRoutes",
                "/routes",
            ))
            .header("X-Goog-Api-Key", self.env("GOOGLE_ROUTES_API_KEY"))
            .header("X-Goog-FieldMask", "routes.duration,routes.distanceMeters")
            .json(&body)
            .send()
            .await?;
        ensure!(
            r.status().is_success(),
            "Google Routes failed ({})",
            r.status().as_u16()
        );
        let v: Value = r.json().await?;
        let route = &v["routes"][0];
        let seconds = text(route, "duration")
            .strip_suffix('s')
            .and_then(|duration| duration.parse::<f64>().ok())
            .ok_or_else(|| anyhow!("Google Routes returned no duration"))?;
        ensure!(
            seconds.is_finite() && seconds >= 0.0,
            "Google Routes returned no duration"
        );
        Ok(
            json!({"seconds":seconds,"distanceMeters":route["distanceMeters"],"provider":"google-routes","mode":mode}),
        )
    }
    pub async fn jev(&self, state: &Value, questions: &Value) -> Result<Value> {
        ensure!(!self.env("TYPESAFE_API_KEY").is_empty(), "unconfigured");
        let r = self
            .client
            .post(self.endpoint("https://api.typesafe.ai/v1/systemone", "/jev"))
            .timeout(Duration::from_millis(2500))
            .bearer_auth(self.env("TYPESAFE_API_KEY"))
            .json(&json!({"model":MODEL,"state":state,"questions":questions}))
            .send()
            .await?;
        ensure!(r.status().is_success(), "http_error");
        let v: Value = r.json().await?;
        ensure!(v["model"] == MODEL, "model_mismatch");
        Ok(v)
    }
    pub async fn address(&self, input: &str, language: &str) -> Result<Vec<Value>> {
        let q = input
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(200)
            .collect::<String>();
        if q.chars().count() < 3 {
            return Ok(vec![]);
        }
        let lang = if ["de", "en", "fr"].contains(&language) {
            language
        } else {
            "default"
        };
        let r = self
            .client
            .get(self.endpoint("https://photon.komoot.io/api/", "/photon"))
            .timeout(Duration::from_secs(8))
            .query(&[("q", q.as_str()), ("limit", "6"), ("lang", lang)])
            .header("Accept", "application/geo+json, application/json")
            .send()
            .await?;
        ensure!(
            r.status().is_success(),
            "Address search failed ({})",
            r.status().as_u16()
        );
        let v: Value = r.json().await?;
        Ok(array(&v["features"])
            .iter()
            .filter_map(photon)
            .take(6)
            .collect())
    }
}
pub fn encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}
pub fn physical(s: &str) -> bool {
    !s.trim().is_empty()&&!regex::Regex::new(r"(?i)^(?:https?://|tel:)|\b(?:google\s*meet|zoom|microsoft\s*teams|teams\s*meeting|webex|online|virtual)\b").expect("static regex").is_match(s.trim())
}
fn photon(v: &Value) -> Option<Value> {
    let p = &v["properties"];
    let clean = |s: String| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(300)
            .collect::<String>()
    };
    let street = clean(text(p, "street"));
    let name = clean(text(p, "name"));
    let street_line = clean(format!("{} {}", street, text(p, "housenumber")));
    let locality = ["city", "town", "village", "locality"]
        .iter()
        .map(|k| text(p, k))
        .find(|s| !s.is_empty())
        .unwrap_or_default();
    let city = clean(format!("{} {}", text(p, "postcode"), locality));
    let parts = [
        if name != street { name } else { String::new() },
        street_line,
        city,
        text(p, "state"),
        text(p, "country"),
    ];
    let mut seen = HashSet::new();
    let address: Vec<_> = parts
        .into_iter()
        .map(clean)
        .filter(|s| !s.is_empty() && seen.insert(s.to_lowercase()))
        .collect();
    let id = format!(
        "{}:{}",
        text(p, "osm_type"),
        p["osm_id"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| if p["osm_id"].is_number() {
                p["osm_id"].to_string()
            } else {
                String::new()
            })
    );
    let longitude = v["geometry"]["coordinates"][0].as_f64()?;
    let latitude = v["geometry"]["coordinates"][1].as_f64()?;
    if id == ":" || address.len() < 2 {
        return None;
    }
    Some(
        json!({"providerId":id,"text":address.join(", "),"mainText":address[0],"secondaryText":address[1..].join(", "),"latitude":latitude,"longitude":longitude}),
    )
}
pub fn choice(answer: &Value, expected: &[&str]) -> Option<(String, f64)> {
    let c = text(answer, "choice");
    let confidence = answer["confidence"].as_f64()?;
    if answer["type"] != "choice"
        || !expected.contains(&c.as_str())
        || !(0.0..=1.0).contains(&confidence)
        || answer["probabilities"].as_object()?.len() != expected.len()
    {
        return None;
    }
    let mut sum = 0.0;
    for k in expected {
        let n = answer["probabilities"][k].as_f64()?;
        if !(0.0..=1.0).contains(&n) {
            return None;
        }
        sum += n
    }
    if (sum - 1.0).abs() > 0.0001 {
        return None;
    }
    Some((c, confidence))
}
