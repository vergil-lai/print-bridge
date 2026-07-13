<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { Button } from '@/components/ui/button';
import type { CliIntegrationStatus } from '@/types';

const props = defineProps<{ status: CliIntegrationStatus; loading: boolean }>();
const emit = defineEmits<{ install: []; uninstall: [] }>();
const { t } = useI18n();

const statusLabel = computed(() => t(`cliStatus_${props.status.kind}`));
const action = computed<'install' | 'uninstall' | null>(() => {
  if (props.status.kind === 'installed') return 'uninstall';
  if (props.status.kind === 'not_installed' || props.status.kind === 'stale') return 'install';
  return null;
});

function handleAction(): void {
  if (action.value === 'install') emit('install');
  if (action.value === 'uninstall') emit('uninstall');
}
</script>

<template>
  <div class="flex min-h-14 flex-col gap-2 rounded-md border px-3 py-2 md:flex-row md:items-center md:justify-between">
    <div class="min-w-0">
      <p class="text-sm font-medium">{{ t('commandLineTool') }}</p>
      <p class="mt-1 truncate text-xs text-muted-foreground">
        {{ statusLabel }}<template v-if="status.command_path"> · {{ status.command_path }}</template>
      </p>
      <p v-if="!status.path_ready && status.kind === 'installed'" class="mt-1 text-xs text-amber-600">
        {{ t('cliPathHint') }}
      </p>
    </div>
    <Button
      v-if="action"
      variant="outline"
      class="shrink-0"
      :data-action="action"
      :disabled="loading"
      @click="handleAction"
    >
      {{ loading ? t('processing') : action === 'install' ? t(status.kind === 'stale' ? 'reinstallCli' : 'installCli') : t('uninstallCli') }}
    </Button>
  </div>
</template>
