import { useRef, type ReactNode } from 'react';

import { FieldError, inputClass } from '../components/Field';
import { MessagePreview } from '../components/MessagePreview';
import { cn } from '../lib/cn';
import { fill } from '../lib/fill';
import type { RowIds } from './Row';

/**
 * A message of the owner's that a player reads: its text, the placeholders to insert where the caret
 * is, and how it reads once they are filled in.
 */
export function MessageField<Key extends string>({
    ids,
    value,
    problem,
    placeholders,
    example,
    words,
    onChange,
    children,
}: {
    ids: RowIds;
    value: string;
    /** Why the message cannot be saved, once saving was tried. */
    problem: string | null;
    placeholders: readonly Key[];
    /** What each placeholder reads in the preview. */
    example: Record<Key, string>;
    words: { insert: string; preview: string };
    onChange: (value: string) => void;
    /** Said between the placeholders and the preview. */
    children?: ReactNode;
}) {
    const field = useRef<HTMLTextAreaElement>(null);
    const insert = (key: Key) => {
        const area = field.current;
        const text = `{${key}}`;
        const start = area?.selectionStart ?? value.length;
        const end = area?.selectionEnd ?? value.length;
        onChange(value.slice(0, start) + text + value.slice(end));
        requestAnimationFrame(() => {
            area?.focus();
            area?.setSelectionRange(start + text.length, start + text.length);
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
                <span className="mr-1 text-small text-ink-3">{words.insert}</span>
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
            <MessagePreview caption={words.preview} message={fill(value.trim(), example)} />
        </>
    );
}
