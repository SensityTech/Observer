import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';

/** One part of a whole: its name, its share from 0 to 1 (`null` unmeasured), and its fill. */
export type Part = { name: string; share: number | null; fill: string };

/**
 * A whole split into parts on one bar, each part named with its share beside it: the colours
 * never speak alone. Nothing measured, or a whole of nothing, leaves the empty rail.
 */
export function Distribution({ parts, className }: { parts: readonly Part[]; className?: string }) {
    const { t, format } = useI18n();
    const measured = parts.some((part) => part.share !== null && part.share > 0);
    return (
        <div className={className}>
            <div
                className={cn('flex h-2 gap-0.5 overflow-hidden rounded-pill', !measured && 'bg-surface-3')}
                aria-hidden="true"
            >
                {measured &&
                    parts.map((part) => (
                        <span
                            key={part.name}
                            className={cn('block h-full', part.fill)}
                            style={{ width: `${(part.share ?? 0) * 100}%` }}
                        />
                    ))}
            </div>
            <div className="mt-2.25 flex flex-wrap gap-x-3.5 gap-y-1 text-meta text-ink-3">
                {parts.map((part) => (
                    <span key={part.name} className="inline-flex items-center gap-1.5">
                        <i className={cn('size-2 rounded-[2px]', part.fill)} aria-hidden="true" />
                        {t.distribution.key(
                            part.name,
                            measured && part.share !== null
                                ? format.percent(part.share * 100)
                                : t.tiles.unknown,
                        )}
                    </span>
                ))}
            </div>
        </div>
    );
}
