import { Check, Copy } from 'lucide-react';
import { useId, useRef, useState } from 'react';

import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { Button } from './Button';
import { inputClass } from './Field';

/** A secret shown once: readable, selectable, and copied in one click where the browser allows it. */
export function CopyField({ label, value }: { label: string; value: string }) {
    const { t } = useI18n();
    const id = useId();
    const input = useRef<HTMLInputElement>(null);
    const [state, setState] = useState<'idle' | 'copied' | 'blocked'>('idle');

    const copy = async () => {
        try {
            await navigator.clipboard.writeText(value);
            setState('copied');
        } catch {
            // No clipboard outside a secure context (plain HTTP on a LAN address): select it instead.
            input.current?.select();
            setState('blocked');
        }
    };

    return (
        <div className="flex flex-col gap-1.5">
            <label htmlFor={id} className="text-ui font-medium text-ink">
                {label}
            </label>
            <div className="flex gap-2">
                <input
                    ref={input}
                    id={id}
                    readOnly
                    value={value}
                    spellCheck={false}
                    className={cn(inputClass, 'min-w-0 font-mono text-ui [font-variant-ligatures:none]')}
                    onFocus={(event) => event.currentTarget.select()}
                />
                <Button icon={state === 'copied' ? Check : Copy} onClick={copy}>
                    {state === 'copied' ? t.common.copied : t.common.copy}
                </Button>
            </div>
            {state === 'blocked' && (
                <output className="block text-small text-ink-3">{t.common.copyBlocked}</output>
            )}
        </div>
    );
}
