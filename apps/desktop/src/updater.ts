import { relaunch } from '@tauri-apps/plugin-process';
import { check, type DownloadEvent, type Update } from '@tauri-apps/plugin-updater';

export type UpdateStatus =
  | 'idle'
  | 'checking'
  | 'available'
  | 'not-available'
  | 'downloading'
  | 'installed'
  | 'error';

export interface UpdateInfo {
  version: string;
  currentVersion: string;
  date?: string;
  body?: string;
}

export interface UpdateProgress {
  downloadedBytes: number;
  contentLength: number | null;
}

export interface DownloadUpdateResult {
  downloadedBytes: number;
  contentLength: number | null;
}

/** 检查配置的 Tauri 更新端点是否有可用更新。 */
export async function checkForAppUpdate(): Promise<Update | null> {
  return check();
}

/** 把 Tauri 更新对象映射成 UI 需要的精简结构。 */
export function toUpdateInfo(update: Update): UpdateInfo {
  return {
    version: update.version,
    currentVersion: update.currentVersion,
    date: update.date,
    body: update.body,
  };
}

/** 下载并安装更新，同时向 UI 回报字节进度。 */
export async function downloadAndInstallAppUpdate(
  update: Update,
  onProgress: (progress: UpdateProgress) => void,
): Promise<DownloadUpdateResult> {
  let downloadedBytes = 0;
  let contentLength: number | null = null;

  await update.downloadAndInstall((event: DownloadEvent) => {
    if (event.event === 'Started') {
      downloadedBytes = 0;
      contentLength = event.data.contentLength ?? null;
      onProgress({ downloadedBytes, contentLength });
      return;
    }

    if (event.event === 'Progress') {
      downloadedBytes += event.data.chunkLength;
      onProgress({ downloadedBytes, contentLength });
    }
  });

  return { downloadedBytes, contentLength };
}

/** 更新安装完成后重启桌面应用。 */
export function relaunchApp(): Promise<void> {
  return relaunch();
}
