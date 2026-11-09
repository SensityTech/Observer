import { Check, CircleX, FolderSearch, TriangleAlert } from 'lucide-react';

import type { FolderReport } from '../api/generated/FolderReport';
import { Button } from '../components/Button';
import { useI18n } from '../i18n/I18nProvider';

/**
 * What the folder check found: what FXServer will run from the folder, or why it cannot and a
 * folder nearby that can. `problemShown`: something beside it says why already.
 */
export function FolderVerdict({
    report,
    problemShown = false,
    suggestionLabel,
    onSuggestion,
}: {
    report: FolderReport;
    problemShown?: boolean;
    suggestionLabel: string;
    onSuggestion: (path: string) => void;
}) {
    const { t, format } = useI18n();
    const words = t.serverSetup.folder;
    const resources = report.resources;
    if (report.problem !== null || resources === null) {
        const problem = report.problem ?? 'noResources';
        const suggestion = report.suggestion;
        return (
            <>
                {!problemShown && (
                    <p className="flex items-start gap-2 text-body text-ink-2">
                        <CircleX
                            size={16}
                            strokeWidth={1.5}
                            className="mt-0.5 flex-none text-ink-3"
                            aria-hidden="true"
                        />
                        {problem === 'invalidPath' && report.pathProblem !== null
                            ? t.server.problems[report.pathProblem]
                            : words.problems[problem]}
                    </p>
                )}
                {suggestion !== null && (
                    <div className="flex flex-col items-start gap-2">
                        <p className="text-small text-ink-2">
                            {words.suggestion}{' '}
                            <span className="font-mono text-ink [overflow-wrap:anywhere]">{suggestion}</span>
                        </p>
                        <Button
                            size="sm"
                            icon={FolderSearch}
                            className="max-md:w-full"
                            onClick={() => onSuggestion(suggestion)}
                        >
                            {suggestionLabel}
                        </Button>
                    </div>
                )}
            </>
        );
    }
    const detected = report.detectedCfg;
    const skipped = resources.withoutManifest;
    return (
        <>
            <p className="flex items-center gap-2.5 text-card font-semibold text-ink">
                <span className="grid size-6.5 flex-none place-items-center rounded-full bg-success-soft text-success">
                    <Check size={14} strokeWidth={2.5} aria-hidden="true" />
                </span>
                {words.accepted}
            </p>
            <div className="@container">
                <dl className="grid grid-cols-2 gap-3 @xl:grid-cols-3">
                    <Figure
                        label={words.browser.resources}
                        value={`${format.number(resources.resources)}${resources.truncated ? '+' : ''}`}
                        detail={words.browser.inCategories(resources.categories)}
                    />
                    <Figure
                        label={words.browser.cfgCount}
                        value={format.number(report.cfgFiles.length)}
                        detail={detected === null ? words.noCfgFile : words.browser.cfgFirst(detected)}
                    />
                    {skipped.length > 0 && (
                        <Figure
                            label={words.withoutManifestCount}
                            value={format.number(skipped.length)}
                            detail={words.skipped}
                        />
                    )}
                </dl>
            </div>
            {resources.truncated && <p className="text-small text-ink-3">{words.countStopped}</p>}
            {skipped.length > 0 && (
                <p className="flex items-start gap-2 text-small text-warning">
                    <TriangleAlert
                        size={14}
                        strokeWidth={1.5}
                        className="mt-0.5 flex-none"
                        aria-hidden="true"
                    />
                    <span className="[overflow-wrap:anywhere]">{words.withoutManifest(skipped)}</span>
                </p>
            )}
            {report.cfgFiles.length > 0 && (
                <ul className="flex flex-wrap gap-1.5" aria-label={words.cfgFiles}>
                    {report.cfgFiles.map((file) => (
                        <li
                            key={file}
                            className="rounded-xs border border-line bg-surface-2 px-1.5 py-0.5 font-mono text-small text-ink-2 [overflow-wrap:anywhere]"
                        >
                            {file}
                        </li>
                    ))}
                </ul>
            )}
        </>
    );
}

function Figure({ label, value, detail }: { label: string; value: string; detail: string }) {
    return (
        <div className="flex flex-col gap-1 rounded-box border border-line p-3">
            <dt className="eyebrow">{label}</dt>
            <dd className="text-figure-sm text-ink tabular-nums">{value}</dd>
            <dd className="text-small text-ink-3 [overflow-wrap:anywhere]">{detail}</dd>
        </div>
    );
}
