use crate::Store;
use pebble_core::{PebbleError, Result};
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScheduledSend {
    pub id: String,
    pub account_id: String,
    pub kind: String,
    pub due_at: i64,
    pub state: String,
    pub payload: Vec<u8>,
    pub revision: i64,
}

fn read(row: &Row<'_>) -> rusqlite::Result<ScheduledSend> {
    Ok(ScheduledSend {
        id: row.get(0)?,
        account_id: row.get(1)?,
        kind: row.get(2)?,
        due_at: row.get(3)?,
        state: row.get(4)?,
        payload: row.get(5)?,
        revision: row.get(6)?,
    })
}

pub fn validate_future(due_at: i64, now: i64) -> Result<()> {
    if due_at <= now || due_at > now.saturating_add(366 * 24 * 60 * 60) {
        return Err(PebbleError::Validation(
            "Choose a future time within one year".into(),
        ));
    }
    Ok(())
}

impl Store {
    pub fn scheduled_source_draft_ids(&self, account_id: &str) -> Result<Vec<String>> {
        self.with_read(|conn| {
            let mut query = conn.prepare(
                "SELECT id FROM messages WHERE account_id=?1 AND is_draft=1 AND is_deleted=0",
            )?;
            let rows = query
                .query_map([account_id], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn insert_scheduled_send(&self, job: &ScheduledSend, now: i64) -> Result<()> {
        validate_future(job.due_at, now)?;
        self.with_write(|conn| {
            conn.execute("INSERT INTO scheduled_sends (id,account_id,kind,due_at,state,payload,revision,updated_at) VALUES (?1,?2,?3,?4,'pending',?5,0,?6)",
                params![job.id, job.account_id, job.kind, job.due_at, job.payload, now])?;
            Ok(())
        })
    }

    pub fn list_scheduled_sends(&self) -> Result<Vec<ScheduledSend>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare("SELECT id,account_id,kind,due_at,state,payload,revision FROM scheduled_sends ORDER BY due_at,id")?;
            let rows = stmt.query_map([], read)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn get_scheduled_send(&self, id: &str) -> Result<Option<ScheduledSend>> {
        self.with_read(|conn| conn.query_row("SELECT id,account_id,kind,due_at,state,payload,revision FROM scheduled_sends WHERE id=?1", [id], read).optional().map_err(Into::into))
    }

    pub fn due_scheduled_sends(&self, now: i64) -> Result<Vec<ScheduledSend>> {
        self.with_read(|conn| {
            let mut stmt=conn.prepare("SELECT id,account_id,kind,due_at,state,payload,revision FROM scheduled_sends WHERE state='pending' AND due_at<=?1 ORDER BY due_at,id LIMIT 10")?;
            let rows=stmt.query_map([now],read)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn recover_scheduled_sends(&self, now: i64) -> Result<usize> {
        self.with_write(|conn| {
            Ok(conn.execute("UPDATE scheduled_sends SET state='review',revision=revision+1,updated_at=?1 WHERE state='dispatching' OR (state='pending' AND (kind='undo' OR due_at<=?1))", [now])?)
        })
    }

    pub fn claim_scheduled_send(&self, id: &str, revision: i64, now: i64) -> Result<bool> {
        self.with_write(|conn| Ok(conn.execute("UPDATE scheduled_sends SET state='dispatching',revision=revision+1,updated_at=?3 WHERE id=?1 AND revision=?2 AND state='pending' AND due_at<=?3", params![id,revision,now])? == 1))
    }

    /// Only the worker that claimed this revision can finish it. No retry state exists.
    pub fn finish_scheduled_send(
        &self,
        id: &str,
        revision: i64,
        sent: bool,
        now: i64,
    ) -> Result<bool> {
        self.with_write(|conn| Ok(conn.execute("UPDATE scheduled_sends SET state=?3,revision=revision+1,updated_at=?4 WHERE id=?1 AND revision=?2 AND state='dispatching'", params![id,revision,if sent { "sent" } else { "review" },now])? == 1))
    }

    pub fn restore_scheduled_send(&self, id: &str, revision: i64, now: i64) -> Result<bool> {
        self.with_write(|conn| Ok(conn.execute("UPDATE scheduled_sends SET state='draft',revision=revision+1,updated_at=?3 WHERE id=?1 AND revision=?2 AND state IN ('pending','review','draft')",params![id,revision,now])? == 1))
    }

    pub fn reschedule_send(&self, id: &str, revision: i64, due_at: i64, now: i64) -> Result<bool> {
        validate_future(due_at, now)?;
        self.with_write(|conn| Ok(conn.execute("UPDATE scheduled_sends SET state='pending',kind='scheduled',due_at=?3,revision=revision+1,updated_at=?4 WHERE id=?1 AND revision=?2 AND state='pending'",params![id,revision,due_at,now])? == 1))
    }

    pub fn delete_scheduled_send(&self, id: &str, revision: i64) -> Result<bool> {
        self.with_write(|conn| Ok(conn.execute("DELETE FROM scheduled_sends WHERE id=?1 AND revision=?2 AND state IN ('draft','sent')",params![id,revision])? == 1))
    }

    /// Internal encrypted cleanup receipts do not change the user-visible delivery revision.
    pub fn update_delivered_send_payload(
        &self,
        id: &str,
        revision: i64,
        payload: &[u8],
    ) -> Result<()> {
        self.with_write(|conn| {
            if conn.execute("UPDATE scheduled_sends SET payload=?3 WHERE id=?1 AND revision=?2 AND state='sent'", params![id, revision, payload])? != 1 {
                return Err(PebbleError::Validation("Delivered message changed during draft cleanup".into()));
            }
            Ok(())
        })
    }

    pub fn scheduled_send_exit_count(&self) -> Result<usize> {
        self.with_read(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM scheduled_sends WHERE state IN ('pending','dispatching')",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n as usize)
            .map_err(Into::into)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    fn job(store: &Store, id: &str, kind: &str, due: i64) {
        store.with_write(|conn| { conn.execute("INSERT OR IGNORE INTO accounts (id,email,display_name,provider,created_at,updated_at) VALUES ('a','a@example.com','A','imap',0,0)",[])?; Ok(()) }).unwrap();
        store
            .insert_scheduled_send(
                &ScheduledSend {
                    id: id.into(),
                    account_id: "a".into(),
                    kind: kind.into(),
                    due_at: due,
                    state: "pending".into(),
                    payload: vec![1, 2, 3],
                    revision: 0,
                },
                100,
            )
            .unwrap();
    }

    #[test]
    fn clocks_validation_and_future_claim() {
        let store = Store::open_in_memory().unwrap();
        job(&store, "a", "scheduled", 200);
        assert!(!store.claim_scheduled_send("a", 0, 199).unwrap());
        assert!(store.claim_scheduled_send("a", 0, 200).unwrap());
        assert!(!store.restore_scheduled_send("a", 0, 200).unwrap());
        assert!(!store.restore_scheduled_send("a", 1, 200).unwrap());
        assert!(!store.finish_scheduled_send("a", 0, true, 201).unwrap());
        assert!(store.finish_scheduled_send("a", 1, false, 201).unwrap());
        assert!(!store.reschedule_send("a", 2, 300, 201).unwrap());
        assert!(!store.claim_scheduled_send("a", 2, 400).unwrap());
        assert!(validate_future(201, 201).is_err());
        assert!(validate_future(i64::MAX, 201).is_err());
    }

    #[test]
    fn cancellation_and_worker_race_has_exactly_one_winner() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        job(&store, "a", "undo", 200);
        let barrier = Arc::new(Barrier::new(3));
        let workers: Vec<_> = (0..2)
            .map(|i| {
                let store = store.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    if i == 0 {
                        store.claim_scheduled_send("a", 0, 200).unwrap()
                    } else {
                        store.restore_scheduled_send("a", 0, 200).unwrap()
                    }
                })
            })
            .collect();
        barrier.wait();
        assert_eq!(
            workers
                .into_iter()
                .filter(|w| w.thread().id() != std::thread::current().id())
                .map(|w| usize::from(w.join().unwrap()))
                .sum::<usize>(),
            1
        );
    }

    #[test]
    fn durable_restart_requires_review_without_shifting_utc() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.db");
        {
            let store = Store::open(&path).unwrap();
            job(&store, "late", "scheduled", 200);
            job(&store, "future", "scheduled", 500);
            job(&store, "undo", "undo", 500);
            job(&store, "flight", "scheduled", 200);
            assert!(store.claim_scheduled_send("flight", 0, 200).unwrap());
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.recover_scheduled_sends(300).unwrap(), 3);
        let future = store.get_scheduled_send("future").unwrap().unwrap();
        assert_eq!(future.due_at, 500);
        assert_eq!(future.payload, vec![1, 2, 3]);
        assert_eq!(future.state, "pending");
        for id in ["late", "undo", "flight"] {
            assert_eq!(
                store.get_scheduled_send(id).unwrap().unwrap().state,
                "review"
            );
        }
        assert_eq!(store.recover_scheduled_sends(300).unwrap(), 0);
    }

    #[test]
    fn edits_require_cancel_and_current_revision_and_keep_payload() {
        let store = Store::open_in_memory().unwrap();
        job(&store, "a", "scheduled", 200);
        assert!(store.restore_scheduled_send("a", 0, 150).unwrap());
        assert!(!store.claim_scheduled_send("a", 0, 500).unwrap());
        assert!(!store.reschedule_send("a", 0, 400, 200).unwrap());
        assert!(
            !store.reschedule_send("a", 1, 400, 200).unwrap(),
            "a restored backup must never compete with the editable compose"
        );
        assert_eq!(
            store.get_scheduled_send("a").unwrap().unwrap().payload,
            vec![1, 2, 3]
        );
        assert!(!store.delete_scheduled_send("a", 2).unwrap());
        assert!(store.delete_scheduled_send("a", 1).unwrap());
    }
}
