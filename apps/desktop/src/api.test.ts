// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

import { runDoctor } from '@/api';
import type { DoctorReport } from '@/types';

describe('runDoctor', () => {
  beforeEach(() => mocks.invoke.mockReset());

  it('invokes the Desktop doctor Tauri command without a payload', async () => {
    const report: DoctorReport = {
      checks: [],
      summary: { pass: 0, warn: 0, fail: 0 },
    };
    mocks.invoke.mockResolvedValue(report);

    await expect(runDoctor()).resolves.toEqual(report);
    expect(mocks.invoke).toHaveBeenCalledWith('run_doctor');
  });
});
