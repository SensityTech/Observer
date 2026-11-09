import { useQuery } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import { CircleArrowUp } from 'lucide-react';

import type { UpdatesView } from '../api/generated/UpdatesView';
import { isUpdating, updatesQuery } from '../api/updates';
import { Chip } from '../components/Chip';
import { useI18n } from '../i18n/I18nProvider';

/** Read again every ten minutes, and closely while an install runs. */
const watched = {
    ...updatesQuery,
    refetchInterval: (query: { state: { data: UpdatesView | undefined } }) =>
        isUpdating(query.state.data) ? 5000 : 600_000,
};

/**
 * At the foot of the rail, for the owner: a newer release, or the install under way. A link to the
 * About page's card that never reads as the current entry, which the About page is not one of.
 */
export function UpdateSignal({ onNavigate }: { onNavigate: () => void }) {
    const { t } = useI18n();
    const navigate = useNavigate();
    const updates = useQuery(watched);
    const view = updates.data;
    const busy = isUpdating(view);
    const version = busy ? view?.job?.version : view?.available?.version;
    if (version === undefined) {
        return null;
    }
    return (
        <a
            href="/about#updates"
            className="nav-item"
            onClick={(event) => {
                event.preventDefault();
                onNavigate();
                void navigate({ to: '/about', hash: 'updates' });
            }}
        >
            <CircleArrowUp size={16} strokeWidth={1.5} aria-hidden="true" />
            {busy ? t.updates.railBusy : t.updates.rail}
            <Chip tone="success" className="ml-auto font-mono">
                {version}
            </Chip>
        </a>
    );
}
