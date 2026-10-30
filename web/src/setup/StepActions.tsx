import { ArrowLeft } from 'lucide-react';
import type { ReactNode } from 'react';

import { Button } from '../components/Button';
import { useI18n } from '../i18n/I18nProvider';

/**
 * A step's buttons under its content: Back on the left, the step's own on the right, and above
 * them why the step cannot go on yet. Phones stack them across the whole width.
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
        <div className="flex flex-col gap-3">
            {note && <p className="text-small text-ink-3">{note}</p>}
            <div className="flex flex-wrap items-center justify-end gap-2 max-md:flex-col max-md:items-stretch">
                {onBack !== undefined && (
                    <Button tone="ghost" icon={ArrowLeft} className="mr-auto max-md:mr-0" onClick={onBack}>
                        {t.serverSetup.back}
                    </Button>
                )}
                {children}
            </div>
        </div>
    );
}
