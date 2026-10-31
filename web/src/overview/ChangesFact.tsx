import { Link } from '@tanstack/react-router';
import { ArrowRight } from 'lucide-react';

import type { SnapshotSummary } from '../api/generated/SnapshotSummary';
import { useChangesLive } from '../changes/live';
import { changedResources, isBusy, pendingWords } from '../changes/words';
import type { Messages } from '../i18n/en';
import { useI18n } from '../i18n/I18nProvider';

/** What the latest start changed against the one before, in one line. */
const sentence = (t: Messages, latest: SnapshotSummary) => {
    const words = t.changes.overview;
    if (latest.baseline) {
        return isBusy(latest) ? words.baseline : words.baselineTaken;
    }
    if (latest.counts === null) {
        if (latest.state === 'failed') {
            return words.failed;
        }
        return isBusy(latest) ? words.listing : words.notCompared;
    }
    const resources = changedResources(latest.counts);
    const parts = [
        resources > 0 ? words.resources(resources) : null,
        ...latest.counts.environment.map((fact) => t.changes.facts[fact]),
        pendingWords(t, latest, latest.counts),
    ].filter((part) => part !== null);
    return parts.length === 0 ? words.none : words.since(parts.join(' · '));
};

/** The state card's way to the Changes page, from `server.changes`; nothing before a first start. */
export function ChangesFact() {
    const { t } = useI18n();
    const latest = useChangesLive()?.latest ?? null;
    if (latest === null) {
        return null;
    }
    return (
        <Link
            to="/changes"
            className="inline-flex items-center gap-1.5 self-start text-small font-medium text-ink-2 underline-offset-2 hover:text-ink hover:underline"
        >
            {sentence(t, latest)}
            <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
        </Link>
    );
}
