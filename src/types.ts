/** Agent 配置中保存的实际纸张尺寸。 */
export interface EffectivePaper {
  width_mm: number;
  height_mm: number;
}

/** Tauri UI 和本地 Agent 共享的完整配置。 */
export interface AgentConfig {
  service: {
    host: string;
    port: number;
  };
  security: {
    allowed_origins: string[];
  };
  printing: {
    default_printer: string | null;
    default_paper: EffectivePaper | null;
    default_copies: number;
  };
  limits: {
    max_file_size_mb: number;
    max_batch_jobs: number;
    max_copies: number;
    download_timeout_seconds: number;
  };
  app: {
    autostart: boolean;
  };
  remote: {
    enabled: boolean;
    endpoint_url: string | null;
    bearer_token: string | null;
    device_id: string | null;
    device_name: string | null;
    poll_interval_seconds: number;
    max_report_retries: number;
    history_retention_days: number;
  };
}

/** 本地服务返回的打印机摘要。 */
export interface PrinterInfo {
  name: string;
  is_default: boolean;
}

/** 本地服务返回的纸张尺寸摘要。 */
export interface PaperInfo {
  id: string;
  name: string;
  width_mm: number;
  height_mm: number;
}

/** 桌面 UI 展示的最近任务日志记录。 */
export interface TaskLogEntry {
  timestamp: string;
  request_id: string | null;
  batch_id: string | null;
  job_id: string | null;
  origin: string | null;
  status: string;
  message: string;
}

export type TaskHistoryStatus =
  | 'queued'
  | 'downloading'
  | 'printing'
  | 'submitted'
  | 'completed'
  | 'failed'
  | 'unknown'
  | 'cancelled';

export type TaskHistorySource = 'web_socket' | 'remote' | 'test';

export interface TaskHistoryJob {
  job_id: string;
  request_id: string | null;
  batch_id: string | null;
  source: TaskHistorySource;
  current_status: TaskHistoryStatus;
  current_message: string | null;
  printer_name: string | null;
  paper_name: string | null;
  copies: number | null;
  created_at: string;
  updated_at: string;
  finished_at: string | null;
}

export interface TaskHistoryEvent {
  id: number;
  job_id: string;
  status: TaskHistoryStatus;
  message: string | null;
  occurred_at: string;
}
