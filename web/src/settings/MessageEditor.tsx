import { useRef, type ReactNode } from 'react';

import { FieldError, inputClass } from '../components/Field';
import { MessagePreview } from '../components/MessagePreview';
import { cn } from '../lib/cn';
import type { RowIds } from './Row';

/**
 * A message the owner writes for players: its text, the placeholders a click inserts where the
 * cursor stands, and the message as a player reads it, with example values.
 */
export function MessageEditor<K extends string>({
    ids,
    value,
    onChange,
    placeholders,
    insertLabel,
    problem,
    previewLabel,
    preview,
    children,
}: {
    ids: RowIds;
    value: string;
    onChange: (value: string) => void;
    placeholders: readonly K[];
    insertLabel: string;
    /** What keeps the message from being saved, once a save was tried. */
    problem: string | null;
    previewLabel: string;
    preview: string;
    /** Said between the placeholders and the preview. */
    children?: ReactNode;
}) {
    const field = useRef<HTMLTextAreaElement>(null);
    const insert = (key: K) => {
        const text = `{${key}}`;
        const start = field.current?.selectionStart ?? value.length;
        const end = field.current?.selectionEnd ?? value.length;
        onChange(value.slice(0, start) + text + value.slice(end));
        requestAnimationFrame(() => {
            field.current?.focus();
            field.current?.setSelectionRange(start + text.length, start + text.length);
        });
    };

    return (
        <>
            <textarea
                ref={field}
                id={ids.control}
                rows={Math.min(10, Math.max(4, value.split('\n').length + 1))}
                aria-invalid={problem !== null || undefined}
                aria-describedby={problem !== null ? `${ids.control}-error ${ids.about}` : ids.about}
                className={cn(inputClass, 'h-auto min-h-28 resize-y py-2.5 font-mono text-ui leading-[1.55]')}
                value={value}
                onChange={(event) => onChange(event.target.value)}
            />
            {problem !== null && <FieldError id={`${ids.control}-error`}>{problem}</FieldError>}
            <div className="flex flex-wrap items-center gap-1.5">
                <span className="mr-1 text-small text-ink-3">{insertLabel}</span>
                {placeholders.map((key) => (
                    <button
                        key={key}
                        type="button"
                        className="inline-flex h-7 items-center rounded-xs border border-dashed border-line-strong px-2 font-mono text-small text-accent-ink hover:bg-hover max-md:h-11"
                        onClick={() => insert(key)}
                    >
                        {`{${key}}`}
                    </button>
                ))}
            </div>
            {children}
            <MessagePreview caption={previewLabel} message={preview} />
        </>
    );
}
