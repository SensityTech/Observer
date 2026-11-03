import type { PlayerHours } from '../api/generated/PlayerHours';
import type { ScheduleSettings } from '../api/generated/ScheduleSettings';

/** A move is worth suggesting when it cuts off at least this share fewer players, and this many. */
const LEAST_SHARE = 0.2;
const LEAST_PLAYERS = 2;

export type Advice =
    | { kind: 'none' }
    /** No fixed time to compare: the quietest hour alone. */
    | { kind: 'quiet'; quiet: number; players: number }
    | { kind: 'interval'; quiet: number; players: number }
    | { kind: 'already'; time: string }
    | { kind: 'close'; time: string; players: number }
    | { kind: 'move'; time: string; players: number; quiet: number; after: number };

/** The hour of a schedule's `HH:MM`. */
const hourOf = (time: string) => Number(time.slice(0, 2));

/**
 * What the heat map says about the schedule: move its busiest fixed time to the quietest hour when
 * that cuts off clearly fewer players, never when the quietest hour is already planned.
 */
export const restartAdvice = (hours: PlayerHours, schedule: ScheduleSettings | null): Advice => {
    const quiet = hours.quietest;
    const at = (hour: number) => hours.daily[hour] ?? null;
    if (quiet === null) {
        return { kind: 'none' };
    }
    const quietPlayers = at(quiet) ?? 0;
    if (
        schedule === null ||
        !schedule.enabled ||
        (schedule.mode === 'fixedTimes' && schedule.times.length === 0)
    ) {
        return { kind: 'quiet', quiet, players: quietPlayers };
    }
    if (schedule.mode !== 'fixedTimes') {
        return { kind: 'interval', quiet, players: quietPlayers };
    }
    const planned = schedule.times.filter((time) => at(hourOf(time)) !== null);
    if (schedule.times.some((time) => hourOf(time) === quiet)) {
        return { kind: 'already', time: schedule.times.find((time) => hourOf(time) === quiet) ?? '' };
    }
    const busiest = planned.reduce<string | null>(
        (best, time) => (best === null || (at(hourOf(time)) ?? 0) > (at(hourOf(best)) ?? 0) ? time : best),
        null,
    );
    if (busiest === null) {
        return { kind: 'quiet', quiet, players: quietPlayers };
    }
    const players = at(hourOf(busiest)) ?? 0;
    if (quietPlayers <= players * (1 - LEAST_SHARE) && players - quietPlayers >= LEAST_PLAYERS) {
        return { kind: 'move', time: busiest, players, quiet, after: quietPlayers };
    }
    return { kind: 'close', time: busiest, players };
};

/** An hour of the day as the schedule writes it: `06:00`. */
export const hourLabel = (hour: number) => `${String(hour).padStart(2, '0')}:00`;
