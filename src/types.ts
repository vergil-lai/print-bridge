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
