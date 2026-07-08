use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};

/// 本地任务历史 SQLite 存储。
#[derive(Debug)]
pub struct TaskHistoryStore {
    conn: Mutex<Connection>,
}

/// 本地任务历史记录中的任务状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskHistoryStatus {
    Queued,
    Downloading,
    Printing,
    Submitted,
    Completed,
    Failed,
    Unknown,
    Cancelled,
}

/// 本地任务历史记录中的任务来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskHistorySource {
    WebSocket,
    Remote,
    Test,
}

/// 任务历史列表中的单个任务摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskHistoryJob {
    pub job_id: String,
    pub request_id: Option<String>,
    pub batch_id: Option<String>,
    pub source: TaskHistorySource,
    pub current_status: TaskHistoryStatus,
    pub current_message: Option<String>,
    pub printer_name: Option<String>,
    pub paper_name: Option<String>,
    pub copies: Option<u16>,
    pub created_at: String,
    pub updated_at: String,
    pub finished_at: Option<String>,
}

/// 任务历史中的单个状态事件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskHistoryEvent {
    pub id: i64,
    pub job_id: String,
    pub status: TaskHistoryStatus,
    pub message: Option<String>,
    pub occurred_at: String,
}

/// 写入任务历史时使用的新事件输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTaskHistoryEvent<'a> {
    pub job_id: &'a str,
    pub request_id: Option<&'a str>,
    pub batch_id: Option<&'a str>,
    pub source: TaskHistorySource,
    pub status: TaskHistoryStatus,
    pub message: Option<&'a str>,
    pub printer_name: Option<&'a str>,
    pub paper_name: Option<&'a str>,
    pub copies: Option<u16>,
    pub occurred_at: &'a str,
}

impl TaskHistoryStore {
    /// 打开或创建磁盘上的任务历史数据库。
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        Self::from_connection(conn)
    }

    /// 打开仅用于测试或临时运行的内存任务历史数据库。
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

    fn lock_conn(&self) -> rusqlite::Result<MutexGuard<'_, Connection>> {
        self.conn.lock().map_err(|_| rusqlite::Error::InvalidQuery)
    }

    /// 记录任务状态事件，并同步更新任务摘要。
    pub fn record_event(&self, event: &NewTaskHistoryEvent<'_>) -> rusqlite::Result<()> {
        let mut conn = self.lock_conn()?;
        let transaction = conn.transaction()?;
        let finished_at = if event.status.is_terminal() {
            Some(event.occurred_at)
        } else {
            None
        };

        transaction.execute(
            "INSERT INTO task_history_events (
                job_id, status, message, occurred_at
            ) VALUES (?1, ?2, ?3, ?4)",
            params![
                event.job_id,
                event.status.as_str(),
                event.message,
                event.occurred_at
            ],
        )?;

        transaction.execute(
            "INSERT INTO task_history_jobs (
                job_id, request_id, batch_id, source, current_status, current_message,
                printer_name, paper_name, copies, created_at, updated_at, finished_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, ?11)
            ON CONFLICT(job_id) DO UPDATE SET
                request_id = COALESCE(excluded.request_id, task_history_jobs.request_id),
                batch_id = COALESCE(excluded.batch_id, task_history_jobs.batch_id),
                source = excluded.source,
                current_status = excluded.current_status,
                current_message = excluded.current_message,
                printer_name = COALESCE(excluded.printer_name, task_history_jobs.printer_name),
                paper_name = COALESCE(excluded.paper_name, task_history_jobs.paper_name),
                copies = COALESCE(excluded.copies, task_history_jobs.copies),
                updated_at = excluded.updated_at,
                finished_at = COALESCE(excluded.finished_at, task_history_jobs.finished_at)",
            params![
                event.job_id,
                event.request_id,
                event.batch_id,
                event.source.as_str(),
                event.status.as_str(),
                event.message,
                event.printer_name,
                event.paper_name,
                event.copies,
                event.occurred_at,
                finished_at,
            ],
        )?;

        transaction.commit()
    }

    /// 按更新时间倒序读取最近任务摘要。
    pub fn recent_jobs(&self, limit: u32) -> rusqlite::Result<Vec<TaskHistoryJob>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT
                job_id, request_id, batch_id, source, current_status, current_message,
                printer_name, paper_name, copies, created_at, updated_at, finished_at
            FROM task_history_jobs
            ORDER BY updated_at DESC, job_id DESC
            LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], Self::row_to_job)?;

        rows.collect()
    }

    /// 读取指定任务的所有状态事件。
    pub fn events_for_job(&self, job_id: &str) -> rusqlite::Result<Vec<TaskHistoryEvent>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, job_id, status, message, occurred_at
            FROM task_history_events
            WHERE job_id = ?1
            ORDER BY occurred_at ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![job_id], Self::row_to_event)?;

        rows.collect()
    }

    /// 清空本地任务历史。
    pub fn clear(&self) -> rusqlite::Result<()> {
        let mut conn = self.lock_conn()?;
        let transaction = conn.transaction()?;
        transaction.execute("DELETE FROM task_history_events", [])?;
        transaction.execute("DELETE FROM task_history_jobs", [])?;
        transaction.commit()
    }

    fn initialize_schema(&self) -> rusqlite::Result<()> {
        let conn = self.lock_conn()?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS task_history_jobs (
                job_id TEXT PRIMARY KEY,
                request_id TEXT NULL,
                batch_id TEXT NULL,
                source TEXT NOT NULL,
                current_status TEXT NOT NULL,
                current_message TEXT NULL,
                printer_name TEXT NULL,
                paper_name TEXT NULL,
                copies INTEGER NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                finished_at TEXT NULL
            );

            CREATE TABLE IF NOT EXISTS task_history_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                job_id TEXT NOT NULL,
                status TEXT NOT NULL,
                message TEXT NULL,
                occurred_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_task_history_jobs_updated_at
            ON task_history_jobs (updated_at);

            CREATE INDEX IF NOT EXISTS idx_task_history_events_job_id
            ON task_history_events (job_id, occurred_at);
            ",
        )?;
        Ok(())
    }

    fn row_to_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskHistoryJob> {
        let source: String = row.get(3)?;
        let current_status: String = row.get(4)?;

        Ok(TaskHistoryJob {
            job_id: row.get(0)?,
            request_id: row.get(1)?,
            batch_id: row.get(2)?,
            source: TaskHistorySource::from_str(&source)?,
            current_status: TaskHistoryStatus::from_str(&current_status)?,
            current_message: row.get(5)?,
            printer_name: row.get(6)?,
            paper_name: row.get(7)?,
            copies: row.get(8)?,
            created_at: row.get(9)?,
            updated_at: row.get(10)?,
            finished_at: row.get(11)?,
        })
    }

    fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskHistoryEvent> {
        let status: String = row.get(2)?;

        Ok(TaskHistoryEvent {
            id: row.get(0)?,
            job_id: row.get(1)?,
            status: TaskHistoryStatus::from_str(&status)?,
            message: row.get(3)?,
            occurred_at: row.get(4)?,
        })
    }
}

