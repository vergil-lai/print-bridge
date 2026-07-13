// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import { i18n } from '@/i18n';
import CliIntegrationRow from './CliIntegrationRow.vue';

describe('CliIntegrationRow', () => {
  it('hides actions when Linux installed the command through the system package', () => {
    const wrapper = mount(CliIntegrationRow, {
      global: { plugins: [i18n] },
      props: {
        status: {
          kind: 'installed_system',
          command_path: '/usr/bin/print-bridge',
          path_ready: true,
        },
        loading: false,
      },
    });

    expect(wrapper.findAll('button')).toHaveLength(0);
  });

  it('shows the matching action for manageable states', async () => {
    const wrapper = mount(CliIntegrationRow, {
      global: { plugins: [i18n] },
      props: {
        status: {
          kind: 'not_installed',
          command_path: '/usr/local/bin/print-bridge',
          path_ready: true,
        },
        loading: false,
      },
    });
    expect(wrapper.get('button').attributes('data-action')).toBe('install');

    await wrapper.setProps({ status: { ...wrapper.props('status'), kind: 'installed' } });
    expect(wrapper.get('button').attributes('data-action')).toBe('uninstall');

    await wrapper.setProps({ status: { ...wrapper.props('status'), kind: 'stale' } });
    expect(wrapper.get('button').attributes('data-action')).toBe('install');
  });
});
