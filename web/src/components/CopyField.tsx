import { Check, Copy } from 'lucide-react';
import { useId, useRef } from 'react';

import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { useCopy } from '../lib/useCopy';
import { Button } from './Button';
import { inputClass } from './Field';

/** A secret shown once: readable, selectable, and copied in one click where the browser allows it. */
export function CopyField({ label, value }: { label: string; value: string }) {
    const { t } = useI18n();
    const id = useId();
    const input = useRef<HTMLInputElement>(null);
    const { state, copy } = useCopy({ kept: true });

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
                <Button
                    icon={state === 'copied' ? Check : Copy}
                    onClick={async () => {
                        // Where the browser refuses, the value is selected instead.
                        if (!(await copy(value))) {
                            input.current?.select();
                        }
                    }}
                >
                    {state === 'copied' ? t.common.copied : t.common.copy}
                </Button>
            </div>
            {state === 'blocked' && (
                <output className="block text-small text-ink-3">{t.common.copyBlocked}</output>
            )}
        </div>
    );
}
