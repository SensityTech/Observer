import { useMutation, useQueryClient } from '@tanstack/react-query';
import { ChevronDown, HardDrive } from 'lucide-react';
import { useId, type ReactNode } from 'react';

import { checkDisk } from '../api/changes';
import type { Comparison } from '../api/generated/Comparison';
import type { SnapshotSummary } from '../api/generated/SnapshotSummary';
import { Button } from '../components/Button';
import { inputClass } from '../components/Field';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { refreshMoved } from './live';
import type { ChangesSearch } from './search';
import { countsWords, isBusy, startWords } from './words';

/** From and To, each a start of the history, To also the disk as last checked; and the check itself. */
export function ComparisonBar({
    search,
    compared,
    starts,
    check,
    onCompare,
}: {
    search: ChangesSearch;
    compared: Comparison | undefined;
    starts: SnapshotSummary[];
    check: SnapshotSummary | null;
    onCompare: (search: ChangesSearch) => void;
}) {
    const { t, format } = useI18n();
    const words = t.changes.bar;
    const id = useId();
    const queryClient = useQueryClient();
    const run = useMutation({
        mutationFn: checkDisk,
        onSuccess: () => {
            // The check before is gone: nothing compared with it may stay on screen.
            refreshMoved(queryClient, { starts: true, to: new Set(['disk']) });
            onCompare({ to: 'disk' });
        },
    });
    const checking = check !== null && isBusy(check);

    // A start compared stays listed even when it is past the pages loaded.
    const listed = [compared?.to, compared?.from].reduce<SnapshotSummary[]>(
        (list, extra) =>
            extra !== undefined && extra.kind === 'start' && !list.some((start) => start.id === extra.id)
                ? [...list, extra].toSorted((a, b) => b.id - a.id)
                : list,
        starts,
    );
    const option = (start: SnapshotSummary) =>
        words.option(
            format.timestamp(start.takenAt, { seconds: false }),
            startWords(t, start),
            countsWords(t, start),
        );
    const fromValue = String(compared?.from.id ?? search.from ?? '');
    const toValue = search.to === 'disk' ? 'disk' : String(compared?.to.id ?? search.to ?? '');
    let diskWords = words.disk;
    if (check !== null) {
        diskWords = checking
            ? words.diskChecking
            : words.diskAt(format.timestamp(check.takenAt, { seconds: false }));
    }

    return (
        <section className="card" aria-labelledby={`${id}-title`}>
            <h2 id={`${id}-title`} className="sr-only">
                {words.title}
            </h2>
            <div className="grid grid-cols-1 items-end gap-3 p-4 md:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto]">
                <StartSelect
                    id={`${id}-from`}
                    label={words.from}
                    value={fromValue}
                    onChange={(value) => onCompare({ from: Number(value), to: search.to ?? compared?.to.id })}
                >
                    {listed.map((start) => (
                        <option key={start.id} value={String(start.id)}>
                            {option(start)}
                        </option>
                    ))}
                </StartSelect>
                <StartSelect
                    id={`${id}-to`}
                    label={words.to}
                    value={toValue}
                    onChange={(value) =>
                        onCompare({ from: search.from, to: value === 'disk' ? 'disk' : Number(value) })
                    }
                >
                    {(check !== null || search.to === 'disk') && <option value="disk">{diskWords}</option>}
                    {listed.map((start) => (
                        <option key={start.id} value={String(start.id)}>
                            {option(start)}
                        </option>
                    ))}
                </StartSelect>
                <Button
                    icon={HardDrive}
                    pending={run.isPending || checking}
                    title={words.checkHint}
                    onClick={() => run.mutate()}
                >
                    {checking ? words.checking : words.check}
                </Button>
            </div>
            {run.isError && (
                <p role="alert" className="px-4 pb-4 text-small text-danger-ink">
                    {errorMessage(t, run.error)}
                </p>
            )}
        </section>
    );
}

function StartSelect({
    id,
    label,
    value,
    onChange,
    children,
}: {
    id: string;
    label: string;
    value: string;
    onChange: (value: string) => void;
    children: ReactNode;
}) {
    return (
        <div className="flex min-w-0 flex-col gap-1.5">
            <label htmlFor={id} className="text-ui font-medium text-ink">
                {label}
            </label>
            <div className="relative">
                <select
                    id={id}
                    className={cn(inputClass, 'appearance-none truncate pr-9')}
                    value={value}
                    onChange={(event) => onChange(event.target.value)}
                >
                    {value === '' && (
                        <option value="" disabled>
                            {label}
                        </option>
                    )}
                    {children}
                </select>
                <ChevronDown
                    size={16}
                    strokeWidth={1.5}
                    className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2 text-ink-3"
                    aria-hidden="true"
                />
            </div>
        </div>
    );
}
