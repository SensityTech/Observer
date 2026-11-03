import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { useId } from 'react';

import type { DropsRange } from '../../api/generated/DropsRange';
import type { SessionSurvival } from '../../api/generated/SessionSurvival';
import { sessionsQuery } from '../../api/statistics';
import { formatShare } from '../../drops/words';
import { formatDay } from '../../i18n/format';
import { useI18n } from '../../i18n/I18nProvider';
import { useSchedule } from '../../schedule/context';
import { CardState, InsightCard } from './InsightCard';

const WIDTH = 600;
const HEIGHT = 230;
const LEFT = 40;
const RIGHT = 590;
const TOP = 20;
const BOTTOM = 190;
const MINUTES = 240;
const STEP = 2;
const EARLY = 10;

const x = (minutes: number) => LEFT + (Math.min(minutes, MINUTES) / MINUTES) * (RIGHT - LEFT);
const y = (share: number) => BOTTOM - share * (BOTTOM - TOP);

/** A survival curve as steps: each share holds until the next mark. */
export const survivalPath = (points: readonly number[]) =>
    points
        .map((share, index) => {
            const at = index * STEP;
            return index === 0 ? `M${x(at)} ${y(share)}` : `H${x(at)} V${y(share).toFixed(1)}`;
        })
        .join(' ');

export function SessionsCard({ range }: { range: DropsRange }) {
    const { t } = useI18n();
    const s = t.statistics.sessions;
    const query = useQuery({ ...sessionsQuery(range), placeholderData: keepPreviousData });
    const data = query.data;
    return (
        <InsightCard title={s.title} meta={s.meta} stale={query.isPlaceholderData}>
            {data === undefined || data.now.sessions === 0 ? (
                <CardState pending={query.isPending} error={query.error} failed={s.failed} empty={s.empty} />
            ) : (
                <Curves survival={data} />
            )}
        </InsightCard>
    );
}

function Curves({ survival }: { survival: SessionSurvival }) {
    const { t, locale, format } = useI18n();
    const s = t.statistics.sessions;
    const zone = useSchedule().zone ?? 'UTC';
    const summaryId = useId();
    const median = (value: number | null) => (value === null ? s.longer : format.duration(value * 60));
    const share = (value: number | null) =>
        value === null ? t.statistics.instant.unknown : formatShare(locale, value);
    const hasBefore = survival.before.sessions > 0;
    const marks = [60, 120, 180];
    const cut = Date.parse(survival.keptFrom) > Date.parse(survival.before.from) - 1000 && hasBefore;
    return (
        <figure className="flex flex-col gap-2">
            <svg viewBox={`0 0 ${WIDTH} ${HEIGHT}`} className="w-full" aria-hidden="true">
                <rect
                    x={LEFT}
                    y={TOP}
                    width={x(EARLY) - LEFT}
                    height={BOTTOM - TOP}
                    className="fill-warning-soft"
                />
                <text x={x(EARLY) + 6} y={BOTTOM - 6} className="fill-warning font-mono text-micro">
                    {s.first}
                </text>
                <line
                    x1={LEFT}
                    x2={RIGHT}
                    y1={y(0.5)}
                    y2={y(0.5)}
                    className="stroke-ink-3"
                    strokeDasharray="4 3"
                />
                <text x={LEFT - 4} y={y(1) + 4} textAnchor="end" className="fill-ink-3 font-mono text-micro">
                    {formatShare(locale, 1)}
                </text>
                <text
                    x={LEFT - 4}
                    y={y(0.5) + 4}
                    textAnchor="end"
                    className="fill-ink-3 font-mono text-micro"
                >
                    {formatShare(locale, 0.5)}
                </text>
                {hasBefore && (
                    <path
                        d={survivalPath(survival.before.points)}
                        fill="none"
                        className="stroke-ink-3"
                        strokeWidth={1.5}
                        strokeDasharray="5 4"
                    />
                )}
                <path
                    d={survivalPath(survival.now.points)}
                    fill="none"
                    className="stroke-accent"
                    strokeWidth={2}
                />
                {survival.now.median !== null && (
                    <g>
                        <line
                            x1={x(survival.now.median)}
                            x2={x(survival.now.median)}
                            y1={y(0.5)}
                            y2={BOTTOM}
                            className="stroke-accent"
                            strokeDasharray="2 3"
                        />
                        <circle
                            cx={x(survival.now.median)}
                            cy={y(0.5)}
                            r={4}
                            className="fill-accent stroke-surface"
                            strokeWidth={2}
                        />
                        <text
                            x={x(survival.now.median) - 8}
                            y={y(0.5) - 8}
                            textAnchor="end"
                            className="fill-ink font-mono text-eyebrow font-semibold"
                        >
                            {median(survival.now.median)}
                        </text>
                    </g>
                )}
                {hasBefore && survival.before.median !== null && (
                    <g>
                        <line
                            x1={x(survival.before.median)}
                            x2={x(survival.before.median)}
                            y1={y(0.5)}
                            y2={BOTTOM}
                            className="stroke-ink-3"
                            strokeDasharray="2 3"
                        />
                        <text
                            x={x(survival.before.median) + 8}
                            y={y(0.5) - 8}
                            className="fill-ink-2 font-mono text-eyebrow"
                        >
                            {s.usual(median(survival.before.median))}
                        </text>
                    </g>
                )}
                <line x1={LEFT} x2={RIGHT} y1={BOTTOM} y2={BOTTOM} className="stroke-line-strong" />
                <text
                    x={LEFT}
                    y={BOTTOM + 18}
                    textAnchor="middle"
                    className="fill-ink-3 font-mono text-micro"
                >
                    0
                </text>
                {marks.map((minutes) => (
                    <text
                        key={minutes}
                        x={x(minutes)}
                        y={BOTTOM + 18}
                        textAnchor="middle"
                        className="fill-ink-3 font-mono text-micro"
                    >
                        {format.duration(minutes * 60)}
                    </text>
                ))}
                <text x={RIGHT} y={BOTTOM + 18} textAnchor="end" className="fill-ink-3 font-mono text-micro">
                    {format.duration(MINUTES * 60)}
                </text>
            </svg>
            <div className="flex flex-wrap gap-x-4 gap-y-1 text-small text-ink-3" aria-hidden="true">
                <span className="flex items-center gap-1.5">
                    <span className="h-0.5 w-3.5 bg-accent" />
                    {s.now}
                </span>
                {hasBefore && (
                    <span className="flex items-center gap-1.5">
                        <span className="w-3.5 border-t-[1.5px] border-dashed border-ink-3" />
                        {s.before}
                    </span>
                )}
            </div>
            <figcaption id={summaryId} className="text-small text-ink-3">
                {hasBefore ? s.summary(median(survival.now.median), median(survival.before.median)) : ''}{' '}
                {s.early(share(survival.now.early), share(survival.before.early))}
                {survival.now.earlyCrashes !== null && ` ${s.earlyCrashes(share(survival.now.earlyCrashes))}`}
                {cut && ` ${s.kept(formatDay(locale, survival.keptFrom, zone, { weekday: false }))}`}
            </figcaption>
        </figure>
    );
}
