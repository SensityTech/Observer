import { infiniteQueryOptions, queryOptions } from '@tanstack/react-query';

import { getJson, send } from './client';
import type { Community } from './generated/Community';
import type { OnlinePlayers } from './generated/OnlinePlayers';
import type { PlayerPage } from './generated/PlayerPage';
import type { PlayerProfile } from './generated/PlayerProfile';
import type { PlayerSort } from './generated/PlayerSort';
import type { PlayersImport } from './generated/PlayersImport';
import type { PlayersImportRequest } from './generated/PlayersImportRequest';
import type { PlayersSettings } from './generated/PlayersSettings';

const PATH = '/servers/default/players';

/** Every players query: what an import or a forgotten player changes. */
export const allPlayers = { queryKey: ['players'] };

/** Who is connected, read again every 10 s while someone looks. */
export const onlineQuery = queryOptions({
    queryKey: ['players', 'online'],
    queryFn: ({ signal }) => getJson<OnlinePlayers>(`${PATH}/online`, signal),
    refetchInterval: 10_000,
});

export const communityQuery = queryOptions({
    queryKey: ['players', 'community'],
    queryFn: ({ signal }) => getJson<Community>(`${PATH}/community`, signal),
});

/** The list's query string, without an empty search or the default sort. */
export const playersSearch = (search: string, sort: PlayerSort, after?: string) => {
    const query = new URLSearchParams();
    if (search !== '') {
        query.set('search', search);
    }
    if (sort !== 'playTime') {
        query.set('sort', sort);
    }
    if (after !== undefined) {
        query.set('after', after);
    }
    const text = query.toString();
    return text === '' ? '' : `?${text}`;
};

export const playersQuery = (search: string, sort: PlayerSort) =>
    infiniteQueryOptions({
        queryKey: ['players', 'list', search, sort],
        queryFn: ({ pageParam, signal }) =>
            getJson<PlayerPage>(`${PATH}${playersSearch(search, sort, pageParam)}`, signal),
        initialPageParam: undefined as string | undefined,
        getNextPageParam: (page) => page.next ?? undefined,
    });

/** The first players a name finds, those seen latest first: what a field that picks a player lists. */
export const playersFoundQuery = (search: string) =>
    queryOptions({
        queryKey: ['players', 'found', search],
        queryFn: ({ signal }) => getJson<PlayerPage>(`${PATH}${playersSearch(search, 'lastSeen')}`, signal),
        staleTime: 10_000,
    });

export const profileQuery = (id: number) =>
    queryOptions({
        queryKey: ['players', 'profile', id],
        queryFn: ({ signal }) => getJson<PlayerProfile>(`${PATH}/${id}`, signal),
    });

/** Forgets the player for an erasure request: the owner's alone, with the password again. */
export const forgetPlayer = (id: number) => send('DELETE', `${PATH}/${id}`);

export const playersSettingsQuery = queryOptions({
    queryKey: ['players', 'settings'],
    queryFn: ({ signal }) => getJson<PlayersSettings>(`${PATH}/settings`, signal),
});

export const savePlayersSettings = (settings: PlayersSettings) =>
    send<PlayersSettings>('PUT', `${PATH}/settings`, settings);

/** The txAdmin import under way, read every 2 s while it writes; or the last one, or none. */
export const importQuery = queryOptions({
    queryKey: ['players', 'import'],
    queryFn: ({ signal }) => getJson<PlayersImport | null>(`${PATH}/import`, signal),
    refetchInterval: (query) => (query.state.data?.state === 'writing' ? 2_000 : false),
});

export const startImport = (path: string) =>
    send<PlayersImport>('POST', `${PATH}/import`, { path } satisfies PlayersImportRequest);

/** The longest retention Observer takes, a hundred years: what "kept for good" stands for in the ranges. */
export const MOST_DAYS = 36_500;

/** The shortest each retention may be. */
export const FEWEST_DAYS = {
    linkDays: 30,
    addressDays: 7,
    dayDays: 30,
    profileDays: 365,
} satisfies Record<keyof PlayersSettings, number>;

const within = (value: number, low: number, high: number) =>
    Number.isInteger(value) && value >= low && value <= high;

/**
 * The retentions out of range, as Observer checks them: each one within what it hangs on, so that
 * nothing outlives the profile it belongs to.
 */
export const retentionIssues = (settings: PlayersSettings) => {
    const profile = Math.min(settings.profileDays ?? MOST_DAYS, MOST_DAYS);
    const out: (keyof PlayersSettings | null)[] = [
        settings.profileDays !== null && !within(settings.profileDays, FEWEST_DAYS.profileDays, MOST_DAYS)
            ? 'profileDays'
            : null,
        within(settings.linkDays, FEWEST_DAYS.linkDays, profile) ? null : 'linkDays',
        within(settings.addressDays, FEWEST_DAYS.addressDays, settings.linkDays) ? null : 'addressDays',
        within(settings.dayDays, FEWEST_DAYS.dayDays, profile) ? null : 'dayDays',
    ];
    return out.filter((field) => field !== null);
};

/** A licence's first hex digits, as lists show it; the whole one is copied. */
export const shortLicence = (license: string) => `${license.replace(/^license:/, '').slice(0, 8)}…`;

/** A hardware token is 64 hex digits behind its type: its first ones tell it apart. */
export const shortToken = (token: string) => (token.length > 14 ? `${token.slice(0, 14)}…` : token);

/** Total play time: what Observer saw, txAdmin's, and the session under way. */
export const totalOf = (player: Pick<PlayerProfile, 'playMs' | 'importedMs' | 'inProgressMs'>) =>
    player.playMs + player.importedMs + player.inProgressMs;

/** The day `back` days before `day`, both `YYYY-MM-DD`: calendar arithmetic, no zone involved. */
export const dayBefore = (day: string, back: number) =>
    new Date(Date.parse(`${day}T00:00:00Z`) - back * 86_400_000).toISOString().slice(0, 10);

/** The `count` days up to `today`, oldest first, a day without play at zero. */
export const fillDays = (
    played: readonly { day: string; playMs: number }[],
    today: string,
    count: number,
) => {
    const byDay = new Map(played.map((each) => [each.day, each.playMs]));
    return Array.from({ length: count }, (_, index) => {
        const day = dayBefore(today, count - 1 - index);
        return { day, playMs: byDay.get(day) ?? 0 };
    });
};

/** A point of the community chart: a day, or a week named after its first day. */
export type CommunityPoint = {
    day: string;
    /** Unknown for a week before the last two: unique players do not add up across days. */
    players: number | null;
    newPlayers: number;
    playMs: number;
    /** Holds today, whose figures are so far. */
    today: boolean;
};

export const dailyPoints = (community: Community): CommunityPoint[] =>
    community.days.map((each) => ({ ...each, today: each.day === community.today }));

/**
 * The 30 days as weeks of seven ending today, oldest first; the two days left over are dropped.
 * New players and play time add up; unique players come from the API's last two weeks only.
 */
export const weeklyPoints = (community: Community): CommunityPoint[] => {
    const known = [community.week.players, community.previousWeek.players];
    const weeks: CommunityPoint[] = [];
    for (let end = community.days.length, back = 0; end >= 7; end -= 7, back += 1) {
        const days = community.days.slice(end - 7, end);
        weeks.unshift({
            day: days[0]?.day ?? community.today,
            players: known[back] ?? null,
            newPlayers: days.reduce((sum, each) => sum + each.newPlayers, 0),
            playMs: days.reduce((sum, each) => sum + each.playMs, 0),
            today: back === 0,
        });
    }
    return weeks;
};