impl TaskHistoryStatus {
    /// 返回写入数据库和 JSON 时使用的状态字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Downloading => "downloading",
            Self::Printing => "printing",
            Self::Submitted => "submitted",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
            Self::Cancelled => "cancelled",
        }
    }

    fn from_str(value: &str) -> rusqlite::Result<Self> {
        match value {
            "queued" => Ok(Self::Queued),
            "downloading" => Ok(Self::Downloading),
            "printing" => Ok(Self::Printing),
            "submitted" => Ok(Self::Submitted),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "unknown" => Ok(Self::Unknown),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }

    fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Submitted | Self::Completed | Self::Failed | Self::Unknown | Self::Cancelled
        )
    }
}

impl TaskHistorySource {
    /// 返回写入数据库和 JSON 时使用的来源字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WebSocket => "web_socket",
            Self::Remote => "remote",
            Self::Test => "test",
        }
    }

    fn from_str(value: &str) -> rusqlite::Result<Self> {
        match value {
            "web_socket" => Ok(Self::WebSocket),
            "remote" => Ok(Self::Remote),
            "test" => Ok(Self::Test),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event<'a>(
        job_id: &'a str,
        status: TaskHistoryStatus,
        occurred_at: &'a str,
    ) -> NewTaskHistoryEvent<'a> {
        NewTaskHistoryEvent {
            job_id,
            request_id: Some("request-1"),
            batch_id: Some("batch-1"),
            source: TaskHistorySource::WebSocket,
            status,
            message: Some("queued"),
            printer_name: Some("Office Printer"),
            paper_name: Some("A4"),
            copies: Some(2),
            occurred_at,
        }
    }

    #[test]
    fn upserts_job_summary_when_event_is_recorded() {
        let store = TaskHistoryStore::open_in_memory().unwrap();
        store
            .record_event(&event(
                "job-1",
                TaskHistoryStatus::Queued,
                "2026-07-06T10:00:00Z",
            ))
            .unwrap();

        store
            .record_event(&NewTaskHistoryEvent {
                job_id: "job-1",
                request_id: None,
                batch_id: None,
                source: TaskHistorySource::WebSocket,
                status: TaskHistoryStatus::Printing,
                message: Some("printing"),
                printer_name: None,
                paper_name: None,
                copies: None,
                occurred_at: "2026-07-06T10:01:00Z",
            })
            .unwrap();

        let jobs = store.recent_jobs(10).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].job_id, "job-1");
        assert_eq!(jobs[0].request_id.as_deref(), Some("request-1"));
        assert_eq!(jobs[0].batch_id.as_deref(), Some("batch-1"));
        assert_eq!(jobs[0].current_status, TaskHistoryStatus::Printing);
        assert_eq!(jobs[0].current_message.as_deref(), Some("printing"));
        assert_eq!(jobs[0].printer_name.as_deref(), Some("Office Printer"));
        assert_eq!(jobs[0].paper_name.as_deref(), Some("A4"));
        assert_eq!(jobs[0].copies, Some(2));
        assert_eq!(jobs[0].created_at, "2026-07-06T10:00:00Z");
        assert_eq!(jobs[0].updated_at, "2026-07-06T10:01:00Z");

        let events = store.events_for_job("job-1").unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].status, TaskHistoryStatus::Queued);
        assert_eq!(events[1].status, TaskHistoryStatus::Printing);
    }

    #[test]
    fn terminal_status_sets_finished_at() {
        let store = TaskHistoryStore::open_in_memory().unwrap();
        store
            .record_event(&event(
                "job-1",
                TaskHistoryStatus::Queued,
                "2026-07-06T10:00:00Z",
            ))
            .unwrap();
        store
            .record_event(&NewTaskHistoryEvent {
                job_id: "job-1",
                request_id: None,
                batch_id: None,
                source: TaskHistorySource::WebSocket,
                status: TaskHistoryStatus::Completed,
                message: Some("completed"),
                printer_name: None,
                paper_name: None,
                copies: None,
                occurred_at: "2026-07-06T10:03:00Z",
            })
            .unwrap();

        let jobs = store.recent_jobs(10).unwrap();
        assert_eq!(jobs[0].finished_at.as_deref(), Some("2026-07-06T10:03:00Z"));
    }

    #[test]
    fn later_terminal_status_updates_finished_at() {
        let store = TaskHistoryStore::open_in_memory().unwrap();
        store
            .record_event(&event(
                "job-1",
                TaskHistoryStatus::Submitted,
                "2026-07-06T10:02:00Z",
            ))
            .unwrap();
        store
            .record_event(&event(
                "job-1",
                TaskHistoryStatus::Completed,
                "2026-07-06T10:05:00Z",
            ))
            .unwrap();

        let jobs = store.recent_jobs(10).unwrap();
        assert_eq!(jobs[0].finished_at.as_deref(), Some("2026-07-06T10:05:00Z"));
    }

    #[test]
    fn all_terminal_statuses_set_finished_at() {
        for status in [
            TaskHistoryStatus::Submitted,
            TaskHistoryStatus::Completed,
            TaskHistoryStatus::Failed,
            TaskHistoryStatus::Unknown,
            TaskHistoryStatus::Cancelled,
        ] {
            let store = TaskHistoryStore::open_in_memory().unwrap();
            store
                .record_event(&event("job-1", status, "2026-07-06T10:03:00Z"))
                .unwrap();

            let jobs = store.recent_jobs(10).unwrap();
            assert_eq!(jobs[0].finished_at.as_deref(), Some("2026-07-06T10:03:00Z"));
        }
    }

    #[test]
    fn source_literals_match_snake_case_serialization() {
        assert_eq!(TaskHistorySource::WebSocket.as_str(), "web_socket");
        assert_eq!(
            serde_json::to_string(&TaskHistorySource::WebSocket).unwrap(),
            "\"web_socket\""
        );
    }

    #[test]
    fn clear_removes_jobs_and_events() {
        let store = TaskHistoryStore::open_in_memory().unwrap();
        store
            .record_event(&event(
                "job-1",
                TaskHistoryStatus::Queued,
                "2026-07-06T10:00:00Z",
            ))
            .unwrap();

        store.clear().unwrap();

        assert!(store.recent_jobs(10).unwrap().is_empty());
        assert!(store.events_for_job("job-1").unwrap().is_empty());
    }
}
