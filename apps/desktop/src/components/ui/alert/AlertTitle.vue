<script setup lang="ts">
import type { HTMLAttributes } from 'vue';
import { cn } from '@/lib/utils';

const props = defineProps<{
  class?: HTMLAttributes['class'];
}>();
</script>

<template>
  <div
    data-slot="alert-title"
    :class="cn('mb-1 font-medium leading-none tracking-normal', props.class)"
  >
    <slot />
  </div>
</template>
