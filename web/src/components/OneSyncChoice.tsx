import { useId } from 'react';

import type { OneSync } from '../api/generated/OneSync';
import { useI18n } from '../i18n/I18nProvider';

const ONESYNC_MODES: OneSync[] = ['off', 'on', 'legacy'];

/** The OneSync FXServer starts with, a segmented choice of its three modes. */
export function OneSyncChoice({ value, onChange }: { value: OneSync; onChange: (mode: OneSync) => void }) {
    const { t } = useI18n();
    const hint = useId();
    return (
        <fieldset className="flex min-w-0 flex-col gap-1.5" aria-describedby={hint}>
            <legend className="mb-1.5 text-ui font-medium text-ink">{t.server.onesync}</legend>
            <div className="inline-flex max-w-full gap-0.5 self-start rounded-control border border-line bg-surface-2 p-0.75">
                {ONESYNC_MODES.map((mode) => (
                    <label
                        key={mode}
                        className="inline-flex h-6.5 cursor-pointer items-center justify-center rounded-[7px] px-2.5 text-small font-medium text-ink-3 transition-colors hover:text-ink has-checked:bg-raise has-checked:text-ink has-checked:shadow-[var(--elev-sm),0_0_0_1px_var(--line)] has-focus-visible:outline-2 has-focus-visible:outline-offset-1 has-focus-visible:outline-accent max-md:h-11 max-md:min-w-16"
                    >
                        <input
                            type="radio"
                            name="onesync"
                            value={mode}
                            className="sr-only"
                            checked={value === mode}
                            onChange={() => onChange(mode)}
                        />
                        {t.server.onesyncModes[mode]}
                    </label>
                ))}
            </div>
            <p id={hint} className="text-small text-ink-3">
                {t.server.onesyncHint}
            </p>
        </fieldset>
    );
}
