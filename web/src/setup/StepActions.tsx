import { ArrowLeft } from 'lucide-react';
import type { ReactNode } from 'react';

import { Button } from '../components/Button';
import { useI18n } from '../i18n/I18nProvider';

/**
 * A step's buttons in the bar at the foot of the screen: Back on the left, the step's own on the
 * right, and beside them why the step cannot go on yet. On a phone the step's buttons share the
 * width, and the note goes above them.
 */
export function StepActions({
    onBack,
    note,
    children,
}: {
    onBack?: () => void;
    note?: string | null;
    children: ReactNode;
}) {
    const { t } = useI18n();
    return (
        <div className="flex flex-wrap items-center gap-2.5">
            {onBack !== undefined && (
                <Button tone="ghost" icon={ArrowLeft} onClick={onBack}>
                    {t.serverSetup.back}
                </Button>
            )}
            {note && (
                <p className="order-first basis-full text-small text-ink-3 md:order-none md:ml-auto md:basis-auto">
                    {note}
                </p>
            )}
            <div className="ml-auto flex flex-wrap items-center justify-end gap-2.5 max-md:flex-1 max-md:*:flex-1">
                {children}
            </div>
        </div>
    );
}
