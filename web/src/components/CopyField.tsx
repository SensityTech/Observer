import { Check, Copy } from 'lucide-react';
import { useId, useRef, useState } from 'react';

import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { Button } from './Button';
import { inputClass } from './Field';

/**
 * A value to take elsewhere, a secret shown once or lines for a file: readable, selectable, and
 * copied in one click where the browser allows it. `multiline` keeps its line breaks.
 */
export function CopyField({
    label,
    value,
    multiline = false,
}: {
    label: string;
    value: string;
    multiline?: boolean;
}) {
    const { t } = useI18n();
    const id = useId();
    const field = useRef<HTMLInputElement & HTMLTextAreaElement>(null);
    const [state, setState] = useState<'idle' | 'copied' | 'blocked'>('idle');

    const copy = async () => {
        try {
            await navigator.clipboard.writeText(value);
            setState('copied');
        } catch {
            // No clipboard outside a secure context (plain HTTP on a LAN address): select it instead.
            field.current?.select();
            setState('blocked');
        }
    };

    return (
        <div className="flex flex-col gap-1.5">
            <label htmlFor={id} className="text-ui font-medium text-ink">
                {label}
            </label>
            <div className={cn('flex items-start gap-2', multiline && 'flex-wrap')}>
                {multiline ? (
                    <textarea
                        ref={field}
                        id={id}
                        readOnly
                        rows={value.split('\n').length}
                        value={value}
                        spellCheck={false}
                        wrap="off"
                        className={cn(
                            inputClass,
                            // Wide enough for its lines, or a line of its own with the button under it.
                            'h-auto min-w-0 flex-[1_1_17rem] resize-none py-2 font-mono text-ui [font-variant-ligatures:none] max-md:h-auto',
                        )}
                        onFocus={(event) => event.currentTarget.select()}
                    />
                ) : (
                    <input
                        ref={field}
                        id={id}
                        readOnly
                        value={value}
                        spellCheck={false}
                        className={cn(inputClass, 'min-w-0 font-mono text-ui [font-variant-ligatures:none]')}
                        onFocus={(event) => event.currentTarget.select()}
                    />
                )}
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
