import { cva, type VariantProps } from 'class-variance-authority';
import { LoaderCircle, type LucideIcon } from 'lucide-react';
import type { ComponentProps } from 'react';

import { cn } from '../lib/cn';

export const buttonVariants = cva(
    'inline-flex items-center justify-center border font-medium leading-none tracking-btn whitespace-nowrap select-none transition-[background-color,border-color,color,transform] active:translate-y-[.5px] disabled:pointer-events-none disabled:opacity-45 disabled:shadow-none',
    {
        variants: {
            size: {
                md: 'h-9.5 gap-2 rounded-control px-3.5 text-body max-md:h-11',
                sm: 'h-8 gap-1.5 rounded-sm px-2.5 text-ui max-md:h-11',
                icon: 'size-9.5 rounded-control max-md:size-11',
                'icon-sm': 'size-8 rounded-sm max-md:size-11',
            },
            tone: {
                ink: 'border-transparent bg-btn text-btn-ink shadow-btn hover:bg-[color-mix(in_srgb,var(--btn-bg)_86%,var(--btn-ink))]',
                secondary: 'border-line-strong bg-surface text-ink shadow-sm hover:bg-surface-2',
                ghost: 'border-transparent text-ink-2 hover:bg-hover hover:text-ink',
                danger: 'border-transparent bg-danger text-on-danger shadow-sm hover:bg-[color-mix(in_srgb,var(--danger)_88%,var(--ink))]',
            },
        },
        defaultVariants: { size: 'md', tone: 'secondary' },
    },
);

type ButtonProps = ComponentProps<'button'> &
    VariantProps<typeof buttonVariants> & {
        icon?: LucideIcon;
        /** Waiting for the request it sent: disabled, a spinner in place of its icon. */
        pending?: boolean;
    };

export function Button({
    size,
    tone,
    icon: Icon,
    pending = false,
    className,
    children,
    disabled,
    type = 'button',
    ...props
}: ButtonProps) {
    const iconSize = size === 'sm' || size === 'icon-sm' ? 14 : 16;
    return (
        <button
            type={type}
            className={cn(buttonVariants({ size, tone }), className)}
            disabled={disabled === true || pending}
            aria-busy={pending || undefined}
            {...props}
        >
            {pending ? (
                <LoaderCircle size={iconSize} strokeWidth={1.5} className="animate-spin" aria-hidden="true" />
            ) : (
                Icon && <Icon size={iconSize} strokeWidth={1.5} aria-hidden="true" />
            )}
            {children}
        </button>
    );
}
