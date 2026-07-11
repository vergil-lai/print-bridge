use std::collections::VecDeque;

pub use print_bridge_core::activity::TaskLogEntry;

/// 内存任务日志默认保留条数。
pub const DEFAULT_LOG_CAPACITY: usize = 500;

/// 固定容量的内存日志存储，只保留最近任务记录。
#[derive(Debug, Clone)]
pub struct LogStore {
    capacity: usize,
    entries: VecDeque<TaskLogEntry>,
}

impl Default for LogStore {
    /// 使用默认容量创建日志存储。
    fn default() -> Self {
        Self::with_capacity(DEFAULT_LOG_CAPACITY)
    }
}

impl LogStore {
    /// 使用指定最大条数创建日志存储。
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity,
            entries: VecDeque::new(),
        }
    }

    /// 添加日志记录，并在超出容量时丢弃最旧记录。
    pub fn push(&mut self, entry: TaskLogEntry) {
        self.entries.push_back(entry);

        while self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
    }

    /// 按从旧到新的顺序返回日志记录。
    pub fn recent(&self) -> Vec<TaskLogEntry> {
        self.entries.iter().cloned().collect()
    }

    /// 清空当前保留的内存日志。
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}
