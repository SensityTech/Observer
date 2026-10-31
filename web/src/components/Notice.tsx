import { cva, type VariantProps } from 'class-variance-authority';
import { CircleAlert, CircleCheck, Info, TriangleAlert } from 'lucide-react';
import type { ReactNode, Ref } from 'react';

import { cn } from '../lib/cn';

const noticeVariants = cva('flex items-start gap-2.5 rounded-box border px-3.5 py-3 text-small', {
    variants: {
        tone: {
            neutral: 'border-line bg-surface-2 text-ink-2',
            success: 'border-transparent bg-success-soft text-success',
            warning: 'border-transparent bg-warning-soft text-warning',
            danger: 'border-transparent bg-danger-soft text-danger-ink',
        },
    },
    defaultVariants: { tone: 'neutral' },
});

const ICONS = { neutral: Info, success: CircleCheck, warning: TriangleAlert, danger: CircleAlert };

type NoticeProps = VariantProps<typeof noticeVariants> & {
    children: ReactNode;
    className?: string;
    ref?: Ref<HTMLDivElement>;
    /** -1 for a notice the keyboard is moved to, when what it was on went away. */
    tabIndex?: number;
    /**
     * False for a notice that stays on the page and changes with figures read again, which would
     * be announced each time: it is read where it stands, never as a live region.
     */
    live?: boolean;
};

/** A sentence about what just happened, above or below the thing it concerns. */
export function Notice({ tone, className, children, ref, tabIndex, live = true }: NoticeProps) {
    const Icon = ICONS[tone ?? 'neutral'];
    return (
        <div
            ref={ref}
            tabIndex={tabIndex}
            role={live ? (tone === 'danger' || tone === 'warning' ? 'alert' : 'status') : undefined}
            className={cn(noticeVariants({ tone }), className)}
        >
            <Icon size={16} strokeWidth={1.5} className="mt-px flex-none" aria-hidden="true" />
            <div className="min-w-0 flex-1">{children}</div>
        </div>
    );
}
