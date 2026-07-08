<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef } from 'vue';
import {
  Download,
  ExternalLink,
  FileDown,
  FileUp,
  Monitor,
  Moon,
  Printer,
  RefreshCw,
  RotateCw,
  Save,
  Shuffle,
  Sun,
  Trash2,
  X,
} from '@lucide/vue';
import { getVersion } from '@tauri-apps/api/app';
import { open as openDialog, save as saveDialog } from '@tauri-apps/plugin-dialog';
import { openUrl } from '@tauri-apps/plugin-opener';
import appIcon from '@/assets/app-icon.png';
import {
  exportConfigFile,
  fetchPapers,
  fetchPrinters,
  clearTaskHistory,
  getConfig,
  getTaskHistory,
  getTaskHistoryEvents,
  importConfigFile,
  isDebugBuild,
  printTestPage,
  previewConfigImport,
  saveConfig,
  testRemoteConnection,
} from '@/api';
import type {
  AgentConfig,
  EffectivePaper,
  ExportConfigOptions,
  ImportPreview,
  PaperInfo,
  PrinterInfo,
  TaskHistoryEvent,
  TaskHistoryJob,
  TaskHistorySource,
  TaskHistoryStatus,
} from '@/types';
import {
  checkForAppUpdate,
  downloadAndInstallAppUpdate,
  relaunchApp,
  toUpdateInfo,
  type UpdateInfo,
  type UpdateProgress,
  type UpdateStatus,
} from '@/updater';
import type { Update } from '@tauri-apps/plugin-updater';
import { Alert, AlertDescription } from '@/components/ui/alert';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { ScrollArea } from '@/components/ui/scroll-area';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { Separator } from '@/components/ui/separator';
import { Switch } from '@/components/ui/switch';
import {
  Table,
  TableBody,
  TableCell,
  TableEmpty,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs';

const DEFAULT_PAPER: EffectivePaper = {
  width_mm: 60,
  height_mm: 40,
};
const DEFAULT_REMOTE_CONFIG: AgentConfig['remote'] = {
  enabled: false,
  endpoint_url: null,
  bearer_token: null,
  device_id: null,
  device_name: null,
  poll_interval_seconds: 10,
  max_report_retries: 10,
  history_retention_days: 3,
};

const GITHUB_REPOSITORY_URL = 'https://github.com/vergil-lai/print-bridge';
const GITHUB_RELEASES_URL = 'https://github.com/vergil-lai/print-bridge/releases';
const MIN_SERVICE_PORT = 10000;
const MAX_SERVICE_PORT = 65535;
const MIN_REMOTE_POLL_INTERVAL_SECONDS = 3;
const MIN_REMOTE_MAX_REPORT_RETRIES = 1;
const THEME_STORAGE_KEY = 'printbridge.theme';
type ThemeMode = 'system' | 'light' | 'dark';

const config = ref<AgentConfig | null>(null);
const printers = ref<PrinterInfo[]>([]);
const papers = ref<PaperInfo[]>([]);
const taskHistory = ref<TaskHistoryJob[]>([]);
const selectedTaskJobId = ref<string | null>(null);
const selectedTaskEvents = ref<TaskHistoryEvent[]>([]);
const themeMode = ref<ThemeMode>(readThemeMode());
const originDraft = ref('');
const originErrorMessage = ref('');
const errorMessage = ref('');
const successMessage = ref('');
const loadingConfig = ref(true);
const loadingPrinters = ref(false);
const loadingPapers = ref(false);
const loadingTaskHistory = ref(false);
const loadingTaskEvents = ref(false);
const clearingTaskHistory = ref(false);
const confirmingClearTaskHistory = ref(false);
const saving = ref(false);
const exportingConfig = ref(false);
const importingConfig = ref(false);
const previewingConfigImport = ref(false);
const showExportDialog = ref(false);
const showImportDialog = ref(false);
const showEmptyPasswordTokenConfirm = ref(false);
const exportPassword = ref('');
const importPassword = ref('');
const importPath = ref('');
const importPreview = ref<ImportPreview | null>(null);
const importErrorMessage = ref('');
const exportOptions = ref<ExportConfigOptions>(defaultExportOptions());
const testingPrint = ref(false);
const testingRemote = ref(false);
const activePort = ref<number | null>(null);
const appVersion = ref('-');
const updateStatus = ref<UpdateStatus>('idle');
const updateMessage = ref('');
const availableUpdate = shallowRef<Update | null>(null);
const updateInfo = ref<UpdateInfo | null>(null);
const updateProgress = ref<UpdateProgress>({
  downloadedBytes: 0,
  contentLength: null,
});
let colorSchemeQuery: MediaQueryList | null = null;
let taskHistoryRequestId = 0;
let taskEventsRequestId = 0;

/** UI 请求当前使用的本地服务端口。 */
const servicePort = computed(() => activePort.value ?? config.value?.service.port ?? 0);
/** 保存的配置端口是否还未在当前服务中生效。 */
const hasPendingPortChange = computed(
  () =>
    activePort.value !== null &&
    config.value !== null &&
    config.value.service.port !== activePort.value,
);
/** 顶部状态栏显示的可读状态。 */
const statusLabel = computed(() => {
  if (loadingConfig.value) return '加载中';
  if (errorMessage.value) return '需处理';
  return '已就绪';
});
/** 根据当前错误状态计算状态徽标样式。 */
const statusVariant = computed(() => (errorMessage.value ? 'destructive' : 'secondary'));
/** 更新器当前是否正在检查新版本。 */
const checkingUpdate = computed(() => updateStatus.value === 'checking');
/** 当前是否正在下载或安装更新。 */
const installingUpdate = computed(() => updateStatus.value === 'downloading');
/** 更新安装下载进度百分比。 */
const updateProgressPercent = computed(() => {
  const contentLength = updateProgress.value.contentLength;
  if (!contentLength) return 0;

  return Math.min(100, Math.round((updateProgress.value.downloadedBytes / contentLength) * 100));
});
/** 更新安装时显示的下载进度文本。 */
const updateProgressText = computed(() => {
  const { downloadedBytes, contentLength } = updateProgress.value;
  if (!contentLength) return formatBytes(downloadedBytes);

  return `${formatBytes(downloadedBytes)} / ${formatBytes(contentLength)}`;
});
/** 更新面板中显示的当前版本。 */
const currentAppVersion = computed(() => updateInfo.value?.currentVersion ?? appVersion.value);
/** 更新面板中显示的可用版本。 */
const availableUpdateVersion = computed(() => updateInfo.value?.version ?? '-');
/** 根据当前更新状态生成主按钮文案。 */
const updateButtonLabel = computed(() => {
  if (installingUpdate.value) return '更新中';
  if (updateInfo.value?.version) return `更新到 v${updateInfo.value.version}`;

  return '下载并安装';
});
/** 当前配置是否足够提交测试打印。 */
const canTestPrint = computed(
  () =>
    Boolean(config.value?.printing.default_printer) &&
    Boolean(config.value?.printing.default_paper),
);
/** 默认打印机选择项的双向计算值。 */
const selectedPrinter = computed({
  get: () => config.value?.printing.default_printer ?? '',
  set: (value: string) => {
    if (!config.value) return;
    config.value.printing.default_printer = value || null;
  },
});

/** 纸张预设或自定义纸张选择项的双向计算值。 */
const selectedPaper = computed({
  get: () => {
    const currentPaper = config.value?.printing.default_paper;
    if (!currentPaper) return 'custom';

    return matchingPaper(currentPaper)?.id ?? 'custom';
  },
  set: (paperId: string) => {
    const paper = papers.value.find((item) => item.id === paperId);
    if (!paper || !config.value) return;

    config.value.printing.default_paper = {
      width_mm: paper.width_mm,
      height_mm: paper.height_mm,
    };
  },
});
/** 当前选中的任务摘要。 */
const selectedTask = computed(
  () => taskHistory.value.find((item) => item.job_id === selectedTaskJobId.value) ?? null,
);

/** 确保加载后的配置始终有可用的默认纸张对象。 */
function normalizeConfig(value: AgentConfig): AgentConfig {
  return {
    ...value,
    printing: {
      ...value.printing,
      default_paper: value.printing.default_paper ?? { ...DEFAULT_PAPER },
    },
    remote: {
      ...DEFAULT_REMOTE_CONFIG,
      ...value.remote,
    },
  };
}

function defaultExportOptions(): ExportConfigOptions {
  return {
    service_port: true,
    allowed_origins: true,
    remote_enabled: true,
    remote_endpoint_url: true,
    remote_bearer_token: true,
    remote_poll_interval_seconds: true,
    remote_max_report_retries: true,
  };
}

/** 判断已存储的字符串是否是支持的主题模式。 */
function isThemeMode(value: string | null): value is ThemeMode {
  return value === 'system' || value === 'light' || value === 'dark';
}

/** 从本地存储读取已保存的主题模式。 */
function readThemeMode(): ThemeMode {
  if (typeof window === 'undefined') return 'system';

  const storedMode = window.localStorage.getItem(THEME_STORAGE_KEY);
  return isThemeMode(storedMode) ? storedMode : 'system';
}

/** 把选择的主题或系统推导出的主题应用到页面。 */
function applyTheme(mode: ThemeMode): void {
  if (typeof window === 'undefined') return;

  const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
  const shouldUseDark = mode === 'dark' || (mode === 'system' && prefersDark);

  document.documentElement.classList.toggle('dark', shouldUseDark);
  document.documentElement.style.colorScheme = shouldUseDark ? 'dark' : 'light';
}

/** 保存选择的主题模式，并立即应用。 */
function setThemeMode(value: string): void {
  const nextMode = isThemeMode(value) ? value : 'system';

  themeMode.value = nextMode;
  window.localStorage.setItem(THEME_STORAGE_KEY, nextMode);
  applyTheme(nextMode);
}

/** 生成单个主题选项按钮的样式类。 */
function themeOptionClass(mode: ThemeMode): string {
  const isActive = themeMode.value === mode;
  return [
    'inline-flex h-8 items-center justify-center gap-2 rounded-md px-3 text-sm font-medium transition-colors',
    'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background',
    isActive
      ? 'bg-primary text-primary-foreground shadow-sm'
      : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground',
  ].join(' ');
}

/** 系统配色变化时重新应用系统主题。 */
function handleSystemThemeChange(): void {
  if (themeMode.value === 'system') {
    applyTheme('system');
  }
}

/** 注册系统主题模式需要的系统配色监听器。 */
function setupThemeSync(): void {
  if (typeof window === 'undefined') return;

  colorSchemeQuery = window.matchMedia('(prefers-color-scheme: dark)');
  colorSchemeQuery.addEventListener('change', handleSystemThemeChange);
}

/** 返回可编辑的纸张配置，必要时创建默认纸张。 */
function currentPaper(): EffectivePaper {
  if (!config.value) return DEFAULT_PAPER;
  if (!config.value.printing.default_paper) {
    config.value.printing.default_paper = { ...DEFAULT_PAPER };
  }

  return config.value.printing.default_paper;
}

/** 查找与给定纸张尺寸相同的打印机纸张预设。 */
function matchingPaper(paper: EffectivePaper): PaperInfo | undefined {
  return papers.value.find(
    (item) =>
      Math.abs(item.width_mm - paper.width_mm) < 0.01 &&
      Math.abs(item.height_mm - paper.height_mm) < 0.01,
  );
}

/** 输入值有效时更新配置中的服务端口。 */
function setPort(value: string | number): void {
  if (!config.value) return;
  const port = Number(value);
  if (Number.isInteger(port)) {
    config.value.service.port = Math.min(MAX_SERVICE_PORT, Math.max(MIN_SERVICE_PORT, port));
  }
}

/** 输入值有效时更新一个自定义纸张尺寸。 */
function setPaperDimension(key: keyof EffectivePaper, value: string | number): void {
  const dimension = Number(value);
  if (Number.isFinite(dimension) && dimension > 0) {
    currentPaper()[key] = dimension;
  }
}

/** 更新远程任务配置中的可空字符串。 */
function setRemoteString(
  key: 'endpoint_url' | 'bearer_token' | 'device_id' | 'device_name',
  value: string | number,
): void {
  if (!config.value) return;
  const nextValue = String(value).trim();
  config.value.remote[key] = nextValue || null;
}

/** 更新远程任务配置中的正整数。 */
function setRemoteNumber(
  key: 'poll_interval_seconds' | 'max_report_retries',
  value: string | number,
): void {
  if (!config.value) return;
  const nextValue = Number(value);
  if (Number.isInteger(nextValue)) {
    const minValue =
      key === 'poll_interval_seconds'
        ? MIN_REMOTE_POLL_INTERVAL_SECONDS
        : MIN_REMOTE_MAX_REPORT_RETRIES;
    config.value.remote[key] = Math.max(minValue, nextValue);
  }
}

/** 关闭顶部提示消息。 */
function dismissMessage(kind: 'error' | 'success'): void {
  if (kind === 'error') {
    errorMessage.value = '';
  } else {
    successMessage.value = '';
  }
}

function openExportDialog(): void {
  exportOptions.value = defaultExportOptions();
  exportPassword.value = '';
  showEmptyPasswordTokenConfirm.value = false;
  showExportDialog.value = true;
}

async function handleExportConfig(): Promise<void> {
  if (exportOptions.value.remote_bearer_token && exportPassword.value.length === 0) {
    showEmptyPasswordTokenConfirm.value = true;
    return;
  }

  await runExportConfig();
}

async function confirmEmptyPasswordExport(): Promise<void> {
  showEmptyPasswordTokenConfirm.value = false;
  await runExportConfig();
}

async function runExportConfig(): Promise<void> {
  exportingConfig.value = true;
  errorMessage.value = '';
  successMessage.value = '';

  try {
    const path = await saveDialog({
      defaultPath: 'printbridge-config.json',
      filters: [{ name: 'PrintBridge Config', extensions: ['json'] }],
    });
    if (!path) return;

    await exportConfigFile(path, exportPassword.value, exportOptions.value);
    showExportDialog.value = false;
    successMessage.value = '配置已导出';
  } catch (error) {
    errorMessage.value = error instanceof Error ? error.message : '导出配置失败';
  } finally {
    exportingConfig.value = false;
  }
}

function openImportDialog(): void {
  importPassword.value = '';
  importPath.value = '';
  importPreview.value = null;
  importErrorMessage.value = '';
  showImportDialog.value = true;
}

function resetImportPreview(): void {
  importPreview.value = null;
  importErrorMessage.value = '';
}

async function chooseImportFile(): Promise<void> {
  const path = await openDialog({
    multiple: false,
    filters: [{ name: 'PrintBridge Config', extensions: ['json'] }],
  });
  if (typeof path === 'string') {
    importPath.value = path;
    resetImportPreview();
  }
}

async function handlePreviewConfigImport(): Promise<void> {
  if (!importPath.value) return;
  previewingConfigImport.value = true;
  importErrorMessage.value = '';
  errorMessage.value = '';
  successMessage.value = '';

  try {
    importPreview.value = await previewConfigImport(importPath.value, importPassword.value);
  } catch (error) {
    importPreview.value = null;
    importErrorMessage.value = error instanceof Error ? error.message : '导入预览失败';
  } finally {
    previewingConfigImport.value = false;
  }
}

async function handleImportConfig(): Promise<void> {
  if (!importPath.value || !importPreview.value) return;
  importingConfig.value = true;
  importErrorMessage.value = '';
  errorMessage.value = '';
  successMessage.value = '';

  try {
    config.value = normalizeConfig(
      await importConfigFile(importPath.value, importPassword.value, importPreview.value.file_hash),
    );
    showImportDialog.value = false;
    successMessage.value = '配置已导入';
  } catch (error) {
    importErrorMessage.value = error instanceof Error ? error.message : '导入配置失败';
  } finally {
    importingConfig.value = false;
  }
}

/** 为当前设备生成随机 UUID v4。 */
function generateRemoteDeviceId(): void {
  if (!config.value) return;
  config.value.remote.device_id = crypto.randomUUID();
}

/** 加载配置、打印机、纸张和任务历史，初始化页面状态。 */
async function loadConfig(): Promise<void> {
  loadingConfig.value = true;
  errorMessage.value = '';

  try {
    config.value = normalizeConfig(await getConfig());
    activePort.value = config.value.service.port;
    await Promise.all([refreshPrinters(), refreshTaskHistory()]);
  } catch (error) {
    errorMessage.value = error instanceof Error ? error.message : '加载配置失败';
  } finally {
    loadingConfig.value = false;
  }
}

/** 从 Tauri 读取当前桌面应用版本。 */
async function loadAppVersion(): Promise<void> {
  try {
    appVersion.value = await getVersion();
  } catch {
    appVersion.value = '-';
  }
}

/** 刷新打印机列表，并在未配置时选择合适的默认打印机。 */
async function refreshPrinters(): Promise<void> {
  if (!config.value) return;
  loadingPrinters.value = true;
  errorMessage.value = '';

  try {
    printers.value = await fetchPrinters(servicePort.value);
    if (!config.value.printing.default_printer) {
      config.value.printing.default_printer =
        printers.value.find((printer) => printer.is_default)?.name ??
        printers.value[0]?.name ??
        null;
    }
    await refreshPapers();
  } catch (error) {
    printers.value = [];
    papers.value = [];
    errorMessage.value = error instanceof Error ? error.message : '刷新打印机失败';
  } finally {
    loadingPrinters.value = false;
  }
}

/** 刷新当前选中打印机的纸张选项。 */
async function refreshPapers(): Promise<void> {
  if (!config.value?.printing.default_printer) {
    papers.value = [];
    return;
  }

  loadingPapers.value = true;
  errorMessage.value = '';

  try {
    papers.value = await fetchPapers(servicePort.value, config.value.printing.default_printer);
  } catch (error) {
    papers.value = [];
    errorMessage.value = error instanceof Error ? error.message : '刷新纸张失败';
  } finally {
    loadingPapers.value = false;
  }
}

/** 应用打印机选择，并重新加载对应纸张列表。 */
async function handlePrinterChange(value: string): Promise<void> {
  selectedPrinter.value = value;
  await refreshPapers();
}

/** 通过 Tauri 保存当前设置；端口变化时重启应用让新监听端口生效。 */
async function persistConfig(): Promise<void> {
  if (!config.value) return;
  saving.value = true;
  errorMessage.value = '';
  successMessage.value = '';

  try {
    const savedPort = config.value.service.port;
    const portChanged = activePort.value !== null && savedPort !== activePort.value;
    if (config.value.remote.enabled) {
      await testRemoteConnection(config.value);
    }
    config.value = normalizeConfig(await saveConfig(config.value));
    if (portChanged) {
      if (await isDebugBuild()) {
        successMessage.value = '设置已保存；开发模式下请手动重启 pnpm tauri dev 后生效';
        return;
      }
      successMessage.value = '设置已保存，正在重启应用';
      await relaunchApp();
      return;
    }
    successMessage.value = '设置已保存';
  } catch (error) {
    errorMessage.value = error instanceof Error ? error.message : '保存或重启失败';
  } finally {
    saving.value = false;
  }
}

/** 手动测试远程任务 URL 的 GET/POST 连通性。 */
async function handleTestRemoteConnection(): Promise<void> {
  if (!config.value || !config.value.remote.enabled) return;
  testingRemote.value = true;
  errorMessage.value = '';
  successMessage.value = '';

  try {
    await testRemoteConnection(config.value);
    successMessage.value = '远程任务连接测试通过';
  } catch (error) {
    errorMessage.value = error instanceof Error ? error.message : '远程任务连接测试失败';
  } finally {
    testingRemote.value = false;
  }
}

/** 使用当前 Agent 默认打印设置提交测试校准页。 */
async function handleTestPrint(): Promise<void> {
  if (!config.value || !canTestPrint.value) return;
  testingPrint.value = true;
  errorMessage.value = '';
  successMessage.value = '';

  try {
    await printTestPage(config.value);
    successMessage.value = '测试打印已提交';
  } catch (error) {
    errorMessage.value = error instanceof Error ? error.message : '测试打印失败';
  } finally {
    testingPrint.value = false;
  }
}

/** 从本地 Agent 刷新任务历史。 */
async function refreshTaskHistory(): Promise<void> {
  confirmingClearTaskHistory.value = false;
  const requestId = ++taskHistoryRequestId;
  loadingTaskHistory.value = true;

  try {
    const nextTaskHistory = await getTaskHistory();
    if (requestId !== taskHistoryRequestId) return;

    taskHistory.value = nextTaskHistory;
    errorMessage.value = '';
    if (selectedTaskJobId.value) {
      const stillExists = taskHistory.value.some((item) => item.job_id === selectedTaskJobId.value);
      if (stillExists) {
        await selectTask(selectedTaskJobId.value);
      } else {
        selectedTaskJobId.value = null;
        selectedTaskEvents.value = [];
      }
    }
  } catch (error) {
    if (requestId !== taskHistoryRequestId) return;
    errorMessage.value = error instanceof Error ? error.message : '任务历史加载失败';
  } finally {
    if (requestId === taskHistoryRequestId) {
      loadingTaskHistory.value = false;
    }
  }
}

/** 读取单个任务的状态事件。 */
async function selectTask(jobId: string): Promise<void> {
  const requestId = ++taskEventsRequestId;
  selectedTaskJobId.value = jobId;
  selectedTaskEvents.value = [];
  loadingTaskEvents.value = true;

  try {
    const nextEvents = await getTaskHistoryEvents(jobId);
    if (requestId !== taskEventsRequestId || selectedTaskJobId.value !== jobId) return;

    selectedTaskEvents.value = nextEvents;
    errorMessage.value = '';
  } catch (error) {
    if (requestId !== taskEventsRequestId || selectedTaskJobId.value !== jobId) return;
    errorMessage.value = error instanceof Error ? error.message : '任务状态加载失败';
  } finally {
    if (requestId === taskEventsRequestId && selectedTaskJobId.value === jobId) {
      loadingTaskEvents.value = false;
    }
  }
}

/** 清空本地任务历史。 */
async function handleClearTaskHistory(): Promise<void> {
  if (!confirmingClearTaskHistory.value) {
    confirmingClearTaskHistory.value = true;
    errorMessage.value = '';
    successMessage.value = '';
    return;
  }

  clearingTaskHistory.value = true;
  errorMessage.value = '';
  successMessage.value = '';

  try {
    await clearTaskHistory();
    taskHistoryRequestId++;
    taskEventsRequestId++;
    taskHistory.value = [];
    selectedTaskJobId.value = null;
    selectedTaskEvents.value = [];
    confirmingClearTaskHistory.value = false;
    successMessage.value = '任务历史已清空';
  } catch (error) {
    confirmingClearTaskHistory.value = false;
    errorMessage.value = error instanceof Error ? error.message : '清空任务历史失败';
  } finally {
    clearingTaskHistory.value = false;
  }
}

/** 把校验通过的浏览器 Origin 加入允许列表。 */
function addOrigin(): void {
  if (!config.value) return;
  const origin = originDraft.value.trim();
  originErrorMessage.value = '';

  if (!origin) return;
  if (!isValidOrigin(origin)) {
    originErrorMessage.value = '请输入有效 Origin，例如 https://example.com';
    return;
  }
  if (config.value.security.allowed_origins.includes(origin)) return;

  config.value.security.allowed_origins.push(origin);
  originDraft.value = '';
}

/** 校验 Origin 字符串必须只包含协议和主机。 */
function isValidOrigin(value: string): boolean {
  try {
    const url = new URL(value);
    if (url.protocol !== 'http:' && url.protocol !== 'https:') return false;

    return `${url.protocol}//${url.host}` === value;
  } catch {
    return false;
  }
}

/** 从允许列表移除一个浏览器 Origin。 */
function removeOrigin(origin: string): void {
  if (!config.value) return;
  config.value.security.allowed_origins = config.value.security.allowed_origins.filter(
    (item) => item !== origin,
  );
}

/** 格式化 RFC3339 时间用于展示。 */
function formatDateTime(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;

  return date.toLocaleString();
}

/** 返回任务状态的中文标签。 */
function taskStatusLabel(status: TaskHistoryStatus): string {
  const labels: Record<TaskHistoryStatus, string> = {
    queued: '已排队',
    downloading: '下载中',
    printing: '提交中',
    submitted: '已提交系统队列',
    completed: '系统队列已完成',
    failed: '失败',
    unknown: '无法确认后续状态',
    cancelled: '已取消',
  };

  return labels[status];
}

/** 返回任务来源的中文标签。 */
function taskSourceLabel(source: TaskHistorySource): string {
  const labels: Record<TaskHistorySource, string> = {
    web_socket: '网页',
    remote: '远程',
    test: '测试',
  };

  return labels[source];
}

/** 格式化字节数用于更新下载进度展示。 */
function formatBytes(value: number): string {
  if (value < 1024) return `${value} B`;
  const kib = value / 1024;
  if (kib < 1024) return `${kib.toFixed(1)} KB`;

  return `${(kib / 1024).toFixed(1)} MB`;
}

/** 格式化毫米尺寸，设置页用整数毫米展示。 */
function formatMillimeters(value: number): string {
  return Math.round(value).toString();
}

/** 生成纸张下拉项文案，避免尺寸型名称重复显示。 */
function formatPaperLabel(paper: PaperInfo): string {
  const sizeLabel = `${formatMillimeters(paper.width_mm)} x ${formatMillimeters(paper.height_mm)} mm`;
  if (/^\d+(?:\.\d+)? x \d+(?:\.\d+)? mm$/.test(paper.name)) {
    return sizeLabel;
  }

  return `${paper.name} · ${sizeLabel}`;
}

/** 检查是否有可用的桌面应用更新。 */
async function checkForUpdate(): Promise<void> {
  updateStatus.value = 'checking';
  updateMessage.value = '';
  availableUpdate.value = null;
  updateInfo.value = null;
  updateProgress.value = {
    downloadedBytes: 0,
    contentLength: null,
  };

  try {
    const update = await checkForAppUpdate();
    if (!update) {
      updateStatus.value = 'not-available';
      updateMessage.value = '当前版本已经是最新版本';
      return;
    }

    availableUpdate.value = update;
    updateInfo.value = toUpdateInfo(update);
    updateStatus.value = 'available';
    updateMessage.value = `检测到新版本：${update.version}`;
  } catch (error) {
    updateStatus.value = 'error';
    updateMessage.value = error instanceof Error ? error.message : '检查更新失败';
  }
}

/** 下载并安装当前可用的更新。 */
async function installAvailableUpdate(): Promise<void> {
  if (!availableUpdate.value) return;

  updateStatus.value = 'downloading';
  updateMessage.value = '正在下载并安装更新';
  updateProgress.value = {
    downloadedBytes: 0,
    contentLength: null,
  };

  try {
    updateProgress.value = await downloadAndInstallAppUpdate(availableUpdate.value, (progress) => {
      updateProgress.value = progress;
    });
    updateStatus.value = 'installed';
    updateMessage.value = '更新已安装，重启后生效';
  } catch (error) {
    updateStatus.value = 'error';
    updateMessage.value = error instanceof Error ? error.message : '安装更新失败';
  }
}

/** 更新安装完成后重启应用。 */
async function restartAfterUpdate(): Promise<void> {
  try {
    await relaunchApp();
  } catch (error) {
    updateStatus.value = 'error';
    updateMessage.value = error instanceof Error ? error.message : '重启应用失败';
  }
}

/** 用默认浏览器打开项目仓库。 */
async function openGitHubRepository(): Promise<void> {
  try {
    await openUrl(GITHUB_REPOSITORY_URL);
  } catch (error) {
    updateStatus.value = 'error';
    updateMessage.value = error instanceof Error ? error.message : '打开 GitHub 失败';
  }
}

/** 用默认浏览器打开 GitHub Releases 页面。 */
async function openReleaseNotes(): Promise<void> {
  try {
    await openUrl(GITHUB_RELEASES_URL);
  } catch (error) {
    updateStatus.value = 'error';
    updateMessage.value = error instanceof Error ? error.message : '打开更新日志失败';
  }
}

applyTheme(themeMode.value);

onMounted(() => {
  // 首次加载页面时，先同步主题监听，再加载异步应用数据。
  setupThemeSync();
  void loadConfig();
  void loadAppVersion();
});

onBeforeUnmount(() => {
  // 设置页面销毁时移除全局媒体查询监听器。
  colorSchemeQuery?.removeEventListener('change', handleSystemThemeChange);
});
</script>

<template>
  <main class="min-h-screen bg-muted/30 px-4 py-4 text-foreground md:px-6">
    <div class="mx-auto flex w-full max-w-5xl flex-col gap-4">
      <header class="flex flex-col gap-3 border-b bg-background/80 pb-4 md:flex-row md:items-center md:justify-between">
        <div>
          <h1 class="text-xl font-semibold tracking-normal">PrintBridge</h1>
          <p class="text-sm text-muted-foreground">本地端口 {{ servicePort || '-' }}</p>
        </div>
        <div class="flex flex-wrap items-center gap-2">
          <Badge :variant="statusVariant">
            {{ statusLabel }}
          </Badge>
          <Button :disabled="!config || saving" @click="persistConfig">
            <Save class="size-4" />
            {{ saving ? '保存中' : '保存' }}
          </Button>
          <Button variant="outline" :disabled="!config || exportingConfig" @click="openExportDialog">
            <FileDown class="size-4" />
            导出配置
          </Button>
          <Button variant="outline" :disabled="!config || importingConfig" @click="openImportDialog">
            <FileUp class="size-4" />
            导入配置
          </Button>
        </div>
      </header>

      <div v-if="errorMessage || successMessage" class="grid gap-2 text-sm">
        <Alert v-if="errorMessage" variant="error" class="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-3">
          <AlertDescription class="min-w-0 break-words">
            {{ errorMessage }}
          </AlertDescription>
          <Button variant="ghost" size="icon-sm"
            class="shrink-0 text-destructive hover:bg-destructive/15 hover:text-destructive" aria-label="关闭错误提示"
            @click="dismissMessage('error')">
            <X class="size-4" />
          </Button>
        </Alert>
        <Alert v-if="successMessage" variant="success" class="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-3">
          <AlertDescription class="min-w-0 break-words">
            {{ successMessage }}
          </AlertDescription>
          <Button variant="ghost" size="icon-sm"
            class="shrink-0 text-emerald-800 hover:bg-emerald-500/15 hover:text-emerald-900 dark:text-emerald-200 dark:hover:text-emerald-100"
            aria-label="关闭成功提示" @click="dismissMessage('success')">
            <X class="size-4" />
          </Button>
        </Alert>
      </div>

      <Card v-if="loadingConfig">
        <CardContent class="py-12 text-center text-sm text-muted-foreground">
          正在加载设置...
        </CardContent>
      </Card>

      <Tabs v-else-if="config" default-value="settings">
        <TabsList class="grid w-full grid-cols-5 md:w-[600px]">
          <TabsTrigger value="settings"> 设置 </TabsTrigger>
          <TabsTrigger value="remote"> 远程 </TabsTrigger>
          <TabsTrigger value="security"> 安全 </TabsTrigger>
          <TabsTrigger value="logs"> 任务 </TabsTrigger>
          <TabsTrigger value="updates"> 关于 </TabsTrigger>
        </TabsList>

        <TabsContent value="settings" class="mt-2">
          <Card>
            <CardHeader class="pb-3">
              <CardTitle class="text-base"> 打印设置 </CardTitle>
            </CardHeader>
            <CardContent class="grid gap-5">
              <div class="grid gap-2 md:grid-cols-[minmax(0,1fr)_auto_auto] md:items-end">
                <div class="grid gap-2">
                  <Label for="default-printer">默认打印机</Label>
                  <Select :model-value="selectedPrinter" @update:model-value="handlePrinterChange(String($event))">
                    <SelectTrigger id="default-printer" class="w-full">
                      <SelectValue placeholder="选择打印机" />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem v-for="printer in printers" :key="printer.name" :value="printer.name">
                        {{ printer.name }}{{ printer.is_default ? '（系统默认）' : '' }}
                      </SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                <Button class="whitespace-nowrap" variant="outline" :disabled="loadingPrinters"
                  @click="refreshPrinters">
                  <RefreshCw class="size-4" :class="{ 'animate-spin': loadingPrinters }" />
                  刷新
                </Button>
                <Button class="whitespace-nowrap" variant="outline" :disabled="!canTestPrint || testingPrint"
                  @click="handleTestPrint">
                  <Printer class="size-4" />
                  {{ testingPrint ? '提交中' : '测试打印' }}
                </Button>
              </div>

              <div
                class="grid items-start gap-4 md:grid-cols-[minmax(0,1.35fr)_minmax(140px,0.45fr)_minmax(140px,0.45fr)]"
              >
                <div class="grid gap-2">
                  <Label for="default-paper">默认纸张</Label>
                  <Select :model-value="selectedPaper" @update:model-value="selectedPaper = String($event)">
                    <SelectTrigger id="default-paper" class="w-full">
                      <SelectValue placeholder="选择纸张" />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="custom"> 自定义尺寸 </SelectItem>
                      <SelectItem v-for="paper in papers" :key="paper.id" :value="paper.id">
                        {{ formatPaperLabel(paper) }}
                      </SelectItem>
                    </SelectContent>
                  </Select>
                  <p class="text-xs text-muted-foreground">
                    {{
                      loadingPapers
                        ? '正在读取纸张列表'
                        : papers.length
                          ? '可选择驱动纸张，也可直接编辑尺寸'
                          : '未读取到纸张列表，可直接编辑默认尺寸'
                    }}
                  </p>
                </div>
                <div class="grid gap-2">
                  <Label for="paper-width">宽度（mm）</Label>
                  <Input id="paper-width" type="number" min="1" step="0.1"
                    :model-value="config.printing.default_paper?.width_mm ?? DEFAULT_PAPER.width_mm"
                    @update:model-value="setPaperDimension('width_mm', $event)" />
                </div>
                <div class="grid gap-2">
                  <Label for="paper-height">高度（mm）</Label>
                  <Input id="paper-height" type="number" min="1" step="0.1" :model-value="config.printing.default_paper?.height_mm ?? DEFAULT_PAPER.height_mm
                    " @update:model-value="setPaperDimension('height_mm', $event)" />
                </div>
              </div>

              <Separator />

              <div class="grid gap-4 md:grid-cols-2">
                <div class="grid gap-2">
                  <Label for="service-port">本地端口</Label>
                  <Input id="service-port" type="number" :min="MIN_SERVICE_PORT" :max="MAX_SERVICE_PORT" :model-value="config.service.port"
                    @update:model-value="setPort" />
                  <p v-if="hasPendingPortChange" class="text-xs text-muted-foreground">
                    当前会话仍连接 {{ activePort }}，保存后将重启应用生效。
                  </p>
                </div>
                <div class="grid gap-2">
                  <Label for="autostart">开机自启</Label>
                  <div class="flex min-h-10 items-center justify-between gap-3">
                    <p class="text-xs text-muted-foreground">启动系统后自动运行 PrintBridge</p>
                    <Switch id="autostart" v-model="config.app.autostart" />
                  </div>
                </div>
              </div>

              <Separator />

              <div
                class="flex flex-col gap-3 rounded-md border px-3 py-3 sm:flex-row sm:items-center sm:justify-between"
              >
                <Label>界面主题</Label>
                <div
                  class="grid w-full grid-cols-3 gap-1 rounded-lg border bg-muted/40 p-1 sm:w-auto"
                  role="group"
                  aria-label="界面主题"
                >
                  <button
                    type="button"
                    :class="themeOptionClass('light')"
                    :aria-pressed="themeMode === 'light'"
                    @click="setThemeMode('light')"
                  >
                    <Sun class="size-4" />
                    <span>浅色</span>
                  </button>
                  <button
                    type="button"
                    :class="themeOptionClass('dark')"
                    :aria-pressed="themeMode === 'dark'"
                    @click="setThemeMode('dark')"
                  >
                    <Moon class="size-4" />
                    <span>深色</span>
                  </button>
                  <button
                    type="button"
                    :class="themeOptionClass('system')"
                    :aria-pressed="themeMode === 'system'"
                    @click="setThemeMode('system')"
                  >
                    <Monitor class="size-4" />
                    <span>跟随系统</span>
                  </button>
                </div>
              </div>
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="remote" class="mt-2">
          <Card>
            <CardHeader class="pb-3">
              <div class="flex items-center justify-between gap-3">
                <CardTitle class="text-base"> 远程任务 </CardTitle>
                <Switch id="remote-enabled" v-model="config.remote.enabled" />
              </div>
            </CardHeader>
            <CardContent class="grid gap-5">
              <div class="grid gap-2 md:grid-cols-[minmax(0,1fr)_auto] md:items-end">
                <div class="grid min-w-0 gap-2">
                  <Label for="remote-url">任务 URL</Label>
                  <Input id="remote-url" type="url" placeholder="https://api.example.com/print-task"
                    :model-value="config.remote.endpoint_url ?? ''"
                    @update:model-value="setRemoteString('endpoint_url', $event)" />
                </div>
                <Button class="whitespace-nowrap" variant="outline" :disabled="!config.remote.enabled || testingRemote"
                  @click="handleTestRemoteConnection">
                  <RefreshCw class="size-4" :class="{ 'animate-spin': testingRemote }" />
                  {{ testingRemote ? '测试中' : '测试连接' }}
                </Button>
              </div>

              <div class="grid gap-2">
                <Label for="remote-token">Authorization Bearer Token</Label>
                <Input id="remote-token" type="password" autocomplete="off"
                  :model-value="config.remote.bearer_token ?? ''"
                  @update:model-value="setRemoteString('bearer_token', $event)" />
              </div>

              <div class="grid gap-4 md:grid-cols-2">
                <div class="grid gap-2">
                  <Label for="remote-device-id">Device ID</Label>
                  <div class="flex gap-2">
                    <Input id="remote-device-id" class="min-w-0 flex-1" :model-value="config.remote.device_id ?? ''"
                      @update:model-value="setRemoteString('device_id', $event)" />
                    <Button class="whitespace-nowrap" variant="outline" type="button" @click="generateRemoteDeviceId">
                      <Shuffle class="size-4" />
                      随机生成
                    </Button>
                  </div>
                </div>
                <div class="grid gap-2">
                  <Label for="remote-device-name">Device Name</Label>
                  <Input id="remote-device-name" :model-value="config.remote.device_name ?? ''"
                    @update:model-value="setRemoteString('device_name', $event)" />
                </div>
              </div>

              <div class="grid gap-4 md:grid-cols-2">
                <div class="grid gap-2">
                  <Label for="remote-poll-interval">轮询时间（秒）</Label>
                  <Input id="remote-poll-interval" type="number" :min="MIN_REMOTE_POLL_INTERVAL_SECONDS"
                    :model-value="config.remote.poll_interval_seconds"
                    @update:model-value="setRemoteNumber('poll_interval_seconds', $event)" />
                </div>
                <div class="grid gap-2">
                  <Label for="remote-max-retries">上报重试次数</Label>
                  <Input id="remote-max-retries" type="number" :min="MIN_REMOTE_MAX_REPORT_RETRIES" :model-value="config.remote.max_report_retries"
                    @update:model-value="setRemoteNumber('max_report_retries', $event)" />
                </div>
              </div>
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="security" class="mt-2">
          <Card>
            <CardHeader class="pb-3">
              <CardTitle class="text-base"> Origin 白名单 </CardTitle>
            </CardHeader>
            <CardContent class="grid gap-4">
              <form class="flex items-start gap-2" @submit.prevent="addOrigin">
                <div class="grid flex-1 gap-1">
                  <Input v-model="originDraft" placeholder="https://example.com" autocomplete="off"
                    :aria-invalid="originErrorMessage ? 'true' : 'false'" />
                  <p v-if="originErrorMessage" class="text-xs text-destructive">
                    {{ originErrorMessage }}
                  </p>
                </div>
                <Button type="submit"> 添加 </Button>
              </form>
              <div class="grid gap-2">
                <div v-for="origin in config.security.allowed_origins" :key="origin"
                  class="flex items-center justify-between rounded-md border px-3 py-2 text-sm">
                  <span class="truncate">{{ origin }}</span>
                  <Button variant="ghost" size="icon-sm" aria-label="删除 Origin" @click="removeOrigin(origin)">
                    <Trash2 class="size-4" />
                  </Button>
                </div>
                <p v-if="config.security.allowed_origins.length === 0"
                  class="rounded-md border border-dashed px-3 py-6 text-center text-sm text-muted-foreground">
                  暂无白名单 Origin
                </p>
              </div>
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="updates" class="mt-2">
          <div class="grid gap-4">
            <div>
              <h2 class="text-base font-semibold tracking-normal">关于</h2>
              <p class="text-sm text-muted-foreground">查看版本信息与更新状态。</p>
            </div>

            <Card>
              <CardContent class="grid gap-5 p-5">
                <div class="flex flex-col gap-4 md:flex-row md:items-start md:justify-between">
                  <div class="flex min-w-0 items-start gap-3">
                    <img :src="appIcon" alt="" class="mt-1 size-10 rounded-md" />
                    <div class="min-w-0">
                      <div class="flex flex-wrap items-center gap-2">
                        <h3 class="text-xl font-semibold tracking-normal">PrintBridge</h3>
                        <Badge variant="outline">版本 v{{ currentAppVersion }}</Badge>
                      </div>
                      <p class="mt-2 text-sm text-muted-foreground">本地打印桥接程序</p>
                    </div>
                  </div>

                  <div class="flex flex-wrap gap-2 md:justify-end">
                    <Button variant="outline" @click="openGitHubRepository">
                      <ExternalLink class="size-4" />
                      GitHub
                    </Button>
                    <Button variant="outline" @click="openReleaseNotes">
                      <ExternalLink class="size-4" />
                      更新日志
                    </Button>
                    <Button variant="outline" :disabled="checkingUpdate || installingUpdate" @click="checkForUpdate">
                      <RefreshCw class="size-4" :class="{ 'animate-spin': checkingUpdate }" />
                      {{ checkingUpdate ? '检查中' : '检查更新' }}
                    </Button>
                    <Button v-if="availableUpdate && updateStatus !== 'installed'" :disabled="installingUpdate"
                      @click="installAvailableUpdate">
                      <Download class="size-4" />
                      {{ updateButtonLabel }}
                    </Button>
                    <Button v-if="updateStatus === 'installed'" variant="outline" @click="restartAfterUpdate">
                      <RotateCw class="size-4" />
                      重启应用
                    </Button>
                  </div>
                </div>

                <div class="rounded-md border px-4 py-3 text-sm" :class="{
                  'border-primary/40 bg-primary/10 text-primary': updateStatus === 'available',
                  'border-destructive/30 bg-destructive/10 text-destructive':
                    updateStatus === 'error',
                  'bg-muted/40 text-muted-foreground':
                    updateStatus !== 'available' && updateStatus !== 'error',
                }">
                  <span v-if="updateStatus === 'available'">
                    检测到新版本：{{ availableUpdateVersion }}
                  </span>
                  <span v-else-if="updateMessage">{{ updateMessage }}</span>
                  <span v-else>点击检查更新获取最新版本状态。</span>
                </div>

                <div v-if="installingUpdate" class="grid gap-2">
                  <div class="h-2 overflow-hidden rounded-full bg-muted">
                    <div class="h-full rounded-full bg-primary transition-all"
                      :style="{ width: `${updateProgressPercent}%` }" />
                  </div>
                  <p class="text-xs text-muted-foreground">
                    {{ updateProgressText }}
                  </p>
                </div>

                <div v-if="updateInfo?.body" class="grid gap-2">
                  <Label>更新说明</Label>
                  <div
                    class="max-h-36 overflow-auto whitespace-pre-wrap rounded-md border bg-muted/20 px-3 py-2 text-sm">
                    {{ updateInfo.body }}
                  </div>
                </div>
              </CardContent>
            </Card>
          </div>
        </TabsContent>

        <TabsContent value="logs" class="mt-2">
          <Card>
            <CardHeader class="flex flex-row items-center justify-between pb-3">
              <CardTitle class="text-base"> 打印任务 </CardTitle>
              <div class="flex items-center gap-2">
                <Button variant="outline" size="sm" :disabled="loadingTaskHistory || clearingTaskHistory"
                  @click="refreshTaskHistory">
                  <RefreshCw class="size-4" :class="{ 'animate-spin': loadingTaskHistory }" />
                  刷新
                </Button>
                <Button :variant="confirmingClearTaskHistory ? 'destructive' : 'outline'" size="sm"
                  :disabled="loadingTaskHistory || clearingTaskHistory || taskHistory.length === 0"
                  @click="handleClearTaskHistory">
                  <Trash2 class="size-4" />
                  {{ confirmingClearTaskHistory ? '确认清空' : '清空' }}
                </Button>
              </div>
            </CardHeader>
            <CardContent class="grid gap-4 xl:grid-cols-[minmax(0,1.45fr)_minmax(320px,0.8fr)]">
              <ScrollArea class="h-[420px] rounded-md border">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead class="w-[170px]"> 时间 </TableHead>
                      <TableHead class="w-[145px]"> 状态 </TableHead>
                      <TableHead class="w-[80px]"> 来源 </TableHead>
                      <TableHead class="w-[150px]"> Job </TableHead>
                      <TableHead class="w-[150px]"> 打印机 </TableHead>
                      <TableHead> 消息 </TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    <TableEmpty v-if="taskHistory.length === 0" :colspan="6">
                      {{ loadingTaskHistory ? '正在加载任务...' : '暂无任务' }}
                    </TableEmpty>
                    <TableRow v-for="entry in taskHistory" v-else :key="entry.job_id" class="cursor-pointer"
                      :class="{ 'bg-muted/60': selectedTaskJobId === entry.job_id }" @click="selectTask(entry.job_id)">
                      <TableCell class="whitespace-nowrap text-muted-foreground">
                        {{ formatDateTime(entry.updated_at) }}
                      </TableCell>
                      <TableCell>
                        <Badge variant="outline">
                          {{ taskStatusLabel(entry.current_status) }}
                        </Badge>
                      </TableCell>
                      <TableCell class="whitespace-nowrap text-muted-foreground">
                        {{ taskSourceLabel(entry.source) }}
                      </TableCell>
                      <TableCell class="max-w-[150px] truncate font-mono text-xs">
                        {{ entry.job_id }}
                      </TableCell>
                      <TableCell class="max-w-[150px] truncate text-muted-foreground">
                        {{ entry.printer_name ?? '-' }}
                      </TableCell>
                      <TableCell class="min-w-[180px] break-words">
                        {{ entry.current_message ?? '-' }}
                      </TableCell>
                    </TableRow>
                  </TableBody>
                </Table>
              </ScrollArea>

              <div class="min-w-0 rounded-md border">
                <div class="min-w-0 border-b px-3 py-2">
                  <div class="truncate text-sm font-medium">
                    {{ selectedTask?.job_id ?? '任务详情' }}
                  </div>
                  <div class="mt-1 break-words text-xs text-muted-foreground">
                    {{
                      selectedTask
                        ? `${selectedTask.paper_name ?? '-'} · ${selectedTask.copies ?? '-'} 份`
                        : '选择左侧任务查看状态'
                    }}
                  </div>
                </div>
                <ScrollArea class="h-[360px]">
                  <div v-if="!selectedTaskJobId" class="px-3 py-8 text-sm text-muted-foreground">
                    暂无选中任务
                  </div>
                  <div v-else-if="loadingTaskEvents" class="px-3 py-8 text-sm text-muted-foreground">
                    正在加载状态...
                  </div>
                  <div v-else-if="selectedTaskEvents.length === 0" class="px-3 py-8 text-sm text-muted-foreground">
                    暂无状态记录
                  </div>
                  <div v-else class="grid gap-3 p-3">
                    <div v-for="event in selectedTaskEvents" :key="event.id"
                      class="grid min-w-0 gap-1 border-l-2 border-muted-foreground/30 pl-3">
                      <div class="flex flex-wrap items-center gap-2">
                        <Badge variant="outline">
                          {{ taskStatusLabel(event.status) }}
                        </Badge>
                        <span class="text-xs text-muted-foreground">
                          {{ formatDateTime(event.occurred_at) }}
                        </span>
                      </div>
                      <div class="break-words text-sm">
                        {{ event.message ?? '-' }}
                      </div>
                    </div>
                  </div>
                </ScrollArea>
              </div>
            </CardContent>
          </Card>
        </TabsContent>
      </Tabs>

      <div v-if="showExportDialog"
        class="fixed inset-0 z-50 grid place-items-center bg-background/80 p-4 backdrop-blur-sm">
        <Card class="w-full max-w-lg">
          <CardHeader class="pb-3">
            <CardTitle class="text-base">导出配置</CardTitle>
          </CardHeader>
          <CardContent class="grid gap-4">
            <div class="grid gap-3">
              <label class="flex items-center gap-2 text-sm">
                <input v-model="exportOptions.service_port" type="checkbox" />
                本地端口
              </label>
              <label class="flex items-center gap-2 text-sm">
                <input v-model="exportOptions.allowed_origins" type="checkbox" />
                Origin 白名单列表
              </label>
              <label class="flex items-center gap-2 text-sm">
                <input v-model="exportOptions.remote_enabled" type="checkbox" />
                远程任务开关
              </label>
              <label class="flex items-center gap-2 text-sm">
                <input v-model="exportOptions.remote_endpoint_url" type="checkbox" />
                远程任务 URL
              </label>
              <label class="flex items-center gap-2 text-sm">
                <input v-model="exportOptions.remote_bearer_token" type="checkbox" />
                远程任务 Authorization Token
              </label>
              <label class="flex items-center gap-2 text-sm">
                <input v-model="exportOptions.remote_poll_interval_seconds" type="checkbox" />
                轮询时间
              </label>
              <label class="flex items-center gap-2 text-sm">
                <input v-model="exportOptions.remote_max_report_retries" type="checkbox" />
                上报重试次数
              </label>
            </div>

            <div class="grid gap-2">
              <Label for="export-password">密码</Label>
              <Input id="export-password" v-model="exportPassword" type="password" />
            </div>

            <Alert v-if="showEmptyPasswordTokenConfirm" variant="warning">
              <AlertDescription>
                已选择导出 Authorization Token，且密码为空。确认后仍会导出加密文件。
              </AlertDescription>
            </Alert>

            <div class="flex flex-wrap justify-end gap-2">
              <Button variant="outline" :disabled="exportingConfig" @click="showExportDialog = false">
                取消
              </Button>
              <Button v-if="showEmptyPasswordTokenConfirm" variant="outline" :disabled="exportingConfig"
                @click="confirmEmptyPasswordExport">
                确认空密码导出
              </Button>
              <Button :disabled="exportingConfig" @click="handleExportConfig">
                {{ exportingConfig ? '导出中' : '导出' }}
              </Button>
            </div>
          </CardContent>
        </Card>
      </div>

      <div v-if="showImportDialog"
        class="fixed inset-0 z-50 grid place-items-center bg-background/80 p-4 backdrop-blur-sm">
        <div class="flex max-h-full w-full items-center justify-center">
          <Card class="flex max-h-[calc(100vh-2rem)] w-full max-w-xl flex-col">
            <CardHeader class="shrink-0 pb-3">
              <CardTitle class="text-base">导入配置</CardTitle>
            </CardHeader>
            <CardContent class="flex min-h-0 flex-1 flex-col gap-4">
              <div class="grid gap-2">
                <Label for="import-path">配置文件</Label>
                <div class="flex gap-2">
                  <Input id="import-path" class="min-w-0 flex-1" :model-value="importPath" readonly />
                  <Button variant="outline" :disabled="previewingConfigImport" @click="chooseImportFile">
                    选择
                  </Button>
                </div>
              </div>

              <div class="grid gap-2">
                <Label for="import-password">密码</Label>
                <Input id="import-password" v-model="importPassword" type="password"
                  @update:model-value="resetImportPreview" />
              </div>

              <Alert v-if="importErrorMessage" variant="error">
                <AlertDescription class="min-w-0 break-words">
                  {{ importErrorMessage }}
                </AlertDescription>
              </Alert>

              <div v-if="importPreview" class="flex min-h-0 flex-col gap-2 rounded-md border bg-muted/20 p-3">
                <div class="shrink-0 text-sm font-medium">导入变更</div>
                <div v-if="importPreview.items.length === 0" class="text-sm text-muted-foreground">
                  没有可导入的变更
                </div>
                <div v-else class="grid max-h-[35vh] min-h-0 gap-2 overflow-y-auto pr-1">
                  <div v-for="item in importPreview.items" :key="item.key" class="grid gap-1 text-sm">
                    <div class="font-medium">{{ item.label }}</div>
                    <div class="break-words text-muted-foreground">
                      {{ item.current }} -> {{ item.next }}
                    </div>
                  </div>
                </div>
              </div>

              <div class="flex shrink-0 flex-wrap justify-end gap-2">
                <Button variant="outline" :disabled="previewingConfigImport || importingConfig"
                  @click="showImportDialog = false">
                  取消
                </Button>
                <Button variant="outline" :disabled="!importPath || previewingConfigImport || importingConfig"
                  @click="handlePreviewConfigImport">
                  {{ previewingConfigImport ? '预览中' : '预览' }}
                </Button>
                <Button :disabled="!importPreview ||
                  importPreview.items.length === 0 ||
                  previewingConfigImport ||
                  importingConfig
                  " @click="handleImportConfig">
                  {{ importingConfig ? '导入中' : '确认导入' }}
                </Button>
              </div>
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  </main>
</template>
