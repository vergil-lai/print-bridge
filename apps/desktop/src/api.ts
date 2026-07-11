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

/** 通过共享命令服务读取打印机列表。 */
export function fetchPrinters(): Promise<PrinterInfo[]> {
  return invoke<PrinterInfo[]>('list_printers');
}

/** 通过共享命令服务读取指定打印机的纸张尺寸。 */
export function fetchPapers(printerName: string): Promise<PaperInfo[]> {
  return invoke<PaperInfo[]>('list_papers', { printerName });
}

/** 使用当前默认打印设置提交一张测试校准页。 */
export function printTestPage(config: AgentConfig): Promise<void> {
  return invoke<void>('print_test', { config });
}
