import type { VariantProps } from 'class-variance-authority';
import { cva } from 'class-variance-authority';

export { default as Alert } from './Alert.vue';
export { default as AlertDescription } from './AlertDescription.vue';
export { default as AlertTitle } from './AlertTitle.vue';

export const alertVariants = cva('relative w-full rounded-lg border px-4 py-3 text-sm shadow-sm', {
  variants: {
    variant: {
      default: 'bg-card text-card-foreground',
      success: 'border-emerald-500/40 bg-emerald-500/15 text-emerald-900 dark:text-emerald-100',
      warning: 'border-amber-500/40 bg-amber-500/15 text-amber-900 dark:text-amber-100',
      error: 'border-destructive/40 bg-destructive/15 text-destructive dark:text-red-200',
    },
  },
  defaultVariants: {
    variant: 'default',
  },
});

export type AlertVariants = VariantProps<typeof alertVariants>;
