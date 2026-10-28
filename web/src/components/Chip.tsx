import { cva, type VariantProps } from 'class-variance-authority';
import type { LucideIcon } from 'lucide-react';
import type { ReactNode } from 'react';

import { cn } from '../lib/cn';

export const chipVariants = cva(
    'inline-flex h-5.5 items-center gap-1.25 rounded-pill border px-2 text-meta leading-none font-medium whitespace-nowrap',
    {
        variants: {
            tone: {
                neutral: 'border-line bg-surface-2 text-ink-2',
                success: 'border-transparent bg-success-soft text-success',
                warning: 'border-transparent bg-warning-soft text-warning',
                danger: 'border-transparent bg-danger-soft text-danger-ink',
            },
        },
        defaultVariants: { tone: 'neutral' },
    },
);

type ChipProps = VariantProps<typeof chipVariants> & {
    icon?: LucideIcon;
    className?: string;
    children: ReactNode;
};

/** A word, with an icon when it warns: a colour never carries a state alone. */
export function Chip({ tone, icon: Icon, className, children }: ChipProps) {
    return (
        <span className={cn(chipVariants({ tone }), className)}>
            {Icon && <Icon size={12} strokeWidth={2} aria-hidden="true" />}
            {children}
        </span>
    );
}
