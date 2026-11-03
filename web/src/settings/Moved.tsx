import { Link } from '@tanstack/react-router';
import { ArrowRight, Settings2 } from 'lucide-react';

import type { Messages } from '../i18n/en';
import { useI18n } from '../i18n/I18nProvider';
import { anchorOf, type CategoryId, type SettingId } from './catalog';

/** Where the settings that sat at the bottom of a page went: their place on the Settings page. */
export function MovedSettings({
    category,
    setting,
    page,
}: {
    category: CategoryId;
    /** The first of the page's settings, when its category holds others too. */
    setting?: SettingId;
    page: keyof Messages['settings']['movedAbout'];
}) {
    const { t } = useI18n();
    return (
        <Link
            to="/settings/$category"
            params={{ category }}
            hash={setting === undefined ? undefined : anchorOf(setting)}
            className="card group flex items-center gap-3 px-5 py-3.5 transition-colors hover:bg-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent max-md:px-4"
        >
            <span className="grid size-6 flex-none place-items-center rounded-[6px] border border-line bg-surface-2 text-ink-2">
                <Settings2 size={14} strokeWidth={1.5} aria-hidden="true" />
            </span>
            <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span className="text-body font-medium text-ink">
                    {t.settings.moved(t.settings.categories[category].name)}
                </span>
                <span className="text-small text-pretty text-ink-3 group-hover:text-ink-2">
                    {t.settings.movedAbout[page]}
                </span>
            </span>
            <ArrowRight size={16} strokeWidth={1.5} className="flex-none text-ink-3" aria-hidden="true" />
        </Link>
    );
}
