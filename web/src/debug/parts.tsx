import { Search } from 'lucide-react';
import type { ReactNode } from 'react';

import { inputClass } from '../components/Field';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { useAtLeast } from '../lib/useAtLeast';

/** The width from which a Debug table shows every column. */
const ROOMY = 900;

/**
 * Whether a Debug table has room for every column: the first value goes on the card that holds it.
 * Without room, the column that says which resource goes under the row's name.
 */
export const useRoomy = () => useAtLeast(ROOMY);

/** How the counts of a window read: as rates, sizes and times, in the panel's own number formats. */
export const useNumbers = (seconds: number) => {
    const { t, format } = useI18n();
    const words = t.debug;
    const shown = (value: number) =>
        value === 0
            ? '0'
            : value < 0.1
              ? words.underTenth
              : format.number(value, { maximumFractionDigits: value < 10 ? 1 : 0 });
    return {
        /** A count over the window, a second. */
        rate: (count: number) => shown(count / seconds),
        /** A count over the window, a minute. */
        aMinute: (count: number) => shown((count / seconds) * 60),
        bytesRate: (bytes: number) => words.bytesRate(format.bytes(Math.round(bytes / seconds))),
        each: (bytes: number, count: number) => format.bytes(count === 0 ? 0 : Math.round(bytes / count)),
        size: (bytes: number) => format.bytes(bytes),
        /** Microseconds as milliseconds. */
        ms: (microseconds: number) =>
            format.number(microseconds / 1000, {
                maximumFractionDigits: microseconds >= 10_000 ? 0 : microseconds >= 1000 ? 1 : 2,
            }),
    };
};

/** A total of the page: what it counts, its value with its unit, and a line under it. */
export function Tile({
    label,
    value,
    unit,
    about,
    warning = false,
}: {
    label: string;
    value: string;
    unit: string;
    about: string;
    warning?: boolean;
}) {
    return (
        <div className="card flex flex-col gap-0.5 px-4 py-3.5">
            <dt className="flex items-center gap-2 font-mono text-micro tracking-[0.08em] text-ink-3 uppercase">
                {warning && <span className="led led-warning" aria-hidden="true" />}
                {label}
            </dt>
            <dd className="flex flex-col gap-0.5">
                <span className="text-[22px] leading-tight font-semibold tracking-tight text-ink tabular-nums">
                    {value}{' '}
                    <span className="text-small font-medium tracking-normal whitespace-nowrap text-ink-2">
                        {unit}
                    </span>
                </span>
                <span className="text-meta text-ink-2">{about}</span>
            </dd>
        </div>
    );
}

export function Tiles({ label, children }: { label: string; children: ReactNode }) {
    return (
        <dl aria-label={label} className="grid grid-cols-[repeat(auto-fit,minmax(10rem,1fr))] gap-3">
            {children}
        </dl>
    );
}

/** A share of the busiest row, as a bar with its figure beside it. */
export function Share({
    share,
    strong = true,
    children,
}: {
    share: number;
    strong?: boolean;
    children: string;
}) {
    return (
        <span className="flex items-center gap-2">
            <span className="h-1.5 min-w-10 flex-1 rounded-full bg-surface-3" aria-hidden="true">
                <span
                    className={cn('block h-1.5 rounded-full', strong ? 'bg-accent' : 'bg-ink-3')}
                    style={{ width: `${Math.max(2, Math.min(100, Math.round(share * 100)))}%` }}
                />
            </span>
            <span className="flex-none font-mono text-meta whitespace-nowrap text-ink-2 tabular-nums">
                {children}
            </span>
        </span>
    );
}

export function Find({
    label,
    value,
    onChange,
}: {
    label: string;
    value: string;
    onChange: (value: string) => void;
}) {
    return (
        <div className="relative ml-auto max-md:ml-0 max-md:w-full">
            <Search
                size={14}
                strokeWidth={1.5}
                className="pointer-events-none absolute top-1/2 left-2.5 -translate-y-1/2 text-ink-3"
                aria-hidden="true"
            />
            <input
                type="search"
                aria-label={label}
                placeholder={label}
                value={value}
                className={cn(inputClass, 'h-8 w-60 pl-8 text-small max-md:h-11 max-md:w-full')}
                onChange={(event) => onChange(event.target.value)}
            />
        </div>
    );
}

/** The resources of a row: one by its name, several by the first with how many more. */
export const useResources = () => {
    const { t } = useI18n();
    return (names: string[], count: number) => (count <= 1 ? (names[0] ?? '') : t.debug.resourceCount(count));
};
