import { Link } from '@tanstack/react-router';
import { ArrowRight } from 'lucide-react';

import type { LaunchSettings } from '../api/generated/LaunchSettings';
import type { ServerStatus } from '../api/generated/ServerStatus';
import { useI18n } from '../i18n/I18nProvider';
import { anchorOf } from '../settings/catalog';
import { hangAdvice } from './state';

/**
 * Under a hang verdict, the delay that ran out and the way to the setting of it: the owner
 * otherwise goes looking for a broken bridge or resource when a first start only needed longer.
 */
export function HangAdvice({ status, settings }: { status: ServerStatus; settings: LaunchSettings | null }) {
    const { t } = useI18n();
    const advice = hangAdvice(t, status, settings);
    if (advice === null) {
        return null;
    }
    return (
        <div className="flex flex-col items-start gap-2 text-small text-ink-2">
            <p>{advice}</p>
            <Link
                to="/settings/$category"
                params={{ category: 'launch' }}
                hash={anchorOf('hangDetection')}
                className="inline-flex items-center gap-1.5 font-medium text-ink underline-offset-2 hover:underline max-md:min-h-11"
            >
                {t.server_state.openHang}
                <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
            </Link>
        </div>
    );
}
