use crate::{
    app::App, array, crypto, epoch, hash, iso, now, number, providers, scheduling as s,
    store::Store, text,
};
use anyhow::{Result, anyhow, ensure};
use serde_json::{Value, json};
fn same(a: &str, b: &str) -> bool {
    epoch(a) == epoch(b)
}
fn cancelled(v: &Value) -> bool {
    v.is_null() || v["status"] == "cancelled"
}
fn applied(change: &Value, event: &Value) -> bool {
    if change["action"] == "cancel" {
        cancelled(event)
    } else {
        !cancelled(event)
            && same(&text(&event["start"], "dateTime"), &text(change, "start"))
            && same(&text(&event["end"], "dateTime"), &text(change, "end"))
    }
}
fn mutable(record: &Value) -> Result<()> {
    ensure!(
        record["status"] == "confirmed",
        "This meeting has been cancelled."
    );
    ensure!(
        epoch(&text(record, "start")).is_some_and(|t| t > now()),
        "This meeting has already started. Please contact Jaack for changes."
    );
    Ok(())
}
impl App {
    pub(crate) async fn event(&self, record: &Value) -> Result<Value> {
        if !self.demo {
            return self
                .providers
                .get_event(&text(record, "googleAccountId"), &text(record, "eventId"))
                .await;
        }
        let mut state = self
            .demo_state
            .lock()
            .map_err(|_| anyhow!("Demo state unavailable"))?;
        let reads = number(&state, "ambiguousReads");
        if reads > 0.0 {
            state["ambiguousReads"] = json!(reads - 1.0);
            return Err(anyhow!("Injected readback timeout"));
        }
        let id = text(record, "eventId");
        let v = if state["events"][&id].is_null() {
            if record["status"] == "pending" {
                Value::Null
            } else {
                json!({"id":id,"etag":format!("demo-{}",record["revision"]),"start":{"dateTime":record["start"]},"end":{"dateTime":record["end"]}})
            }
        } else {
            state["events"][&id].clone()
        };
        let external = state["external"][&id].clone();
        if !external.is_null() {
            state["external"]
                .as_object_mut()
                .expect("demo map")
                .remove(&id);
            state["events"][&id] = external;
        }
        Ok(v)
    }
    pub(crate) fn summary(&self, r: &Value) -> Value {
        json!({"meetingTitle":r["meetingTitle"],"start":r["start"],"end":r["end"],"status":r["status"],"revision":r["revision"],"pendingAction":r["pendingChange"]["action"],"canChange":r["status"]=="confirmed"&&epoch(&text(r,"start")).is_some_and(|t|t>now()),"meetLink":if r["status"]=="confirmed"{r["meetLink"].clone()}else{Value::Null}})
    }
    pub(crate) fn management_record(&self, token: &str) -> Result<Value> {
        let key = crypto::management_key(&self.secret, token)?;
        let record = self.db.get(&key)?;
        ensure!(
            record["eventId"].is_string()
                && ["confirmed", "cancelled"].contains(&text(&record, "status").as_str())
                && epoch(&text(&record, "end")).is_some_and(|t| t + 30 * 86_400_000 >= now()),
            "This meeting link is no longer available. Please contact Jaack."
        );
        Ok(record)
    }
    pub(crate) async fn load(&self, mut record: Value) -> Result<(Value, Value)> {
        if record["status"] == "cancelled" {
            return Ok((record, Value::Null));
        }
        ensure!(
            record["eventId"].is_string()
                && epoch(&text(&record, "end")).is_some_and(|t| t + 30 * 86_400_000 >= now()),
            "This meeting is no longer available."
        );
        let event = self.event(&record).await?;
        let pending = record["pendingChange"].clone();
        let key = text(&record, "idempotencyKey");
        if !pending.is_null() {
            if applied(&pending, &event) {
                record = self.db.complete_change(&key, &pending)?;
            } else if cancelled(&event) || event["etag"] != pending["etag"] {
                self.db.clear_change(&key)?;
                record = self.db.sync_event(&key, &event)?;
            }
        } else {
            record = self.db.sync_event(&key, &event)?
        }
        Ok((record, event))
    }
    pub(crate) async fn check_slot(&self, record: &Value, change: &Value) -> Result<()> {
        let start = text(change, "start");
        let end = text(change, "end");
        ensure!(
            s::constraint(&start, &end, &[], 0.0).is_none()
                && epoch(&start).is_some_and(|t| t > now()),
            "Choose a future meeting time."
        );
        ensure!(
            epoch(&end).zip(epoch(&start)).map(|(e, s)| e - s)
                == epoch(&text(record, "end"))
                    .zip(epoch(&text(record, "start")))
                    .map(|(e, s)| e - s),
            "Keep the original meeting length when rescheduling."
        );
        let duration = (epoch(&end).unwrap_or(0) - epoch(&start).unwrap_or(0)) / 60_000;
        let fresh=self.availability(&json!({"date":change["date"],"viewerTimeZone":change["viewerTimeZone"],"durationMinutes":duration}),None,None,192).await?;
        ensure!(
            array(&fresh["candidates"])
                .iter()
                .any(|v| same(&text(v, "start"), &start) && same(&text(v, "end"), &end)),
            "That time is no longer available. Choose another time."
        );
        self.db.assert_free(change, &text(record, "idempotencyKey"))
    }
    async fn dispatch_change(&self, record: &Value, change: &Value) -> Result<()> {
        if !self.demo {
            return self.providers.mutate(record, change).await;
        }
        let mut state = self
            .demo_state
            .lock()
            .map_err(|_| anyhow!("Demo state unavailable"))?;
        state["mutationCount"] = json!(number(&state, "mutationCount") + 1.0);
        let id = text(record, "eventId");
        if change["action"] == "cancel" {
            state["events"][&id] = json!({"id":id,"status":"cancelled"});
            if state["failAfterCancel"] == true {
                state["failAfterCancel"] = json!(false);
                state["ambiguousReads"] = json!(1);
                return Err(anyhow!("Injected lost provider response"));
            }
        } else {
            if state["failBeforeReschedule"] == true {
                state["failBeforeReschedule"] = json!(false);
                return Err(anyhow!(
                    "Injected provider failure before applying reschedule"
                ));
            }
            let current = state["events"][&id].clone();
            let etag = if current.is_null() {
                json!(format!("demo-{}", record["revision"]))
            } else {
                current["etag"].clone()
            };
            if etag != change["etag"] {
                return Err(providers::HttpError(412).into());
            }
            state["events"][&id] = json!({"id":id,"etag":uuid::Uuid::new_v4().to_string(),"start":{"dateTime":change["start"]},"end":{"dateTime":change["end"]}})
        }
        Ok(())
    }
    pub(crate) async fn change(&self, record: Value, event: Value, input: &Value) -> Result<Value> {
        let action = text(input, "action");
        ensure!(
            ["cancel", "reschedule", "retry"].contains(&action.as_str()),
            "Choose cancel or reschedule."
        );
        if record["status"] == "cancelled" && (action == "cancel" || action == "retry") {
            return Ok(self.summary(&record));
        }
        if action == "retry" && record["pendingChange"].is_null() {
            return Ok(self.summary(&record));
        }
        mutable(&record)?;
        let mut change = record["pendingChange"].clone();
        if !change.is_null() {
            ensure!(
                action == "retry",
                "An earlier change is awaiting confirmation. Use Retry to finish it."
            );
            if change["action"] == "reschedule" {
                self.check_slot(&record, &change).await?;
            }
        } else {
            ensure!(
                input["revision"] == record["revision"],
                "This meeting changed. Reload it before trying again."
            );
            ensure!(
                event["etag"].is_string(),
                "Unable to verify this meeting. Please try again."
            );
            change = json!({"action":action,"etag":event["etag"],"start":text(input,"start"),"end":text(input,"end"),"date":input["date"],"viewerTimeZone":input["viewerTimeZone"],"bufferMinutes":self.settings()?["minimumTravelBufferMinutes"]});
            if action == "reschedule" {
                self.check_slot(&record, &change).await?;
            }
            self.db.begin_change(
                &text(&record, "idempotencyKey"),
                record["revision"].as_i64().unwrap_or(0),
                &change,
            )?;
        }
        let key = text(&record, "idempotencyKey");
        match self.dispatch_change(&record, &change).await {
            Ok(()) => Ok(self.summary(&self.db.complete_change(&key, &change)?)),
            Err(error) => {
                let remote=self.event(&record).await.map_err(|_|anyhow!("The change could not be confirmed. Reload this page and use Retry; your request has been saved."))?;
                if applied(&change, &remote) {
                    return Ok(self.summary(&self.db.complete_change(&key, &change)?));
                }
                if providers::status(&error)
                    .is_some_and(|s| [400, 401, 403, 404, 410, 412].contains(&s))
                {
                    self.db.clear_change(&key)?;
                    return Err(anyhow!(
                        "The calendar could not accept this change. Reload and try again, or contact Jaack."
                    ));
                }
                Err(anyhow!(
                    "The change could not be confirmed. Reload this page and use Retry; your request has been saved."
                ))
            }
        }
    }
    fn normalize_booking(&self, input: &Value, prior: &Value) -> Result<Value> {
        let key = text(input, "idempotencyKey");
        ensure!(
            input["idempotencyKey"].is_string() && key.len() <= 256,
            "Invalid booking request"
        );
        ensure!(key.len() >= 8, "A strong idempotency key is required");
        let start = text(input, "start");
        let end = text(input, "end");
        ensure!(
            s::constraint(&start, &end, &[], 0.0).is_none(),
            "Invalid booking request"
        );
        ensure!(
            input["meetingTitle"].is_null() || input["meetingTitle"].is_string(),
            "Meeting title must be text"
        );
        let name = text(input, "guestName")
            .chars()
            .take(120)
            .collect::<String>();
        let email = text(input, "guestEmail")
            .chars()
            .take(254)
            .collect::<String>();
        let title = text(input, "meetingTitle").trim().to_owned();
        ensure!(
            title.encode_utf16().count() <= 200,
            "Meeting title must be 200 characters or fewer"
        );
        ensure!(
            !title.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}'),
            "Meeting title must be a single line"
        );
        let title = if title.is_empty() {
            if name.is_empty() {
                "Booked meeting".into()
            } else {
                format!("Meeting with {name}")
            }
        } else {
            title
        };
        let account = if !prior["googleAccountId"].is_null() {
            prior["googleAccountId"].clone()
        } else if self.demo {
            Value::Null
        } else {
            self.db.setting("googleBookingAccountId", Value::Null)?
        };
        let links = crypto::management_urls(&self.secret, &self.origin, &key);
        Ok(
            json!({"start":start,"end":end,"bufferMinutes":self.settings()?["minimumTravelBufferMinutes"],"idempotencyKey":key,"guestName":name,"guestEmail":email,"meetingTitle":title,"managementLinks":links,"googleAccountId":account,"providerEventId":format!("jev{}",hash(&key))}),
        )
    }
    fn public_booking(&self, record: &Value) -> Value {
        let mut result = record.clone();
        if let Some(v) = result.as_object_mut() {
            v.remove("googleAccountId");
            v.remove("pendingChange");
        }
        result["managementUrl"] = crypto::management_urls(
            &self.secret,
            &self.origin,
            &text(record, "idempotencyKey"),
        )["view"]
            .clone();
        json!({"booking":result,"demo":self.demo})
    }
    fn created_event(&self, v: &Value) -> Value {
        let meet = v["hangoutLink"].as_str().map(str::to_owned).or_else(|| {
            array(&v["conferenceData"]["entryPoints"])
                .iter()
                .find(|e| e["entryPointType"] == "video")
                .map(|e| text(e, "uri"))
        });
        json!({"id":v["id"],"meetLink":meet})
    }
    pub(crate) async fn book(&self, input: &Value) -> Result<(u16, Value)> {
        let key = text(input, "idempotencyKey");
        let prior = self.db.get(&key)?;
        let v = self.normalize_booking(input, &prior)?;
        if !prior.is_null() {
            ensure!(
                prior["status"] != "cancelled",
                "This booking was cancelled. Start a new booking instead."
            );
            for field in ["start", "end", "guestName", "guestEmail", "meetingTitle"] {
                ensure!(
                    text(&prior, field) == text(&v, field),
                    "Booking request key already used for a different request"
                )
            }
            ensure!(
                prior["googleAccountId"] == v["googleAccountId"],
                "Booking account changed. Please choose the slot again."
            );
            if prior["status"] == "confirmed" {
                return Ok((200, self.public_booking(&prior)));
            }
            if prior["status"] == "pending" {
                ensure!(
                    prior["eventId"] == v["providerEventId"],
                    "An earlier creation needs reconciliation before it can be retried."
                );
                let event = self.event(&prior).await?;
                if !event.is_null() {
                    ensure!(
                        applied(
                            &json!({"action":"reschedule","start":v["start"],"end":v["end"]}),
                            &event
                        ),
                        "The pending event changed in Google Calendar. Contact Jaack."
                    );
                    let confirmed = self.db.confirm(&key, &self.created_event(&event))?;
                    return Ok((200, self.public_booking(&confirmed)));
                }
            }
        }
        ensure!(
            epoch(&text(&v, "start")).is_some_and(|t| t > now()),
            "Booking must be in the future"
        );
        let zone = if s::valid_zone(&text(input, "viewerTimeZone")) {
            text(input, "viewerTimeZone")
        } else {
            text(&self.settings()?, "timezone")
        };
        let date = if input["date"].is_string() {
            text(input, "date")
        } else {
            s::date(
                epoch(&text(&v, "start")).ok_or_else(|| anyhow!("Invalid booking request"))?,
                &zone,
            )?
        };
        let fresh=self.availability(&json!({"date":date,"viewerTimeZone":zone,"durationMinutes":input["durationMinutes"]}),None,None,192).await?;
        ensure!(
            array(&fresh["candidates"])
                .iter()
                .any(|slot| slot["start"] == v["start"] && slot["end"] == v["end"]),
            "That slot is no longer available"
        );
        let mut reserved = self.db.reserve(&v)?;
        if reserved["status"] == "failed" {
            self.db.assert_free(&v, &key)?;
            self.db.execute(
                "UPDATE bookings SET status='pending' WHERE idempotency_key=?",
                &[&key],
            )?;
        }
        self.db.execute(
            "UPDATE bookings SET event_id=? WHERE idempotency_key=?",
            &[&text(&v, "providerEventId"), &key],
        )?;
        reserved["eventId"] = v["providerEventId"].clone();
        if !self.demo {
            let calendar = self.providers.calendar()?;
            let buffer = number(&v, "bufferMinutes") * 60_000.0;
            let start = epoch(&text(&v, "start")).ok_or_else(|| anyhow!("Invalid booking"))?;
            let end = epoch(&text(&v, "end")).ok_or_else(|| anyhow!("Invalid booking"))?;
            let busy = self
                .providers
                .free_busy(
                    &calendar,
                    &iso(start - buffer as i64),
                    &iso(end + buffer as i64),
                )
                .await?;
            if s::constraint(
                &text(&v, "start"),
                &text(&v, "end"),
                &busy,
                number(&v, "bufferMinutes"),
            )
            .is_some()
            {
                self.db.execute(
                    "UPDATE bookings SET status='failed' WHERE idempotency_key=?",
                    &[&key],
                )?;
                return Err(anyhow!("Booking conflict: busy"));
            }
        }
        let created = if self.demo {
            let event = json!({"id":v["providerEventId"],"etag":"demo-0","start":{"dateTime":v["start"]},"end":{"dateTime":v["end"]},"attendees":[{"email":v["guestEmail"]}]});
            let mut state = self
                .demo_state
                .lock()
                .map_err(|_| anyhow!("Demo state unavailable"))?;
            let id = text(&v, "providerEventId");
            state["events"][&id] = event.clone();
            state["creationCount"] = json!(number(&state, "creationCount") + 1.0);
            if state["ambiguousCreate"] == true {
                state["ambiguousCreate"] = json!(false);
                state["ambiguousReads"] = json!(1);
                Err(anyhow!("Injected lost creation response"))
            } else {
                Ok(event)
            }
        } else {
            self.providers.create(&v).await
        };
        let event = match created {
            Ok(v) => v,
            Err(error) => {
                let remote = self.event(&reserved).await;
                if let Ok(remote) = remote {
                    if !remote.is_null()
                        && applied(
                            &json!({"action":"reschedule","start":v["start"],"end":v["end"]}),
                            &remote,
                        )
                    {
                        remote
                    } else {
                        if providers::status(&error).is_some_and(|s| [400, 401, 403].contains(&s)) {
                            self.db.execute(
                                "UPDATE bookings SET status='failed' WHERE idempotency_key=?",
                                &[&key],
                            )?;
                        }
                        return Err(anyhow!(
                            "Booking creation could not be confirmed. Retry the same request; its identity has been saved."
                        ));
                    }
                } else {
                    return Err(anyhow!(
                        "Booking creation could not be confirmed. Retry the same request; its identity has been saved."
                    ));
                }
            }
        };
        let confirmed = self.db.confirm(&key, &self.created_event(&event))?;
        Ok((201, self.public_booking(&confirmed)))
    }
    pub(crate) async fn owner_change(
        &self,
        source: &str,
        id: &str,
        input: &Value,
    ) -> Result<(u16, Value)> {
        let action = text(input, "action");
        let allowed = if action == "cancel" {
            vec!["action", "revision"]
        } else {
            vec![
                "action",
                "revision",
                "start",
                "end",
                "date",
                "viewerTimeZone",
            ]
        };
        if !input.is_object()
            || input
                .as_object()
                .is_some_and(|o| o.keys().any(|k| !allowed.contains(&k.as_str())))
        {
            return Ok((400, json!({"error":"Unexpected booking change field."})));
        }
        if !uuid::Uuid::parse_str(id).is_ok_and(|u| {
            u.get_version_num() >= 1
                && u.get_version_num() <= 8
                && u.get_variant() == uuid::Variant::RFC4122
        }) {
            return Ok((400, json!({"error":"Idempotency-Key must be a UUID."})));
        }
        if !["cancel", "reschedule"].contains(&action.as_str())
            || input["revision"].as_i64().is_none_or(|n| n < 0)
        {
            return Ok((
                400,
                json!({"error":"Provide an action and expected booking revision."}),
            ));
        }
        let mut normalized =
            json!({"sourceId":source,"action":action,"revision":input["revision"]});
        if action == "reschedule" {
            if epoch(&text(input, "start")).is_none()
                || epoch(&text(input, "end")).is_none()
                || !regex::Regex::new(r"^\d{4}-\d{2}-\d{2}$")
                    .expect("static regex")
                    .is_match(&text(input, "date"))
                || !s::valid_zone(&text(input, "viewerTimeZone"))
            {
                return Ok((
                    400,
                    json!({"error":"Reschedule requires an exact start, end, date and time zone."}),
                ));
            }
            for field in ["start", "end", "date", "viewerTimeZone"] {
                normalized[field] = input[field].clone()
            }
        }
        let request_hash = hash(&normalized.to_string());
        let journal = match self.db.begin_journal(id, source, &request_hash) {
            Ok(v) => v,
            Err(e) => return Ok((409, json!({"error":e.to_string()}))),
        };
        let replayed = journal["replayed"] == true;
        if journal["state"] == "succeeded" || journal["state"] == "rejected" {
            let mut v = journal["response"].clone();
            v["replayed"] = json!(true);
            return Ok((
                if journal["state"] == "succeeded" {
                    200
                } else {
                    409
                },
                v,
            ));
        }
        let record = self.db.by_source(source)?;
        if record.is_null() {
            let v = json!({"error":"Booking not found."});
            self.db.finish_journal(id, "rejected", &v)?;
            return Ok((404, v));
        }
        let work=async{let(record,event)=self.load(record).await?;let already=if action=="cancel"{record["status"]=="cancelled"}else{record["status"]=="confirmed"&&same(&text(&record,"start"),&text(input,"start"))&&same(&text(&record,"end"),&text(input,"end"))};if already{ensure!(replayed,"This booking already reflects that change. Reload it before creating a new request.");return Ok::<_,anyhow::Error>(json!({"booking":Store::owner(&record)}))}let pending=&record["pendingChange"];if !pending.is_null(){ensure!(replayed,"A booking change is already awaiting confirmation.");ensure!(pending["action"]==input["action"]&&record["revision"]==input["revision"]&&(action=="cancel"||(same(&text(pending,"start"),&text(input,"start"))&&same(&text(pending,"end"),&text(input,"end")))),"A different booking change is awaiting confirmation.");}else{ensure!(record["revision"]==input["revision"],"This booking changed. Reload it before trying again.");}ensure!(record["status"]=="confirmed","This booking is no longer mutable.");let retry=json!({"action":"retry"});self.change(record.clone(),event,if pending.is_null(){input}else{&retry}).await?;Ok(json!({"booking":Store::owner(&self.db.by_source(source)?)}))}.await;
        match work {
            Ok(mut v) => {
                self.db.finish_journal(id, "succeeded", &v)?;
                v["replayed"] = json!(replayed);
                Ok((200, v))
            }
            Err(e) => {
                let r = self.db.by_source(source)?;
                let uncertain = !r["pendingChange"].is_null();
                let mut v = json!({"error":e.to_string()});
                if uncertain {
                    v["state"] = json!("uncertain");
                    v["pendingAction"] = r["pendingChange"]["action"].clone();
                }
                self.db
                    .finish_journal(id, if uncertain { "uncertain" } else { "rejected" }, &v)?;
                Ok((if uncertain { 503 } else { 409 }, v))
            }
        }
    }
}
