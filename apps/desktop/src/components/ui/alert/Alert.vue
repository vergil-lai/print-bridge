<script setup lang="ts">
import type { HTMLAttributes } from 'vue';
import type { AlertVariants } from '.';
import { cn } from '@/lib/utils';
import { alertVariants } from '.';

const props = defineProps<{
  variant?: AlertVariants['variant'];
  class?: HTMLAttributes['class'];
}>();
</script>

<template>
  <div role="alert" data-slot="alert" :class="cn(alertVariants({ variant }), props.class)">
    <slot />
  </div>
</template>
