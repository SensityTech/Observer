import { Check, Copy } from 'lucide-react';
import { useEffect, useId, useState } from 'react';

import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';

/** How long "Copied" stays under the value. */
const COPIED_FOR = 2_000;

/**
 * A long value shown short, in mono, and copied whole in one click where the browser allows it.
 * Where it does not, the line below gives the whole value to select.
 */
export function CopyValue({ value, shown, label }: { value: string; shown: string; label: string }) {
    const { t } = useI18n();
    const id = useId();
    const [state, setState] = useState<'idle' | 'copied' | 'blocked'>('idle');
    useEffect(() => {
        if (state !== 'copied') {
            return;
        }
        const timer = setTimeout(() => setState('idle'), COPIED_FOR);
        return () => clearTimeout(timer);
    }, [state]);
    const copy = async () => {
        try {
            await navigator.clipboard.writeText(value);
            setState('copied');
        } catch {
            // No clipboard outside a secure context (plain HTTP on a LAN address).
            setState('blocked');
        }
    };
    return (
        <span className="inline-flex max-w-full flex-col">
            <button
                type="button"
                title={value}
                aria-label={label}
                aria-describedby={id}
                className="inline-flex items-center gap-1 self-start rounded-xs font-mono text-small whitespace-nowrap text-ink [font-variant-ligatures:none] hover:text-ink-2 max-md:min-h-11"
                onClick={() => void copy()}
            >
                {shown}
                {state === 'copied' ? (
                    <Check size={11} strokeWidth={1.75} aria-hidden="true" />
                ) : (
                    <Copy size={11} strokeWidth={1.75} className="text-ink-3" aria-hidden="true" />
                )}
            </button>
            <output
                id={id}
                className={cn(
                    'block text-meta text-ink-3 [overflow-wrap:anywhere]',
                    state === 'idle' && 'sr-only',
                )}
            >
                {state === 'copied'
                    ? t.common.copied
                    : state === 'blocked'
                      ? t.common.copyBlockedValue(value)
                      : ''}
            </output>
        </span>
    );
}
