<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { CircleCheck, CircleX, RefreshCw, TriangleAlert, X } from '@lucide/vue';
import {
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogOverlay,
  DialogPortal,
  DialogRoot,
  DialogTitle,
} from 'reka-ui';
import { runDoctor } from '@/api';
import { groupDoctorChecks } from '@/doctor-display';
import { Button } from '@/components/ui/button';
import type { DoctorReport, DoctorStatus } from '@/types';

const open = defineModel<boolean>('open', { required: true });
const emit = defineEmits<{ 'status-change': [needsAttention: boolean] }>();
const { locale, t } = useI18n();

const loading = ref(false);
const report = ref<DoctorReport | null>(null);
const callFailed = ref(false);
const completedAt = ref<Date | null>(null);
const groups = computed(() => groupDoctorChecks(report.value?.checks ?? []));
const completedTime = computed(() =>
  completedAt.value
    ? new Intl.DateTimeFormat(locale.value, {
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
      }).format(completedAt.value)
    : '',
);

async function detect(): Promise<void> {
  if (loading.value) return;
  loading.value = true;
  callFailed.value = false;
  report.value = null;
  try {
    report.value = await runDoctor();
    completedAt.value = new Date();
    emit('status-change', report.value.summary.fail > 0);
  } catch {
    callFailed.value = true;
    emit('status-change', true);
  } finally {
    loading.value = false;
  }
}

watch(
  open,
  (isOpen) => {
    if (isOpen && !loading.value) void detect();
  },
  { immediate: true },
);

function statusClasses(status: DoctorStatus): string {
  if (status === 'PASS') {
    return 'border-emerald-200 bg-emerald-50 text-emerald-800 dark:border-emerald-900 dark:bg-emerald-950/40 dark:text-emerald-200';
  }
  if (status === 'WARN') {
    return 'border-amber-200 bg-amber-50 text-amber-900 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200';
  }
  return 'border-destructive/30 bg-destructive/10 text-destructive';
}
</script>

<template>
  <DialogRoot v-model:open="open">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-background/70 backdrop-blur-sm" />
      <DialogContent
        class="fixed inset-y-0 right-0 z-50 flex w-full max-w-md flex-col border-l bg-background shadow-xl"
      >
        <header class="flex items-start justify-between gap-4 border-b p-5">
          <div>
            <DialogTitle class="text-lg font-semibold">{{ t('doctor.title') }}</DialogTitle>
            <DialogDescription class="mt-1 text-sm text-muted-foreground">
              {{
                completedTime
                  ? t('doctor.completedAt', { time: completedTime })
                  : t('doctor.description')
              }}
            </DialogDescription>
          </div>
          <DialogClose as-child>
            <button type="button" class="rounded-md p-1" :aria-label="t('doctor.close')">
              <X class="size-4" />
            </button>
          </DialogClose>
        </header>

        <div class="min-h-0 flex-1 overflow-y-auto p-5">
          <p v-if="loading" data-testid="doctor-loading" class="text-sm text-muted-foreground">
            {{ t('doctor.running') }}
          </p>
          <div
            v-else-if="callFailed"
            class="rounded-md border border-destructive/30 bg-destructive/10 p-4 text-sm text-destructive"
          >
            {{ t('doctor.runFailed') }}
          </div>
          <template v-else-if="report">
            <div class="grid grid-cols-3 gap-2">
              <div class="rounded-md bg-emerald-50 p-3 dark:bg-emerald-950/40">
                <strong class="block text-emerald-700 dark:text-emerald-200">
                  {{ report.summary.pass }}
                </strong>
                <span class="text-xs text-muted-foreground">{{ t('doctor.passCount') }}</span>
              </div>
              <div class="rounded-md bg-amber-50 p-3 dark:bg-amber-950/40">
                <strong class="block text-amber-700 dark:text-amber-200">
                  {{ report.summary.warn }}
                </strong>
                <span class="text-xs text-muted-foreground">{{ t('doctor.warnCount') }}</span>
              </div>
              <div class="rounded-md bg-destructive/10 p-3">
                <strong class="block text-destructive">{{ report.summary.fail }}</strong>
                <span class="text-xs text-muted-foreground">{{ t('doctor.failCount') }}</span>
              </div>
            </div>

            <section v-for="group in groups" :key="group.key" class="mt-6 grid gap-2">
              <h3 class="text-xs font-semibold tracking-wide text-muted-foreground uppercase">
                {{ t(group.labelKey) }}
              </h3>
              <div
                v-for="item in group.checks"
                :key="item.check.code"
                class="flex gap-3 rounded-md border p-3 text-sm"
                :class="statusClasses(item.check.status)"
              >
                <CircleCheck v-if="item.check.status === 'PASS'" class="mt-0.5 size-4 shrink-0" />
                <TriangleAlert
                  v-else-if="item.check.status === 'WARN'"
                  class="mt-0.5 size-4 shrink-0"
                />
                <CircleX v-else class="mt-0.5 size-4 shrink-0" />
                <div class="min-w-0">
                  <p class="font-medium">{{ t(item.titleKey) }}</p>
                  <p class="mt-1 opacity-80">{{ t(item.resultKey) }}</p>
                  <code v-if="item.technicalCode" class="mt-1 block break-all text-xs opacity-70">
                    {{ item.technicalCode }}
                  </code>
                  <p v-if="item.suggestionKey" class="mt-2 text-xs">
                    {{ t(item.suggestionKey) }}
                  </p>
                </div>
              </div>
            </section>
          </template>
        </div>

        <footer class="border-t p-4">
          <Button
            data-testid="doctor-retry"
            variant="outline"
            class="w-full"
            :disabled="loading"
            @click="detect"
          >
            <RefreshCw class="size-4" :class="{ 'animate-spin': loading }" />
            {{ t('doctor.retry') }}
          </Button>
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
