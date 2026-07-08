import { invoke } from '@tauri-apps/api/core';
import type {
  AgentConfig,
  ExportConfigOptions,
  ImportPreview,
  PaperInfo,
  PrinterInfo,
  TaskHistoryEvent,
  TaskHistoryJob,
  TaskLogEntry,
} from '@/types';

interface PapersResponse {
  papers: PaperInfo[];
  supports_custom: boolean;
}

/** 构造访问本地 Agent HTTP 服务的回环地址。 */
function localUrl(port: number, path: string): string {
  return `http://127.0.0.1:${port}${path}`;
}

/** 从本地服务读取 JSON，并把非 2xx 响应转成错误。 */
async function fetchJson<T>(url: string): Promise<T> {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`Request failed: ${response.status}`);
  }

  return response.json() as Promise<T>;
}

/** 通过 Tauri 读取已持久化的 Agent 配置。 */
export function getConfig(): Promise<AgentConfig> {
  return invoke<AgentConfig>('get_config');
}

/** 通过 Tauri 保存 Agent 配置。 */
export function saveConfig(config: AgentConfig): Promise<AgentConfig> {
  return invoke<AgentConfig>('save_config', { config });
}

/** 导出加密配置文件。 */
export function exportConfigFile(
  path: string,
  password: string,
  options: ExportConfigOptions,
): Promise<void> {
  return invoke<void>('export_config_file', { path, password, options });
}

/** 预览加密配置文件导入后的变更。 */
export function previewConfigImport(path: string, password: string): Promise<ImportPreview> {
  return invoke<ImportPreview>('preview_config_import', { path, password });
}

/** 导入加密配置文件并返回保存后的 Agent 配置。 */
export function importConfigFile(
  path: string,
  password: string,
  expectedFileHash: string,
): Promise<AgentConfig> {
  return invoke<AgentConfig>('import_config_file', { path, password, expectedFileHash });
}

/** 使用当前远程任务配置执行连接测试。 */
export function testRemoteConnection(config: AgentConfig): Promise<void> {
  return invoke<void>('test_remote_connection', { config });
}

/** 读取当前桌面应用是否为 debug 构建。 */
export function isDebugBuild(): Promise<boolean> {
  return invoke<boolean>('is_debug_build');
}

/** 读取本地 Agent 保留的最近任务日志。 */
export function getLogs(): Promise<TaskLogEntry[]> {
  return invoke<TaskLogEntry[]>('get_logs');
}

/** 读取最近任务历史。 */
export function getTaskHistory(): Promise<TaskHistoryJob[]> {
  return invoke<TaskHistoryJob[]>('get_task_history');
}

/** 读取指定任务的历史事件。 */
export function getTaskHistoryEvents(jobId: string): Promise<TaskHistoryEvent[]> {
  return invoke<TaskHistoryEvent[]>('get_task_history_events', { jobId });
}

/** 清空本地任务历史。 */
export function clearTaskHistory(): Promise<void> {
  return invoke<void>('clear_task_history');
}

/** 从本地 HTTP 服务读取打印机列表。 */
export function fetchPrinters(port: number): Promise<PrinterInfo[]> {
  return fetchJson<PrinterInfo[]>(localUrl(port, '/printers'));
}

/** 从本地 HTTP 服务读取指定打印机的纸张尺寸。 */
export async function fetchPapers(port: number, printerName: string): Promise<PaperInfo[]> {
  const encodedPrinter = encodeURIComponent(printerName);
  const response = await fetchJson<PapersResponse>(
    localUrl(port, `/printers/${encodedPrinter}/papers`),
  );

  return response.papers;
}

/** 使用当前默认打印设置提交一张测试校准页。 */
export function printTestPage(config: AgentConfig): Promise<void> {
  return invoke<void>('print_test', { config });
}
