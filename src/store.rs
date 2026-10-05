use crate::{
    crypto::{SecretBox, random},
    hash, iso, now, number, scheduling, text,
};
use anyhow::{Result, anyhow, ensure};
use rusqlite::{Connection, OptionalExtension, params, types::ValueRef};
use serde_json::{Value, json};
use std::{path::Path, sync::Mutex};

pub struct Store {
    connection: Mutex<Connection>,
    secret: SecretBox,
}
pub fn rows(c: &Connection, sql: &str, p: &[&dyn rusqlite::ToSql]) -> Result<Vec<Value>> {
    let mut stmt = c.prepare(sql)?;
    let names: Vec<_> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let values = stmt.query_map(p, |row| {
        let mut v = json!({});
        for (i, n) in names.iter().enumerate() {
            v[n] = match row.get_ref(i)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(x) => json!(x),
                ValueRef::Real(x) => json!(x),
                ValueRef::Text(x) => json!(String::from_utf8_lossy(x)),
                ValueRef::Blob(_) => Value::Null,
            };
        }
        Ok(v)
    })?;
    Ok(values.collect::<std::result::Result<Vec<_>, _>>()?)
}
fn booking(c: &Connection, key: &str) -> Result<Value> {
    let mut v=rows(c,"SELECT idempotency_key AS idempotencyKey,start,end,buffer_minutes AS bufferMinutes,guest_name AS guestName,guest_email AS guestEmail,status,event_id AS eventId,meet_link AS meetLink,meeting_title AS meetingTitle,google_account_id AS googleAccountId,revision,pending_change AS pendingChange,created_at AS createdAt FROM bookings WHERE idempotency_key=?",&[&key])?.into_iter().next().unwrap_or(Value::Null);
    if !v.is_null() {
        v["pendingChange"] = v["pendingChange"]
            .as_str()
            .map(serde_json::from_str)
            .transpose()?
            .unwrap_or(Value::Null);
    }
    Ok(v)
}
fn setting(c: &Connection, key: &str, fallback: Value) -> Result<Value> {
    let v: Option<String> = c
        .query_row("SELECT value FROM settings WHERE key=?", [key], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(v.map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or(fallback))
}
fn put(c: &Connection, key: &str, value: &Value) -> Result<()> {
    c.execute(
        "INSERT OR REPLACE INTO settings VALUES (?,?)",
        params![key, crate::canonical(value).to_string()],
    )?;
    Ok(())
}
fn check_slot(c: &Connection, v: &Value, exclude: &str) -> Result<()> {
    for row in rows(
        c,
        "SELECT start,end,buffer_minutes AS bufferMinutes,pending_change AS pendingChange FROM bookings WHERE status IN ('pending','confirmed') AND idempotency_key != ?",
        &[&exclude],
    )? {
        let buffer = number(v, "bufferMinutes").max(number(&row, "bufferMinutes"));
        let mut slots = vec![row.clone()];
        if let Some(p) = row["pendingChange"].as_str() {
            let change: Value = serde_json::from_str(p)?;
            if change["action"] == "reschedule" {
                slots.push(change)
            }
        }
        ensure!(
            scheduling::constraint(&text(v, "start"), &text(v, "end"), &slots, buffer).is_none(),
            "Booking conflict"
        );
    }
    Ok(())
}
fn outcome(c: &Connection, key: &str, start: &str) -> Result<()> {
    c.execute(
        "INSERT OR IGNORE INTO booking_slot_outcomes VALUES (?,?,?,?)",
        params![hash(key), start, "rescheduled", iso(now())],
    )?;
    Ok(())
}
fn transaction<T>(c: &Connection, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    c.execute_batch("BEGIN IMMEDIATE")?;
    match f(c) {
        Ok(v) => {
            c.execute_batch("COMMIT")?;
            Ok(v)
        }
        Err(e) => {
            c.execute_batch("ROLLBACK")?;
            Err(e)
        }
    }
}
impl Store {
    pub fn open(path: &str, secret: &str) -> Result<Self> {
        if let Some(p) = Path::new(path).parent()
            && !p.as_os_str().is_empty()
        {
            std::fs::create_dir_all(p)?;
        }
        let c = Connection::open(path)?;
        c.busy_timeout(std::time::Duration::from_secs(5))?;
        c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY,value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS tokens (provider TEXT PRIMARY KEY,value TEXT NOT NULL,updated_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS sessions (token_hash TEXT PRIMARY KEY,expires_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS google_accounts (id TEXT PRIMARY KEY,token TEXT NOT NULL,version TEXT NOT NULL,connected_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS bookings (idempotency_key TEXT PRIMARY KEY,start TEXT NOT NULL,end TEXT NOT NULL,buffer_minutes INTEGER NOT NULL,guest_name TEXT,guest_email TEXT,status TEXT NOT NULL,event_id TEXT,created_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS jaack_os_booking_changes (request_id TEXT PRIMARY KEY,source_id TEXT NOT NULL,request_hash TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('pending','succeeded','uncertain','rejected')),response TEXT,created_at TEXT NOT NULL,updated_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS booking_slot_outcomes (booking_hash TEXT NOT NULL,start TEXT NOT NULL,outcome TEXT NOT NULL CHECK(outcome='rescheduled'),recorded_at TEXT NOT NULL,PRIMARY KEY(booking_hash,start,outcome));")?;
        let columns = rows(&c, "PRAGMA table_info(bookings)", &[])?;
        for (name, kind) in [
            ("meet_link", "TEXT"),
            ("meeting_title", "TEXT"),
            ("google_account_id", "TEXT"),
            ("revision", "INTEGER NOT NULL DEFAULT 0"),
            ("pending_change", "TEXT"),
            ("source_id", "TEXT"),
        ] {
            if !columns.iter().any(|v| v["name"] == name) {
                c.execute_batch(&format!("ALTER TABLE bookings ADD COLUMN {name} {kind}"))?;
            }
        }
        for row in rows(
            &c,
            "SELECT idempotency_key FROM bookings WHERE source_id IS NULL",
            &[],
        )? {
            let key = text(&row, "idempotency_key");
            c.execute(
                "UPDATE bookings SET source_id=? WHERE idempotency_key=?",
                params![hash(&key), key],
            )?;
        }
        c.execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS bookings_source_id ON bookings(source_id)",
        )?;
        Ok(Self {
            connection: Mutex::new(c),
            secret: SecretBox::new(secret),
        })
    }
    // SQLite never holds a connection lock or transaction over a provider await.
    pub fn with<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let run = || {
            let c = self
                .connection
                .lock()
                .map_err(|_| anyhow!("Store unavailable"))?;
            f(&c)
        };
        if tokio::runtime::Handle::try_current().is_ok() {
            tokio::task::block_in_place(run)
        } else {
            run()
        }
    }
    pub fn get(&self, key: &str) -> Result<Value> {
        self.with(|c| booking(c, key))
    }
    pub fn setting(&self, key: &str, fallback: Value) -> Result<Value> {
        self.with(|c| setting(c, key, fallback))
    }
    pub fn put(&self, key: &str, v: &Value) -> Result<()> {
        self.with(|c| put(c, key, v))
    }
    pub fn query(&self, sql: &str, p: &[&dyn rusqlite::ToSql]) -> Result<Vec<Value>> {
        self.with(|c| rows(c, sql, p))
    }
    pub fn execute(&self, sql: &str, p: &[&dyn rusqlite::ToSql]) -> Result<usize> {
        self.with(|c| Ok(c.execute(sql, p)?))
    }
    pub fn by_source(&self, id: &str) -> Result<Value> {
        let r = self.query(
            "SELECT idempotency_key FROM bookings WHERE source_id=?",
            &[&id],
        )?;
        r.first()
            .map(|v| self.get(&text(v, "idempotency_key")))
            .unwrap_or(Ok(Value::Null))
    }
    pub fn owner(v: &Value) -> Value {
        json!({"id":hash(&text(v,"idempotencyKey")),"source":"jev-calendar","start":v["start"],"end":v["end"],"status":v["status"],"revision":v["revision"],"pendingAction":v["pendingChange"]["action"],"createdAt":v["createdAt"]})
    }
    pub fn list_owner(&self, limit: i64, offset: i64) -> Result<Value> {
        let r=self.query("SELECT idempotency_key FROM bookings ORDER BY created_at DESC,idempotency_key DESC LIMIT ? OFFSET ?",&[&(limit+1),&offset])?;
        let values: Vec<_> = r
            .iter()
            .take(limit as usize)
            .map(|v| {
                self.get(&text(v, "idempotency_key"))
                    .map(|r| Self::owner(&r))
            })
            .collect::<Result<_>>()?;
        Ok(
            json!({"bookings":values,"hasMore":r.len()>limit as usize,"limit":limit,"offset":offset}),
        )
    }
    pub fn assert_free(&self, v: &Value, exclude: &str) -> Result<()> {
        self.with(|c| check_slot(c, v, exclude))
    }
    pub fn reserve(&self, v: &Value) -> Result<Value> {
        let key = text(v, "idempotencyKey");
        self.with(|c| transaction(c, |c| {
            let prior = booking(c, &key)?;
            if !prior.is_null() {
                return Ok(prior);
            }
            check_slot(c, v, "")?;
            c.execute(
                "INSERT INTO bookings (idempotency_key,source_id,start,end,buffer_minutes,guest_name,guest_email,meeting_title,google_account_id,status,created_at) VALUES (?,?,?,?,?,?,?,?,?,'pending',?)",
                params![key, hash(&key), text(v, "start"), text(v, "end"),
                    number(v, "bufferMinutes"), v["guestName"].as_str(),
                    v["guestEmail"].as_str(), v["meetingTitle"].as_str(),
                    v["googleAccountId"].as_str(), iso(now())],
            )?;
            booking(c, &key)
        }))
    }
    pub fn confirm(&self, key: &str, event: &Value) -> Result<Value> {
        self.execute(
            "UPDATE bookings SET status='confirmed',event_id=?,meet_link=? WHERE idempotency_key=?",
            &[&text(event, "id"), &event["meetLink"].as_str(), &key],
        )?;
        self.get(key)
    }
    pub fn begin_change(&self, key: &str, revision: i64, change: &Value) -> Result<Value> {
        self.with(|c| {
            transaction(c, |c| {
                let v = booking(c, key)?;
                ensure!(
                    v["status"] == "confirmed"
                        && v["revision"] == revision
                        && v["pendingChange"].is_null(),
                    "This booking changed. Reload it before trying again."
                );
                if change["action"] == "reschedule" {
                    check_slot(c, change, key)?;
                }
                c.execute(
                    "UPDATE bookings SET pending_change=? WHERE idempotency_key=?",
                    params![change.to_string(), key],
                )?;
                booking(c, key)
            })
        })
    }
    pub fn clear_change(&self, key: &str) -> Result<()> {
        self.execute(
            "UPDATE bookings SET pending_change=NULL WHERE idempotency_key=?",
            &[&key],
        )?;
        Ok(())
    }
    pub fn complete_change(&self, key: &str, v: &Value) -> Result<Value> {
        self.with(|c| transaction(c, |c| {
            if v["action"] == "cancel" {
                c.execute(
                    "UPDATE bookings SET status='cancelled',pending_change=NULL,revision=revision+1 WHERE idempotency_key=?",
                    [key],
                )?;
            } else {
                let old = booking(c, key)?;
                if crate::epoch(&text(&old, "start")) != crate::epoch(&text(v, "start")) {
                    outcome(c, key, &text(&old, "start"))?;
                }
                c.execute(
                    "UPDATE bookings SET start=?,end=?,buffer_minutes=?,pending_change=NULL,revision=revision+1 WHERE idempotency_key=?",
                    params![text(v, "start"), text(v, "end"), number(v, "bufferMinutes"), key],
                )?;
            }
            booking(c, key)
        }))
    }
    pub fn sync_event(&self, key: &str, event: &Value) -> Result<Value> {
        if event.is_null() || event["status"] == "cancelled" {
            return self.complete_change(key, &json!({"action":"cancel"}));
        }
        let old = self.get(key)?;
        let start = text(&event["start"], "dateTime");
        let end = text(&event["end"], "dateTime");
        if crate::epoch(&start) != crate::epoch(&text(&old, "start"))
            || crate::epoch(&end) != crate::epoch(&text(&old, "end"))
        {
            ensure!(
                !start.is_empty() && !end.is_empty(),
                "This event was changed in Google Calendar. Please contact Jaack to manage it."
            );
            self.with(|c|transaction(c,|c|{outcome(c,key,&text(&old,"start"))?;c.execute("UPDATE bookings SET start=?,end=?,revision=revision+1 WHERE idempotency_key=?",params![start,end,key])?;Ok(())}))?;
        }
        self.get(key)
    }
    pub fn feedback(&self) -> Result<Vec<Value>> {
        let since = iso(now() - 365 * 86_400_000);
        let mut r=self.query("SELECT start,status AS outcome FROM bookings WHERE status IN ('confirmed','cancelled') AND created_at>=?",&[&since])?;
        r.extend(self.query(
            "SELECT start,outcome FROM booking_slot_outcomes WHERE recorded_at>=?",
            &[&since],
        )?);
        Ok(r)
    }
    pub fn accounts(&self) -> Result<Vec<Value>> {
        self.query("SELECT id,version,connected_at AS connectedAt FROM google_accounts ORDER BY connected_at,id",&[])
    }
    pub fn account(&self, id: &str) -> Result<Value> {
        let mut r = self
            .query(
                "SELECT id,version,token FROM google_accounts WHERE id=?",
                &[&id],
            )?
            .into_iter()
            .next()
            .unwrap_or(Value::Null);
        if !r.is_null() {
            r["token"] = serde_json::from_str(&self.secret.decrypt(&text(&r, "token"))?)?;
        }
        Ok(r)
    }
    pub fn connect(&self, id: &str, token: &Value) -> Result<()> {
        let encrypted = self.secret.encrypt(&token.to_string())?;
        self.with(|c| transaction(c, |c| {
            c.execute(
                "INSERT INTO google_accounts VALUES (?,?,?,?) ON CONFLICT(id) DO UPDATE SET token=excluded.token,version=excluded.version",
                params![id, encrypted, uuid::Uuid::new_v4().simple().to_string(), iso(now())],
            )?;
            for key in ["googleOwnerId", "googleBookingAccountId"] {
                if setting(c, key, Value::Null)?.is_null() {
                    put(c, key, &json!(id))?;
                }
            }
            Ok(())
        }))
    }
    pub fn refresh(&self, id: &str, version: &str, token: &Value) -> Result<bool> {
        let encrypted = self.secret.encrypt(&token.to_string())?;
        Ok(self.execute(
            "UPDATE google_accounts SET token=? WHERE id=? AND version=?",
            &[&encrypted, &id, &version],
        )? == 1)
    }
    pub fn select(&self, id: &str) -> Result<()> {
        ensure!(
            !self.account(id)?.is_null(),
            "Google account is not connected"
        );
        self.put("googleBookingAccountId", &json!(id))
    }
    pub fn disconnect(&self, id: &str) -> Result<()> {
        ensure!(
            !self.account(id)?.is_null(),
            "Google account is not connected"
        );
        let selected = self.setting("googleBookingAccountId", Value::Null)? == id;
        ensure!(
            !selected || self.accounts()?.len() <= 1,
            "Choose another account for new bookings before disconnecting this one."
        );
        self.with(|c| {
            transaction(c, |c| {
                c.execute("DELETE FROM google_accounts WHERE id=?", [id])?;
                if selected {
                    put(c, "googleBookingAccountId", &Value::Null)?;
                }
                Ok(())
            })
        })
    }
    pub fn session(&self) -> Result<String> {
        let token = random(32);
        self.execute(
            "INSERT INTO sessions VALUES (?,?)",
            &[&hash(&token), &iso(now() + 8 * 3_600_000)],
        )?;
        Ok(token)
    }
    pub fn valid_session(&self, raw: &str) -> Result<bool> {
        Ok(!raw.is_empty()
            && self
                .query(
                    "SELECT expires_at FROM sessions WHERE token_hash=?",
                    &[&hash(raw)],
                )?
                .first()
                .and_then(|v| crate::epoch(&text(v, "expires_at")))
                .is_some_and(|t| t > now()))
    }
    pub fn journal(&self, id: &str) -> Result<Value> {
        let mut v=self.query("SELECT request_id AS requestId,source_id AS sourceId,request_hash AS requestHash,state,response FROM jaack_os_booking_changes WHERE request_id=?",&[&id])?.into_iter().next().unwrap_or(Value::Null);
        if !v.is_null() {
            v["response"] = v["response"]
                .as_str()
                .map(serde_json::from_str)
                .transpose()?
                .unwrap_or(Value::Null);
        }
        Ok(v)
    }
    pub fn begin_journal(&self, id: &str, source: &str, request_hash: &str) -> Result<Value> {
        self.with(|c| transaction(c, |c| {
            let mut prior = rows(c,
                "SELECT request_id AS requestId,source_id AS sourceId,request_hash AS requestHash,state,response FROM jaack_os_booking_changes WHERE request_id=?",
                &[&id],
            )?.into_iter().next().unwrap_or(Value::Null);
            if !prior.is_null() {
                ensure!(prior["sourceId"] == source && prior["requestHash"] == request_hash,
                    "Idempotency key already belongs to a different booking change.");
                prior["response"] = prior["response"].as_str()
                    .map(serde_json::from_str).transpose()?.unwrap_or(Value::Null);
                prior["replayed"] = json!(true);
                return Ok(prior);
            }
            let time = iso(now());
            c.execute(
                "INSERT INTO jaack_os_booking_changes (request_id,source_id,request_hash,state,created_at,updated_at) VALUES (?,?,?,'pending',?,?)",
                params![id, source, request_hash, time, time],
            )?;
            Ok(json!({
                "requestId": id, "sourceId": source, "requestHash": request_hash,
                "state": "pending", "response": null, "replayed": false,
            }))
        }))
    }
    pub fn finish_journal(&self, id: &str, state: &str, response: &Value) -> Result<()> {
        self.execute("UPDATE jaack_os_booking_changes SET state=?,response=?,updated_at=? WHERE request_id=?",&[&state,&response.to_string(),&iso(now()),&id])?;
        Ok(())
    }
    pub fn backup(&self, dest: &str) -> Result<()> {
        self.with(|c| {
            let mut out = Connection::open(dest)?;
            rusqlite::backup::Backup::new(c, &mut out)?.run_to_completion(
                64,
                std::time::Duration::from_millis(10),
                None,
            )?;
            Ok(())
        })
    }
}
