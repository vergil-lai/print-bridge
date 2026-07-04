use rusqlite::{params, Connection, OptionalExtension};
use std::{path::Path, sync::Mutex};
use uuid::Uuid;

#[derive(Debug)]
pub struct RemoteStore {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteReportStatus {
    Accepted,
    Success,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteDeliveryState {
    Pending,
    Delivered,
    Abandoned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryFailureOutcome {
    WillRetry,
    Abandoned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRemoteJob<'a> {
    pub request_id: &'a str,
    pub batch_id: Option<&'a str>,
    pub job_id: &'a str,
    pub first_seen_at: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRemoteStatusEvent<'a> {
    pub job_id: &'a str,
    pub request_id: &'a str,
    pub batch_id: Option<&'a str>,
    pub status: RemoteReportStatus,
    pub message: Option<&'a str>,
    pub occurred_at: &'a str,
    pub next_retry_at: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteStatusEvent {
    pub event_id: String,
    pub job_id: String,
    pub request_id: String,
    pub batch_id: Option<String>,
    pub status: RemoteReportStatus,
    pub message: Option<String>,
    pub occurred_at: String,
    pub delivery_state: RemoteDeliveryState,
    pub retry_count: u32,
    pub next_retry_at: String,
    pub last_error: Option<String>,
}

impl RemoteStore {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        Self::from_connection(conn)
    }

    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> rusqlite::Result<Self> {
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.initialize_schema()?;
        Ok(store)
    }

    pub fn record_job_if_new(&self, job: &NewRemoteJob<'_>) -> rusqlite::Result<bool> {
        let conn = self.conn.lock().expect("remote store mutex poisoned");
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO remote_jobs (
                job_id, request_id, batch_id, status, first_seen_at, updated_at
            ) VALUES (?1, ?2, ?3, 'queued', ?4, ?4)",
            params![job.job_id, job.request_id, job.batch_id, job.first_seen_at],
        )?;

        Ok(inserted == 1)
    }

    pub fn insert_status_event(
        &self,
        event: &NewRemoteStatusEvent<'_>,
    ) -> rusqlite::Result<Option<RemoteStatusEvent>> {
        let event_id = Uuid::new_v4().to_string();
        let conn = self.conn.lock().expect("remote store mutex poisoned");
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO remote_status_events (
                event_id, job_id, request_id, batch_id, status, message, occurred_at,
                delivery_state, delivered_at, retry_count, next_retry_at, last_error
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', NULL, 0, ?8, NULL)",
            params![
                event_id,
                event.job_id,
                event.request_id,
                event.batch_id,
                event.status.as_str(),
                event.message,
                event.occurred_at,
                event.next_retry_at
            ],
        )?;

        if inserted == 0 {
            return Ok(None);
        }

        Self::find_status_event_locked(&conn, &event_id)
    }

    pub fn pending_status_events(
        &self,
        now: &str,
        limit: u32,
    ) -> rusqlite::Result<Vec<RemoteStatusEvent>> {
        let conn = self.conn.lock().expect("remote store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT
                event_id, job_id, request_id, batch_id, status, message, occurred_at,
                delivery_state, retry_count, next_retry_at, last_error
            FROM remote_status_events
            WHERE delivery_state = 'pending' AND next_retry_at <= ?1
            ORDER BY occurred_at ASC, event_id ASC
            LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![now, limit], Self::row_to_status_event)?;

        rows.collect()
    }

    pub fn mark_delivered(&self, event_id: &str, delivered_at: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("remote store mutex poisoned");
        conn.execute(
            "UPDATE remote_status_events
            SET delivery_state = 'delivered', delivered_at = ?2, last_error = NULL
            WHERE event_id = ?1",
            params![event_id, delivered_at],
        )?;
        Ok(())
    }

    pub fn mark_delivery_failed(
        &self,
        event_id: &str,
        next_retry_at: &str,
        error: &str,
        max_retries: u32,
    ) -> rusqlite::Result<DeliveryFailureOutcome> {
        let conn = self.conn.lock().expect("remote store mutex poisoned");
        let retry_count: u32 = conn.query_row(
            "SELECT retry_count FROM remote_status_events WHERE event_id = ?1",
            params![event_id],
            |row| row.get(0),
        )?;
        let next_retry_count = retry_count.saturating_add(1);
        let outcome = if next_retry_count >= max_retries {
            DeliveryFailureOutcome::Abandoned
        } else {
            DeliveryFailureOutcome::WillRetry
        };
        let delivery_state = match outcome {
            DeliveryFailureOutcome::WillRetry => RemoteDeliveryState::Pending,
            DeliveryFailureOutcome::Abandoned => RemoteDeliveryState::Abandoned,
        };

        conn.execute(
            "UPDATE remote_status_events
            SET retry_count = ?2, next_retry_at = ?3, last_error = ?4, delivery_state = ?5
            WHERE event_id = ?1",
            params![
                event_id,
                next_retry_count,
                next_retry_at,
                error,
                delivery_state.as_str()
            ],
        )?;

        Ok(outcome)
    }

    pub fn cleanup_delivered_before(&self, cutoff: &str) -> rusqlite::Result<usize> {
        let conn = self.conn.lock().expect("remote store mutex poisoned");
        conn.execute(
            "DELETE FROM remote_status_events
            WHERE delivery_state = 'delivered' AND delivered_at < ?1",
            params![cutoff],
        )
    }

    fn initialize_schema(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("remote store mutex poisoned");
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS remote_jobs (
                job_id TEXT PRIMARY KEY,
                request_id TEXT NOT NULL,
                batch_id TEXT NULL,
                status TEXT NOT NULL,
                first_seen_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS remote_status_events (
                event_id TEXT PRIMARY KEY,
                job_id TEXT NOT NULL,
                request_id TEXT NOT NULL,
                batch_id TEXT NULL,
                status TEXT NOT NULL,
                message TEXT NULL,
                occurred_at TEXT NOT NULL,
                delivery_state TEXT NOT NULL DEFAULT 'pending',
                delivered_at TEXT NULL,
                retry_count INTEGER NOT NULL DEFAULT 0,
                next_retry_at TEXT NOT NULL,
                last_error TEXT NULL
            );

            CREATE UNIQUE INDEX IF NOT EXISTS idx_remote_status_events_job_status
            ON remote_status_events (job_id, status);

            CREATE INDEX IF NOT EXISTS idx_remote_status_events_pending
            ON remote_status_events (delivery_state, next_retry_at);
            ",
        )?;
        Ok(())
    }

    fn find_status_event_locked(
        conn: &Connection,
        event_id: &str,
    ) -> rusqlite::Result<Option<RemoteStatusEvent>> {
        conn.query_row(
            "SELECT
                event_id, job_id, request_id, batch_id, status, message, occurred_at,
                delivery_state, retry_count, next_retry_at, last_error
            FROM remote_status_events
            WHERE event_id = ?1",
            params![event_id],
            Self::row_to_status_event,
        )
        .optional()
    }

    fn row_to_status_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<RemoteStatusEvent> {
        let status: String = row.get(4)?;
        let delivery_state: String = row.get(7)?;

        Ok(RemoteStatusEvent {
            event_id: row.get(0)?,
            job_id: row.get(1)?,
            request_id: row.get(2)?,
            batch_id: row.get(3)?,
            status: RemoteReportStatus::from_str(&status)?,
            message: row.get(5)?,
            occurred_at: row.get(6)?,
            delivery_state: RemoteDeliveryState::from_str(&delivery_state)?,
            retry_count: row.get(8)?,
            next_retry_at: row.get(9)?,
            last_error: row.get(10)?,
        })
    }
}

impl RemoteReportStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Success => "success",
            Self::Failed => "failed",
        }
    }

    fn from_str(value: &str) -> rusqlite::Result<Self> {
        match value {
            "accepted" => Ok(Self::Accepted),
            "success" => Ok(Self::Success),
            "failed" => Ok(Self::Failed),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }
}

impl RemoteDeliveryState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Delivered => "delivered",
            Self::Abandoned => "abandoned",
        }
    }

    fn from_str(value: &str) -> rusqlite::Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "delivered" => Ok(Self::Delivered),
            "abandoned" => Ok(Self::Abandoned),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }
}
