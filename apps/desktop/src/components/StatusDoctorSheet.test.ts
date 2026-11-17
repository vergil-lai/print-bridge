// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushPromises, mount } from '@vue/test-utils';
import { i18n, setI18nLocale } from '@/i18n';
import type { DoctorReport } from '@/types';
import StatusDoctorSheet from './StatusDoctorSheet.vue';

const mocks = vi.hoisted(() => ({
  runDoctor: vi.fn(),
  exportDiagnosticReport: vi.fn(),
  save: vi.fn(),
}));
vi.mock('@/api', () => ({
  runDoctor: mocks.runDoctor,
  exportDiagnosticReport: mocks.exportDiagnosticReport,
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: mocks.save }));

const report = (status: 'PASS' | 'WARN' | 'FAIL', code = 'printing.printers'): DoctorReport => ({
  checks: [{ code, status, message: 'backend English', suggestion: 'backend suggestion' }],
  summary: {
    pass: status === 'PASS' ? 1 : 0,
    warn: status === 'WARN' ? 1 : 0,
    fail: status === 'FAIL' ? 1 : 0,
  },
});

function lastStatusChange(wrapper: ReturnType<typeof mount>): unknown[] | undefined {
  const events = wrapper.emitted('status-change') ?? [];
  return events[events.length - 1];
}

describe('StatusDoctorSheet', () => {
  beforeEach(() => {
    mocks.runDoctor.mockReset();
    mocks.exportDiagnosticReport.mockReset();
    mocks.save.mockReset();
    setI18nLocale('zh-CN');
  });

  afterEach(() => {
    document.body.innerHTML = '';
  });

  it('exports to the chosen path, supports cancellation and reports write errors', async () => {
    mocks.runDoctor.mockResolvedValue(report('PASS'));
    mount(StatusDoctorSheet, {
      attachTo: document.body,
      global: { plugins: [i18n] },
      props: { open: true },
    });
    await flushPromises();
    const button = document.querySelector<HTMLButtonElement>('[data-testid="doctor-export"]')!;
    mocks.save.mockResolvedValueOnce(null);
    button.click();
    await flushPromises();
    expect(mocks.exportDiagnosticReport).not.toHaveBeenCalled();
    expect(button.disabled).toBe(false);

    mocks.save.mockResolvedValue('/tmp/report.json');
    mocks.exportDiagnosticReport.mockResolvedValueOnce(undefined);
    button.click();
    await flushPromises();
    expect(mocks.exportDiagnosticReport).toHaveBeenCalledWith('/tmp/report.json');
    expect(document.body.textContent).toContain('诊断报告已导出。');

    mocks.exportDiagnosticReport.mockRejectedValueOnce(new Error('disk full'));
    button.click();
    await flushPromises();
    expect(document.body.textContent).toContain('诊断报告导出失败，请重试。');
    expect(document.body.textContent).not.toContain('诊断报告已导出。');
    expect(button.disabled).toBe(false);
  });

  it('runs on open, renders localized warnings, and keeps the app ready', async () => {
    mocks.runDoctor.mockResolvedValue(report('WARN'));
    const wrapper = mount(StatusDoctorSheet, {
      attachTo: document.body,
      global: { plugins: [i18n] },
      props: { open: true },
    });

    await flushPromises();

    expect(mocks.runDoctor).toHaveBeenCalledTimes(1);
    expect(document.body.textContent).toContain('未检测到打印机');
    expect(document.body.textContent).toContain('安装打印机并检查系统打印服务');
    expect(document.body.textContent).not.toContain('backend English');
    expect(lastStatusChange(wrapper)).toEqual([false]);
  });

  it('reports FAIL and call errors as needing attention', async () => {
    mocks.runDoctor.mockResolvedValueOnce(report('FAIL'));
    const wrapper = mount(StatusDoctorSheet, {
      attachTo: document.body,
      global: { plugins: [i18n] },
      props: { open: true },
    });
    await flushPromises();
    expect(lastStatusChange(wrapper)).toEqual([true]);

    mocks.runDoctor.mockRejectedValueOnce(new Error('raw backend error'));
    document.querySelector<HTMLElement>('[data-testid="doctor-retry"]')!.click();
    await flushPromises();
    expect(document.body.textContent).toContain('状态检测失败，请重试。');
    expect(document.body.textContent).not.toContain('raw backend error');
    expect(lastStatusChange(wrapper)).toEqual([true]);
  });

  it('keeps unknown checks visible and rerenders language without rerunning', async () => {
    mocks.runDoctor.mockResolvedValue(report('WARN', 'future.check'));
    mount(StatusDoctorSheet, {
      attachTo: document.body,
      global: { plugins: [i18n] },
      props: { open: true },
    });
    await flushPromises();
    expect(document.body.textContent).toContain('future.check');

    setI18nLocale('en');
    await flushPromises();
    expect(document.body.textContent).toContain('This check needs attention');
    expect(mocks.runDoctor).toHaveBeenCalledTimes(1);
  });

  it('reuses an in-flight run but starts a fresh run after completion', async () => {
    let resolveRun!: (value: DoctorReport) => void;
    const pending = new Promise<DoctorReport>((resolve) => {
      resolveRun = resolve;
    });
    mocks.runDoctor.mockReturnValue(pending);
    const wrapper = mount(StatusDoctorSheet, {
      attachTo: document.body,
      global: { plugins: [i18n] },
      props: { open: true },
    });

    await wrapper.setProps({ open: false });
    await wrapper.setProps({ open: true });
    expect(mocks.runDoctor).toHaveBeenCalledTimes(1);

    resolveRun(report('PASS'));
    await flushPromises();
    await wrapper.setProps({ open: false });
    await wrapper.setProps({ open: true });
    await flushPromises();
    expect(mocks.runDoctor).toHaveBeenCalledTimes(2);
  });
});
