import { cva, type VariantProps } from 'class-variance-authority';
import { CircleAlert, CircleCheck, Info, TriangleAlert } from 'lucide-react';
import type { ReactNode } from 'react';

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
};

/** A sentence about what just happened, above or below the thing it concerns. */
export function Notice({ tone, className, children }: NoticeProps) {
    const Icon = ICONS[tone ?? 'neutral'];
    return (
        <div
            role={tone === 'danger' || tone === 'warning' ? 'alert' : 'status'}
            className={cn(noticeVariants({ tone }), className)}
        >
            <Icon size={16} strokeWidth={1.5} className="mt-px flex-none" aria-hidden="true" />
            <div className="min-w-0 flex-1">{children}</div>
        </div>
    );
}
