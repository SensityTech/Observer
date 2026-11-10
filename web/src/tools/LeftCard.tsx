import { useId } from 'react';

import type { ArchetypeReport } from '../api/generated/ArchetypeReport';
import type { LeaveReason } from '../api/generated/LeaveReason';
import type { ToolSettings } from '../api/generated/ToolSettings';
import { Switch } from '../components/Switch';
import { useI18n } from '../i18n/I18nProvider';

type Reason = LeaveReason['kind'];

/** The reasons the owner's settings decide, then the one nothing changes, then those settled by hand. */
const BY_HAND: readonly Reason[] = [
    'severalResources',
    'contested',
    'interior',
    'mixedSources',
    'notWritable',
    'excluded',
];

/**
 * Why the report leaves duplicates as found, counted by reason, with the two settings that change
 * that as switches: a duplicate may have several reasons, and counts under each.
 */
export function LeftCard({
    report,
    settings,
    busy,
    onChange,
}: {
    report: ArchetypeReport;
    settings: ToolSettings;
    /** A scan, a fix or a save is under way. */
    busy: boolean;
    onChange: (settings: ToolSettings) => void;
}) {
    const { t } = useI18n();
    const words = t.tools.archetypes.left;
    const id = useId();
    const left = report.groups.filter((group) => group.action.kind === 'leave');
    const count = (kinds: readonly Reason[]) =>
        left.filter(
            (group) =>
                group.action.kind === 'leave' &&
                group.action.reasons.some((reason) => kinds.includes(reason.kind)),
        ).length;
    const escrowed = count(['escrowed']);
    const differ = count(['definitionsDiffer']);
    const manifest = count(['manifestDependent']);
    const byHand = count(BY_HAND);
    // The switches stay while their setting is on: turning one off is how to go back.
    const rows = [
        {
            key: 'escrowed',
            count: escrowed,
            shown: escrowed > 0 || settings.escrowed === 'allow',
            title: words.escrowed.title,
            text: words.escrowed.text,
            control: (
                <label className="flex items-center gap-2.5 text-small text-ink">
                    <Switch
                        checked={settings.escrowed === 'allow'}
                        disabled={busy}
                        onChange={(event) =>
                            onChange({ ...settings, escrowed: event.target.checked ? 'allow' : 'skip' })
                        }
                    />
                    {words.escrowed.control}
                </label>
            ),
        },
        {
            key: 'differ',
            count: differ,
            shown: differ > 0 || settings.differences === 'keepLatest',
            title: words.differ.title,
            text: words.differ.text,
            control: (
                <label className="flex items-center gap-2.5 text-small text-ink">
                    <Switch
                        checked={settings.differences === 'keepLatest'}
                        disabled={busy}
                        onChange={(event) =>
                            onChange({
                                ...settings,
                                differences: event.target.checked ? 'keepLatest' : 'identical',
                            })
                        }
                    />
                    {words.differ.control}
                </label>
            ),
        },
        {
            key: 'manifest',
            count: manifest,
            shown: manifest > 0,
            title: words.manifest.title,
            text: words.manifest.text,
            control: <span className="text-small text-ink-3">{words.nothing}</span>,
        },
        {
            key: 'byHand',
            count: byHand,
            shown: byHand > 0,
            title: words.byHand.title,
            text: words.byHand.text,
            control: <span className="text-small text-ink-3">{words.below}</span>,
        },
    ].filter((row) => row.shown);
    if (rows.length === 0) {
        return null;
    }
    return (
        <section className="card" aria-labelledby={id}>
            <div className="card-head">
                <h2 id={id} className="card-title">
                    {words.title(left.length)}
                </h2>
                <span className="text-small text-ink-3">{words.hint}</span>
            </div>
            <ul className="flex flex-col">
                {rows.map((row) => (
                    <li
                        key={row.key}
                        className="grid grid-cols-[3.5rem_minmax(0,1fr)_auto] items-center gap-x-4 gap-y-2 border-b border-line px-5 py-3.5 last:border-b-0 max-md:grid-cols-[3rem_minmax(0,1fr)] max-md:px-4"
                    >
                        <span className="text-figure-sm text-ink tabular-nums">{row.count}</span>
                        <div className="flex min-w-0 flex-col gap-0.5">
                            <span className="text-ui font-medium text-ink">{row.title}</span>
                            <span className="text-small text-ink-3">{row.text}</span>
                        </div>
                        <div className="max-md:col-start-2">{row.control}</div>
                    </li>
                ))}
            </ul>
        </section>
    );
}
