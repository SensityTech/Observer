import { copiesOf } from '../api/backups';
import type { BackupsView } from '../api/generated/BackupsView';
import { formatClock, formatDay } from '../i18n/format';
import { useI18n } from '../i18n/I18nProvider';

/** A time in the server's zone: the clock alone today, with its day otherwise. */
export const useServerTime = (zone: string) => {
    const { locale } = useI18n();
    return (instant: string) => {
        const day = formatDay(locale, instant, zone, { weekday: false });
        const today = formatDay(locale, Date.now(), zone, { weekday: false });
        const clock = formatClock(locale, instant, zone);
        return day === today ? clock : `${day} ${clock}`;
    };
};

/** How the last backup went and when the next one runs, as the Backups page and the settings say it. */
export const useBackupsSummary = (view: BackupsView) => {
    const { t, format } = useI18n();
    const words = t.backups.summary;
    const at = useServerTime(view.zone);
    const last = view.recent.find((run) => run.finishedAt !== null);
    const sentence = (() => {
        if (last === undefined) {
            return words.none;
        }
        if (last.outcome === 'succeeded' || last.outcome === 'partial') {
            const copies = copiesOf(last);
            return words.last(
                at(last.startedAt),
                format.bytes(last.fileBytes ?? 0),
                copies.stored,
                copies.total,
            );
        }
        return words.lastFailed(at(last.startedAt));
    })();
    const next = (() => {
        const restart = view.nextRestartAt;
        const timed = view.nextAt;
        if (restart !== null && (timed === null || Date.parse(restart) <= Date.parse(timed))) {
            return words.nextRestart(at(restart));
        }
        return timed === null ? words.nothingPlanned : words.nextTime(at(timed));
    })();
    return { sentence, next };
};
