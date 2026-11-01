import { OctagonX, TimerOff, Unplug, UserX, type LucideIcon } from 'lucide-react';

import type { DropsSummary } from '../api/generated/DropsSummary';
import type { DropTile } from '../api/generated/DropTile';
import type { TileKind } from '../api/generated/TileKind';
import { Sparkline } from '../components/Sparkline';
import { useI18n } from '../i18n/I18nProvider';
import { Tile, type Fact } from '../overview/Tiles';
import { formatRate, formatShare, keptSince, trendWindows, VerdictChip } from './words';

const TILE_ICONS = {
    crashes: OctagonX,
    timeouts: TimerOff,
    kicks: UserX,
    unexpected: Unplug,
} satisfies Record<TileKind, LucideIcon>;

const ORDER: TileKind[] = ['crashes', 'timeouts', 'kicks', 'unexpected'];

/** The four tiles: crashes, timeouts and kicks per 100 player-hours, and every unexpected end. */
export function DropTiles({ summary }: { summary: DropsSummary }) {
    const { t, locale } = useI18n();
    const windows = trendWindows(t, locale, summary);
    const since = keptSince(locale, summary.keptFrom, summary.from, summary.zone);
    return (
        <div className="grid grid-cols-1 gap-3 min-[360px]:grid-cols-2 lg:grid-cols-4">
            {ORDER.map((kind) => {
                const tile = summary.tiles.find((each) => each.kind === kind);
                return tile === undefined ? null : (
                    <DropRateTile
                        key={kind}
                        tile={tile}
                        windows={windows}
                        range={t.drops.over[summary.range]}
                        since={since}
                    />
                );
            })}
        </div>
    );
}

/** A rate of drops with its verdict, how many sessions ended that way, and its trend. */
function DropRateTile({
    tile,
    windows,
    range,
    since,
}: {
    tile: DropTile;
    /** Which windows the verdict compared. */
    windows: string;
    /** The range the sparkline covers, in words. */
    range: string;
    /** The day players are counted from, when the sessions kept start after the range. */
    since: string | null;
}) {
    const { t, locale, format } = useI18n();
    const words = t.drops.tiles;
    const rising = tile.trend.verdict === 'rising';
    const facts: Fact[] = [
        { term: words.ended, detail: tile.oneIn === null ? t.drops.unknown : words.oneIn(tile.oneIn) },
        { term: words.drops, detail: format.number(tile.drops) },
    ];
    if (tile.onePlayerShare !== null) {
        facts.push({
            term: words.onePlayer,
            detail: words.onePlayerShare(formatShare(locale, tile.onePlayerShare), words.what[tile.kind]),
        });
        if (since !== null) {
            facts.push({ term: t.drops.playersCounted, detail: t.drops.since(since) });
        }
    }
    return (
        <Tile
            icon={TILE_ICONS[tile.kind]}
            label={words.labels[tile.kind]}
            aside={<VerdictChip trend={tile.trend} windows={windows} className="ml-auto" />}
            value={tile.rate === null ? t.drops.unknown : formatRate(locale, tile.rate)}
            unit={tile.rate === null ? undefined : t.drops.unit}
            warning={rising}
            facts={facts}
            trend={
                <Sparkline
                    className="self-end"
                    values={tile.spark}
                    label={words.trend(words.labels[tile.kind], range)}
                    warning={rising}
                />
            }
        />
    );
}
