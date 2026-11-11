import type { EntityResult } from '../api/generated/EntityResult';
import type { Messages } from '../i18n/en';
import { outcomes } from './model';

/**
 * What a deletion did, in one sentence or two: how many entities it deleted, then what became of
 * the others. A warning as soon as one stayed.
 */
export const deletionNotice = (t: Messages, results: readonly EntityResult[]) => {
    const words = t.entities;
    const summary = outcomes(results);
    const left = summary.others.map(([outcome, value]) => words.notDeleted[outcome](value)).join(', ');
    const deleted = words.deleted(summary.deleted, summary.asked);
    return {
        tone: summary.deleted === summary.asked ? ('success' as const) : ('warning' as const),
        message: left === '' ? deleted : `${deleted} ${left.charAt(0).toUpperCase()}${left.slice(1)}.`,
    };
};
