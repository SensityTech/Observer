import { OctagonX, Package, Play, RotateCw, type LucideIcon } from 'lucide-react';
import { useLayoutEffect, useRef, useState } from 'react';

import { useI18n } from '../i18n/I18nProvider';
import type { Messages } from '../i18n/en';
import { cn } from '../lib/cn';
import { chartTime, clock, clusters, sides, type ChartMarker, type Cluster, type Side } from './markers';

const MARKER_ICONS = {
    start: Play,
    restart: RotateCw,
    crash: OctagonX,
    build: Package,
} satisfies Record<ChartMarker['kind'], LucideIcon>;

/** Placed first when labels compete for room: a crash, then a build change, then the rest. */
const WEIGHTS = { crash: 2, build: 1, start: 0, restart: 0 } satisfies Record<ChartMarker['kind'], number>;

/** Plots narrower than this, phones, show the markers' icons without their labels. */
const LABELS_FROM = 560;
/** Icons closer than this merge into one. */
const ICON_GAP = 16;
/** Half an icon: how far from the band's edges an icon's middle stays, not to be cut. */
const HALF_ICON = 6;
/** The markers the text alternative names; the others are counted. */
const WORDED = 5;

/** A marker and where its instant falls on the plot, in pixels from the plot's left edge. */
export type PlacedMarker = { marker: ChartMarker; x: number };

const key = (marker: ChartMarker) => `${marker.kind}-${marker.at}`;

/** A marker's word: `Crash`, or the build it changed to, `Build 12400`. */
const markerLabel = (t: Messages, marker: ChartMarker) =>
    marker.kind === 'build' ? t.charts.markers.build(marker.build) : t.charts.markers[marker.kind];

type Layout = { width: number; wide: boolean; sides: Side[]; clusters: Cluster[] };

/** The 24 px band above a plot: each marker's label beside its instant, or its icon on a phone. */
export function MarkerBand({ markers }: { markers: readonly PlacedMarker[] }) {
    const { t } = useI18n();
    const band = useRef<HTMLDivElement>(null);
    const labels = useRef<(HTMLSpanElement | null)[]>([]);
    const [layout, setLayout] = useState<Layout>({ width: 0, wide: true, sides: [], clusters: [] });

    // The labels stay laid out, invisible on a phone, so that their widths are known whichever
    // way the plot is resized.
    useLayoutEffect(() => {
        const width = band.current?.offsetWidth ?? 0;
        const wide = width >= LABELS_FROM;
        const next: Layout = {
            width,
            wide,
            sides: sides(
                markers.map(({ marker, x }, index) => ({
                    x,
                    width: labels.current[index]?.offsetWidth ?? 0,
                    weight: WEIGHTS[marker.kind],
                })),
                width,
            ),
            clusters: clusters(
                markers.map(({ marker, x }) => ({ x, crash: marker.kind === 'crash' })),
                ICON_GAP,
            ),
        };
        setLayout((current) =>
            current.width === next.width &&
            current.wide === next.wide &&
            current.sides.join() === next.sides.join() &&
            JSON.stringify(current.clusters) === JSON.stringify(next.clusters)
                ? current
                : next,
        );
    }, [markers]);

    return (
        <div ref={band} className="relative h-6 overflow-hidden" aria-hidden="true">
            {markers.map(({ marker, x }, index) => {
                const Icon = MARKER_ICONS[marker.kind];
                const right = layout.sides[index] === 'right';
                const time = clock(marker.at);
                const back =
                    marker.kind === 'crash' && marker.backOnlineAt !== null
                        ? t.charts.markers.backOnline(clock(marker.backOnlineAt))
                        : '';
                return (
                    <span
                        key={key(marker)}
                        ref={(element) => {
                            labels.current[index] = element;
                        }}
                        className={cn(
                            'absolute top-0 flex h-5 items-center gap-1 text-caption leading-none whitespace-nowrap text-ink-2',
                            right ? 'pl-1.75' : 'pr-1.75',
                            (!layout.wide || layout.sides[index] === 'hidden') && 'invisible',
                        )}
                        style={{ left: x, transform: right ? undefined : 'translateX(-100%)' }}
                    >
                        {right && <span className="font-mono text-[11px] text-ink-3">{time}</span>}
                        <Icon
                            size={12}
                            strokeWidth={2}
                            className={marker.kind === 'crash' ? 'text-danger' : 'text-ink-3'}
                        />
                        {markerLabel(t, marker)}
                        {!right && <span className="font-mono text-[11px] text-ink-3">{time}</span>}
                        {back}
                    </span>
                );
            })}
            {!layout.wide &&
                layout.clusters.map((cluster) => {
                    const placed = markers[cluster.shown];
                    if (placed === undefined) {
                        return null;
                    }
                    const Icon = MARKER_ICONS[placed.marker.kind];
                    return (
                        <Icon
                            key={key(placed.marker)}
                            size={12}
                            strokeWidth={2}
                            data-cluster={cluster.members.length}
                            className={cn(
                                'absolute top-1 -translate-x-1/2 bg-surface',
                                placed.marker.kind === 'crash' ? 'text-danger' : 'text-ink-3',
                            )}
                            style={{
                                left: Math.min(Math.max(placed.x, HALF_ICON), layout.width - HALF_ICON),
                            }}
                        />
                    );
                })}
        </div>
    );
}

/** The markers' dashed lines through the plot, from the band above it down. */
export function MarkerLines({ markers }: { markers: readonly PlacedMarker[] }) {
    return markers.map(({ marker, x }) => (
        <span
            key={key(marker)}
            className={cn(
                'absolute -top-6 bottom-0 border-l border-dashed',
                marker.kind === 'crash' ? 'border-danger' : 'border-ink-3',
            )}
            style={{ left: x }}
        />
    ));
}

/**
 * The range's markers in words, newest first, for the chart's text alternative: `Crash 14:02, back
 * online 14:04 · Restart 12:31`, with their days over buckets of `step` seconds as long as a week's.
 * `null` without any.
 */
export const markerWords = (t: Messages, markers: readonly ChartMarker[], step: number) => {
    if (markers.length === 0) {
        return null;
    }
    const newest = markers.toSorted((a, b) => b.at - a.at);
    const words = newest
        .slice(0, WORDED)
        .map((marker) =>
            t.charts.markers.inWords(
                markerLabel(t, marker),
                chartTime(marker.at, step),
                marker.kind === 'crash' && marker.backOnlineAt !== null
                    ? chartTime(marker.backOnlineAt, step)
                    : null,
            ),
        );
    return t.charts.markers.listed(words, newest.length - words.length);
};
