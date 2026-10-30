import { CircleCheck, FolderOpen, FolderSearch, TriangleAlert } from 'lucide-react';
import { useId, type Ref } from 'react';

import type { FolderCheck } from '../api/generated/FolderCheck';
import type { FolderReport } from '../api/generated/FolderReport';
import type { LaunchSettings } from '../api/generated/LaunchSettings';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { PathField } from '../components/PathField';
import { useI18n } from '../i18n/I18nProvider';
import { folderPasses, type Checked } from './state';
import { StepActions } from './StepActions';

/** The server folder: typed, checked by Observer, and replaced by a folder nearby that fits. */
export function FolderStep({
    titleRef,
    draft,
    checked,
    checking,
    onPath,
    onCheck,
    onContinue,
}: {
    titleRef: Ref<HTMLHeadingElement>;
    draft: LaunchSettings;
    checked: Checked<FolderCheck, FolderReport> | null;
    checking: boolean;
    onPath: (dataPath: string) => void;
    onCheck: (dataPath: string) => void;
    onContinue: () => void;
}) {
    const { t } = useI18n();
    const id = useId();
    const words = t.serverSetup.folder;
    const report =
        checked !== null && checked.request.dataPath === draft.dataPath.trim() ? checked.report : null;
    const problem = report?.problem ?? null;
    const error =
        problem === null
            ? undefined
            : problem === 'invalidPath' && report?.pathProblem != null
              ? t.server.problems[report.pathProblem]
              : words.problems[problem];
    const passes = folderPasses(checked, draft);
    // An empty field is checked too: the report says what the field needs.
    const check = (dataPath: string) => {
        if (!checking) {
            onCheck(dataPath.trim());
        }
    };

    return (
        <Card id={id} icon={FolderOpen} title={t.serverSetup.stepNames.folder} titleRef={titleRef}>
            <div className="flex flex-col gap-5">
                <form
                    className="flex max-w-2xl flex-col gap-4"
                    noValidate
                    onSubmit={(event) => {
                        event.preventDefault();
                        check(draft.dataPath);
                    }}
                >
                    <p className="text-body text-ink-2">{words.lead}</p>
                    <PathField
                        label={words.label}
                        hint={words.hint}
                        value={draft.dataPath}
                        error={error}
                        onChange={onPath}
                    />
                    {/* The form's submit button, so that Enter in the field checks the folder. */}
                    <Button type="submit" pending={checking} className="self-start max-md:w-full">
                        {words.check}
                    </Button>
                    {report?.suggestion != null && (
                        <div className="flex flex-col items-start gap-2">
                            <p className="text-small text-ink-2">
                                {words.suggestion}{' '}
                                <span className="font-mono text-ink [overflow-wrap:anywhere]">
                                    {report.suggestion}
                                </span>
                            </p>
                            <Button
                                size="sm"
                                icon={FolderSearch}
                                className="max-md:w-full"
                                onClick={() => {
                                    const suggestion = report.suggestion ?? '';
                                    onPath(suggestion);
                                    check(suggestion);
                                }}
                            >
                                {words.useSuggestion}
                            </Button>
                        </div>
                    )}
                </form>
                {report !== null && <Accepted report={report} />}
                <StepActions note={report === null && !checking ? words.notChecked : null}>
                    <Button tone="ink" disabled={!passes || checking} onClick={onContinue}>
                        {t.serverSetup.continue}
                    </Button>
                </StepActions>
            </div>
        </Card>
    );
}

/** What an accepted folder holds, the resources FXServer will skip included. */
function Accepted({ report }: { report: FolderReport }) {
    const { t } = useI18n();
    const words = t.serverSetup.folder;
    const resources = report.resources;
    if (report.problem !== null || resources === null) {
        return null;
    }
    return (
        <output className="flex flex-col gap-3">
            <p className="flex items-center gap-2 text-body font-medium text-success">
                <CircleCheck size={16} strokeWidth={1.5} className="flex-none" aria-hidden="true" />
                {words.accepted}
            </p>
            <p className="text-body text-ink-2">
                {words.resources(resources.resources, resources.categories, resources.truncated)}
            </p>
            {resources.withoutManifest.length > 0 && (
                <p className="flex items-start gap-2 text-small text-warning">
                    <TriangleAlert
                        size={14}
                        strokeWidth={1.5}
                        className="mt-0.5 flex-none"
                        aria-hidden="true"
                    />
                    <span className="[overflow-wrap:anywhere]">
                        {words.withoutManifest(resources.withoutManifest)}
                    </span>
                </p>
            )}
            {report.cfgFiles.length === 0 ? (
                <p className="text-small text-ink-3">{words.noCfgFile}</p>
            ) : (
                <div className="flex flex-col gap-1.5">
                    <p className="text-ui font-medium text-ink">{words.cfgFiles}</p>
                    <ul className="flex flex-wrap gap-1.5">
                        {report.cfgFiles.map((file) => (
                            <li
                                key={file}
                                className="rounded-xs border border-line bg-surface-2 px-1.5 py-0.5 font-mono text-small text-ink-2 [overflow-wrap:anywhere]"
                            >
                                {file}
                            </li>
                        ))}
                    </ul>
                </div>
            )}
        </output>
    );
}
