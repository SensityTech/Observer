import { useQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { PanelRightClose, PanelRightOpen, Search, TriangleAlert, Users, X } from 'lucide-react';
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';

import { Permission } from '../api/generated/Permission';
import type { OnlinePlayer } from '../api/generated/OnlinePlayer';
import { ApiError } from '../api/client';
import { seriesQuery } from '../api/metrics';
import { onlineQuery } from '../api/players';
import { allows, useMe } from '../auth/me';
import { buttonVariants } from '../components/Button';
import { inputClass } from '../components/Field';
import { Notice } from '../components/Notice';
import { Segmented } from '../components/Segmented';
import { Sparkline } from '../components/Sparkline';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { PHONE, useMedia } from '../lib/useMedia';
import { secondsSince, useNow } from '../lib/useNow';
import { useServerStatus } from '../server/context';

/** Where the column's folding is kept, per browser. */
export const COLUMN_KEY = 'observer.playersColumn';

/** Wide enough for the rail, a readable page and the column side by side; narrower, it folds to a strip. */
export const COLUMN_WIDE = '(min-width: 1281px)';

/** From this many milliseconds a ping reads as high: an icon and a word say it, the colour repeats it. */
export const HIGH_PING = 150;

export type Sort = 'name' | 'id' | 'time' | 'ping';
const SORTS = ['name', 'id', 'time', 'ping'] as const satisfies readonly Sort[];

const readFolded = () => {
    try {
        return localStorage.getItem(COLUMN_KEY) === 'folded';
    } catch {
        return false;
    }
};

const saveFolded = (folded: boolean) => {
    try {
        if (folded) {
            localStorage.setItem(COLUMN_KEY, 'folded');
        } else {
            localStorage.removeItem(COLUMN_KEY);
        }
    } catch {
        // Storage blocked: the choice lasts for this page only.
    }
};

const hex = (licence: string) => licence.replace(/^license:/, '');

/**
 * A search matches a name in part, a server ID whole, or the start of a licence, with or without
 * its prefix, as the lists show its first digits.
 */
export const matches = (player: OnlinePlayer, search: string) => {
    const text = search.trim().toLowerCase();
    if (text === '') {
        return true;
    }
    return (
        String(player.netId) === text ||
        (player.name ?? '').toLowerCase().includes(text) ||
        (player.license !== null && hex(player.license).startsWith(hex(text)))
    );
};

/**
 * Names in the locale's order, the players without one after them; IDs up; the longest sessions
 * and the highest pings first, as those are what the list is sorted to find.
 */
export const sortPlayers = (players: readonly OnlinePlayer[], sort: Sort, locale: string) => {
    const collator = new Intl.Collator(locale, { sensitivity: 'base', numeric: true });
    return players.toSorted((a, b) => {
        switch (sort) {
            case 'name':
                if (a.name === null || b.name === null) {
                    return Number(a.name === null) - Number(b.name === null) || a.netId - b.netId;
                }
                return collator.compare(a.name, b.name) || a.netId - b.netId;
            case 'id':
                return a.netId - b.netId;
            case 'time':
                return b.sessionMs - a.sessionMs || a.netId - b.netId;
            case 'ping':
                return (b.ping ?? -1) - (a.ping ?? -1) || a.netId - b.netId;
        }
    });
};

/**
 * How many play now and on how many slots: FXServer's own count while the account sees the
 * server's state, the connected list's length otherwise.
 */
const useCount = () => {
    const { status } = useServerStatus();
    const online = useQuery(onlineQuery);
    const players = status?.players ?? null;
    return {
        online,
        count: players?.online ?? online.data?.players.length ?? null,
        max: players?.max ?? null,
        // Asked again every 10 s, the list brings the column back once the owner turns players on.
        off: online.error instanceof ApiError && online.error.code === 'feature_disabled',
    };
};

/**
 * Who plays now, beside every page for an account that sees players, while the players feature is
 * on. Wide, a column the user may fold to a strip, the choice kept by the browser; narrower, the
 * strip, which opens the column as a drawer; on a phone, the drawer alone, which the mobile bar's
 * button opens.
 */
export function OnlineColumn({
    open,
    onOpen,
    onClose,
}: {
    open: boolean;
    onOpen: () => void;
    onClose: () => void;
}) {
    const { t, format } = useI18n();
    const wide = useMedia(COLUMN_WIDE);
    const phone = useMedia(PHONE);
    const [folded, setFolded] = useState(readFolded);
    const { count, max, off } = useCount();
    const docked = wide && !folded;
    const close = useRef<HTMLButtonElement>(null);
    const opener = useRef<HTMLElement | null>(null);

    const fold = (next: boolean) => {
        setFolded(next);
        saveFolded(next);
    };

    // The drawer has no place beside a docked column.
    useEffect(() => {
        if (docked && open) {
            onClose();
        }
    }, [docked, open, onClose]);

    useEffect(() => {
        if (!open) {
            return;
        }
        opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
        close.current?.focus();
        const closeOnEscape = (event: KeyboardEvent) => {
            if (event.key === 'Escape') {
                onClose();
            }
        };
        window.addEventListener('keydown', closeOnEscape);
        return () => {
            window.removeEventListener('keydown', closeOnEscape);
            opener.current?.focus();
        };
    }, [open, onClose]);

    if (off) {
        return null;
    }
    const shown =
        count === null
            ? t.status.unknown
            : t.status.playerCount(format.number(count), max === null ? null : format.number(max));

    return (
        <>
            {docked ? (
                <aside className="online-wrap" aria-label={t.online.label}>
                    <div className="online">
                        <OnlinePanel
                            action={
                                <button
                                    type="button"
                                    className={buttonVariants({ tone: 'ghost', size: 'icon-sm' })}
                                    aria-label={t.online.hide}
                                    title={t.online.hide}
                                    onClick={() => fold(true)}
                                >
                                    <PanelRightClose size={16} strokeWidth={1.5} aria-hidden="true" />
                                </button>
                            }
                        />
                    </div>
                </aside>
            ) : (
                !phone && (
                    <div className="online-wrap is-strip">
                        <button
                            type="button"
                            className="online-strip"
                            aria-label={t.online.show(shown)}
                            aria-controls={wide ? undefined : 'online-drawer'}
                            aria-expanded={wide ? undefined : open}
                            onClick={() => (wide ? fold(false) : onOpen())}
                        >
                            <PanelRightOpen size={16} strokeWidth={1.5} aria-hidden="true" />
                            <Users size={16} strokeWidth={1.5} aria-hidden="true" className="mt-2" />
                            <span className="text-figure-sm text-ink tabular-nums" aria-hidden="true">
                                {count === null ? t.status.unknown : format.number(count)}
                            </span>
                            {max !== null && (
                                <span className="text-meta text-ink-3 tabular-nums" aria-hidden="true">
                                    {t.online.of(format.number(max))}
                                </span>
                            )}
                            {count !== null && max !== null && max > 0 && (
                                <span className="online-strip-fill" aria-hidden="true">
                                    <span style={{ height: `${Math.min(100, (count / max) * 100)}%` }} />
                                </span>
                            )}
                        </button>
                    </div>
                )
            )}
            {!docked && (
                <>
                    <aside
                        id="online-drawer"
                        className={open ? 'online-drawer is-open' : 'online-drawer'}
                        aria-label={t.online.label}
                    >
                        <div className="online">
                            {open && (
                                <OnlinePanel
                                    action={
                                        <button
                                            ref={close}
                                            type="button"
                                            className={buttonVariants({ tone: 'ghost', size: 'icon-sm' })}
                                            aria-label={t.online.close}
                                            onClick={onClose}
                                        >
                                            <X size={16} strokeWidth={1.5} aria-hidden="true" />
                                        </button>
                                    }
                                />
                            )}
                        </div>
                    </aside>
                    {open && <div className="scrim" aria-hidden="true" onClick={onClose} />}
                </>
            )}
        </>
    );
}

/** The mobile bar's way to the players: their count, opening the drawer. */
export function OnlineButton({ open, onOpen }: { open: boolean; onOpen: () => void }) {
    const { t, format } = useI18n();
    const { count, max, off } = useCount();
    if (off) {
        return null;
    }
    const shown =
        count === null
            ? t.status.unknown
            : t.status.playerCount(format.number(count), max === null ? null : format.number(max));
    return (
        <button
            type="button"
            className={cn(buttonVariants({ tone: 'ghost', size: 'sm' }), 'ml-auto tabular-nums')}
            aria-label={t.online.show(shown)}
            aria-controls="online-drawer"
            aria-expanded={open}
            onClick={onOpen}
        >
            <Users size={16} strokeWidth={1.5} aria-hidden="true" />
            <span aria-hidden="true">{count === null ? t.status.unknown : format.number(count)}</span>
        </button>
    );
}

function OnlinePanel({ action }: { action: ReactNode }) {
    const { t, locale, format } = useI18n();
    const me = useMe();
    const words = t.online;
    const { online, count, max } = useCount();
    const [search, setSearch] = useState('');
    const [sort, setSort] = useState<Sort>('name');
    const trend = useQuery({
        ...seriesQuery('1h', ['players']),
        enabled: allows(me, Permission.DashboardView),
    });
    const players = useMemo(() => online.data?.players ?? [], [online.data]);
    const visible = useMemo(
        () =>
            sortPlayers(
                players.filter((player) => matches(player, search)),
                sort,
                locale,
            ),
        [players, search, sort, locale],
    );
    const values = trend.data?.series.find((each) => each.series === 'players')?.values ?? [];

    let list;
    if (online.isPending) {
        list = (
            <div className="flex flex-col gap-3 px-4 py-3" aria-busy="true">
                <span className="sr-only">{words.loading}</span>
                <span className="skeleton w-3/4" />
                <span className="skeleton w-1/2" />
                <span className="skeleton w-2/3" />
            </div>
        );
    } else if (online.isError && online.data === undefined) {
        list = (
            <div className="px-4 py-3">
                <Notice tone="danger">{words.failed}</Notice>
            </div>
        );
    } else if (players.length === 0) {
        list = <p className="px-4 py-6 text-center text-ui text-ink-3">{words.empty}</p>;
    } else if (visible.length === 0) {
        list = <p className="px-4 py-6 text-center text-ui text-ink-3">{words.noMatch}</p>;
    } else {
        list = (
            <ul className="flex flex-col px-2 py-1.5">
                {visible.map((player) => (
                    <PlayerRow key={player.netId} player={player} />
                ))}
            </ul>
        );
    }

    return (
        <>
            <div className="flex flex-col gap-2.5 border-b border-line px-4 pt-3.5 pb-3">
                <div className="flex items-center justify-between gap-2">
                    <h2 className="eyebrow">{words.title}</h2>
                    {action}
                </div>
                <div className="flex items-end gap-2.5">
                    <p className="flex items-end gap-2.5">
                        <span className="text-[34px] leading-none font-semibold tracking-title text-ink tabular-nums">
                            {count === null ? t.status.unknown : format.number(count)}
                        </span>
                        {max !== null && (
                            <span className="pb-0.5 text-ui text-ink-3 tabular-nums">
                                {words.slots(format.number(max))}
                            </span>
                        )}
                        <span className="sr-only">
                            {count === null
                                ? ''
                                : words.count(format.number(count), max === null ? null : format.number(max))}
                        </span>
                    </p>
                    {values.length > 0 && (
                        <Sparkline className="ml-auto h-7.5 w-22.5" values={values} label={words.trend} />
                    )}
                </div>
                {count !== null && max !== null && max > 0 && (
                    <div className="h-1.5 overflow-hidden rounded-pill bg-surface-3" aria-hidden="true">
                        <div
                            className="h-full rounded-pill bg-accent"
                            style={{ width: `${Math.min(100, (count / max) * 100)}%` }}
                        />
                    </div>
                )}
                <div className="relative">
                    <Search
                        size={14}
                        strokeWidth={1.5}
                        className="pointer-events-none absolute top-1/2 left-2.75 -translate-y-1/2 text-ink-3"
                        aria-hidden="true"
                    />
                    <input
                        type="search"
                        aria-label={words.search}
                        placeholder={words.searchHint}
                        className={cn(inputClass, 'h-8 pl-8.5 text-ui')}
                        value={search}
                        onChange={(event) => setSearch(event.target.value)}
                    />
                </div>
                <Segmented
                    label={words.sort}
                    choices={SORTS}
                    value={sort}
                    words={words.sorts}
                    onChange={setSort}
                    className="self-start"
                />
            </div>
            <div className="online-list">{list}</div>
            <LastJoin players={players} />
        </>
    );
}

function PlayerRow({ player }: { player: OnlinePlayer }) {
    const { t, format } = useI18n();
    const words = t.online;
    const confirmed = player.confirmation === 'confirmed';
    const name = confirmed
        ? player.name === null || player.name === ''
            ? t.players.noName
            : player.name
        : player.confirmation === 'confirming'
          ? t.players.now.confirming
          : t.players.now.unconfirmed;
    const time = format.playTime(player.sessionMs);
    const high = player.ping !== null && player.ping >= HIGH_PING;
    const ping = player.ping === null ? words.pingUnknown : format.milliseconds(player.ping);
    const cells = (
        <>
            <span className="w-7 shrink-0 text-right font-mono text-caption text-ink-3">{player.netId}</span>
            <span
                className={cn(
                    'min-w-0 flex-1 truncate text-ui',
                    confirmed && player.name ? 'text-ink-2' : 'text-ink-3',
                )}
            >
                {name}
            </span>
            <span className="shrink-0 font-mono text-caption text-ink-3">{time}</span>
            <span
                className={cn(
                    'inline-flex w-15 shrink-0 items-center justify-end gap-1 font-mono text-caption',
                    high ? 'text-warning' : 'text-ink-3',
                )}
                title={high ? words.highPing : undefined}
            >
                {high && <TriangleAlert size={11} strokeWidth={2} aria-hidden="true" />}
                {ping}
                {high && <span className="sr-only">{words.highPing}</span>}
            </span>
        </>
    );
    const label = words.row(name, player.netId, time, high ? `${ping}, ${words.highPing}` : ping);
    const row = 'flex h-9.5 items-center gap-2.5 rounded-sm px-2 max-md:h-11';
    return (
        <li>
            {player.player === null ? (
                <div className={row}>{cells}</div>
            ) : (
                <Link
                    to="/players/$id"
                    params={{ id: String(player.player) }}
                    className={cn(row, 'transition-colors hover:bg-hover')}
                    aria-label={label}
                >
                    {cells}
                </Link>
            )}
        </li>
    );
}

/** The latest arrival among the players connected now, and how long ago. */
function LastJoin({ players }: { players: readonly OnlinePlayer[] }) {
    const { t, format } = useI18n();
    const now = useNow();
    const last = players.reduce<OnlinePlayer | null>(
        (latest, player) =>
            latest === null || Date.parse(player.since) > Date.parse(latest.since) ? player : latest,
        null,
    );
    if (last === null) {
        return null;
    }
    const name = last.confirmation === 'confirmed' && last.name ? last.name : t.online.someone(last.netId);
    return (
        <p className="mt-auto flex items-center gap-2 border-t border-line bg-bg-subtle px-4 py-2.5 text-small text-ink-3">
            <span className="led led-ok" aria-hidden="true" />
            <span className="min-w-0 truncate">
                {t.online.joined(name, format.duration(secondsSince(last.since, now)))}
            </span>
        </p>
    );
}
