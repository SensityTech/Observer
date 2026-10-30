import { CircleCheck, FileCog, Info, OctagonX, RefreshCw, TriangleAlert } from 'lucide-react';
import { useId, type Ref } from 'react';

import type { CfgCheck } from '../api/generated/CfgCheck';
import type { CfgFacts } from '../api/generated/CfgFacts';
import type { CfgFinding } from '../api/generated/CfgFinding';
import type { CfgReport } from '../api/generated/CfgReport';
import type { FolderReport } from '../api/generated/FolderReport';
import type { LaunchSettings } from '../api/generated/LaunchSettings';
import type { Severity } from '../api/generated/Severity';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { Chip } from '../components/Chip';
import { Notice } from '../components/Notice';
import { PathField } from '../components/PathField';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { Fact } from './Fact';
import { cfgPasses, groupFindings, type Checked } from './state';
import { StepActions } from './StepActions';

const SEVERITY = {
    blocking: { tone: 'danger', icon: OctagonX },
    warning: { tone: 'warning', icon: TriangleAlert },
    note: { tone: 'neutral', icon: Info },
} as const satisfies Record<Severity, unknown>;

/**
 * The cfg, chosen among those the folder holds or typed, and what Observer read in it. Entering the
 * step checks it; the owner checks it again after editing a file.
 */
export function CfgStep({
    titleRef,
    draft,
    folder,
    checked,
    checking,
    onCfg,
    onCheck,
    onBack,
    onContinue,
}: {
    titleRef: Ref<HTMLHeadingElement>;
    draft: LaunchSettings;
    /** The accepted folder's report, which lists the cfg files found in it. */
    folder: FolderReport | null;
    checked: Checked<CfgCheck, CfgReport> | null;
    checking: boolean;
    onCfg: (cfgFile: string) => void;
    onCheck: (cfgFile: string) => void;
    onBack: () => void;
    onContinue: () => void;
}) {
    const { t } = useI18n();
    const id = useId();
    const words = t.serverSetup.cfg;
    const current =
        checked !== null &&
        checked.request.dataPath === draft.dataPath.trim() &&
        checked.request.cfgFile === draft.cfgFile.trim();
    const report = current ? checked.report : null;
    const error =
        report?.problem == null
            ? undefined
            : report.problem === 'invalidPath' && report.pathProblem !== null
              ? t.server.problems[report.pathProblem]
              : words.problems[report.problem];
    const passes = cfgPasses(checked, draft);
    // An empty field is checked too: the report says what the field needs.
    const check = (cfgFile: string) => {
        if (!checking) {
            onCheck(cfgFile.trim());
        }
    };
    const note = checking
        ? null
        : report === null
          ? words.notChecked
          : report.problem === null && report.folder === null && !passes
            ? words.blocked
            : null;

    return (
        <Card id={id} icon={FileCog} title={t.serverSetup.stepNames.cfg} titleRef={titleRef}>
            <div className="flex flex-col gap-5">
                <form
                    className="flex max-w-2xl flex-col gap-4"
                    noValidate
                    onSubmit={(event) => {
                        event.preventDefault();
                        check(draft.cfgFile);
                    }}
                >
                    <p className="text-body text-ink-2">{words.lead}</p>
                    {folder !== null && folder.cfgFiles.length > 0 && (
                        <fieldset className="flex min-w-0 flex-col gap-0.5">
                            <legend className="mb-1.5 text-ui font-medium text-ink">{words.found}</legend>
                            {folder.cfgFiles.map((file) => (
                                <label
                                    key={file}
                                    className="-mx-2 flex cursor-pointer items-center gap-2.5 rounded-sm px-2 py-1.5 hover:bg-hover max-md:min-h-11"
                                >
                                    <input
                                        type="radio"
                                        name="cfg"
                                        value={file}
                                        className="flex-none accent-[var(--btn-bg)]"
                                        checked={draft.cfgFile.trim() === file}
                                        onChange={() => {
                                            onCfg(file);
                                            check(file);
                                        }}
                                    />
                                    <span className="min-w-0 font-mono text-ui text-ink [overflow-wrap:anywhere]">
                                        {file}
                                    </span>
                                    {file === folder.detectedCfg && <Chip>{words.detected}</Chip>}
                                </label>
                            ))}
                        </fieldset>
                    )}
                    <PathField
                        label={words.label}
                        hint={words.hint}
                        value={draft.cfgFile}
                        error={error}
                        onChange={onCfg}
                    />
                </form>
                {report !== null && (
                    <div
                        className={cn('flex flex-col gap-5', checking && 'opacity-50')}
                        aria-busy={checking || undefined}
                    >
                        {report.folder !== null && (
                            <Notice tone="danger">
                                {words.folderRefused} {t.serverSetup.folder.problems[report.folder]}
                            </Notice>
                        )}
                        {report.problem === null && report.folder === null && <Report report={report} />}
                    </div>
                )}
                {report === null && checking && (
                    <div className="flex flex-col gap-3" aria-busy="true">
                        <span className="sr-only">{words.checking}</span>
                        <span className="skeleton w-1/2" />
                        <span className="skeleton w-2/3" />
                        <span className="skeleton w-1/3" />
                    </div>
                )}
                <StepActions onBack={onBack} note={note}>
                    <Button icon={RefreshCw} pending={checking} onClick={() => check(draft.cfgFile)}>
                        {words.checkAgain}
                    </Button>
                    <Button tone="ink" disabled={!passes || checking} onClick={onContinue}>
                        {t.serverSetup.continue}
                    </Button>
                </StepActions>
            </div>
        </Card>
    );
}

function Report({ report }: { report: CfgReport }) {
    const { t } = useI18n();
    const words = t.serverSetup.cfg;
    // A file exec'd again each time is said once: run again.
    const files = report.files.filter(
        (file, index) =>
            report.files.findIndex((other) => other.path === file.path && other.state === file.state) ===
            index,
    );
    return (
        <>
            <div className="grid gap-5 lg:grid-cols-2">
                {report.facts !== null && (
                    <section className="flex min-w-0 flex-col gap-1.5">
                        <h3 className="text-ui font-medium text-ink">{words.facts}</h3>
                        <dl className="flex flex-col">
                            <FactRows facts={report.facts} />
                            <Fact label={words.onesync}>
                                {report.facts.onesync === null
                                    ? words.notSet
                                    : t.server.onesyncModes[report.facts.onesync]}
                            </Fact>
                        </dl>
                    </section>
                )}
                <section className="flex min-w-0 flex-col gap-1.5">
                    <h3 className="text-ui font-medium text-ink">{words.files}</h3>
                    <ul className="flex flex-col">
                        {files.map((file) => (
                            <li
                                key={`${file.state}:${file.path}`}
                                className="flex flex-wrap items-baseline justify-between gap-x-4 border-b border-line py-1.5 last:border-b-0"
                            >
                                <span className="min-w-0 font-mono text-small text-ink [overflow-wrap:anywhere]">
                                    {file.path}
                                </span>
                                <span className="text-small text-ink-3">{words.fileStates[file.state]}</span>
                            </li>
                        ))}
                    </ul>
                </section>
            </div>
            <Findings findings={report.findings} />
        </>
    );
}

/** What the cfg sets, but its OneSync, which the start step's summary repeats. */
export function FactRows({ facts }: { facts: CfgFacts }) {
    const { t, format } = useI18n();
    const words = t.serverSetup.cfg;
    return (
        <>
            <Fact label={words.game}>
                {facts.game === null ? words.defaultGame : words.games[facts.game]}
            </Fact>
            <Fact label={words.hostname}>{facts.hostname ?? words.notSet}</Fact>
            <Fact label={words.endpoints}>
                {facts.endpoints.length === 0 ? (
                    words.none
                ) : (
                    <span className="flex flex-col font-mono text-ui">
                        {facts.endpoints.map((endpoint) => (
                            <span key={`${endpoint.protocol} ${endpoint.address}`}>
                                {words.protocols[endpoint.protocol]} {endpoint.address}
                            </span>
                        ))}
                    </span>
                )}
            </Fact>
            <Fact label={words.slots}>
                {facts.maxClients === null ? words.notSet : format.number(facts.maxClients)}
            </Fact>
            <Fact label={words.licence}>{words.licences[facts.licence]}</Fact>
        </>
    );
}

/** Each finding with a key of its own: two commands on one line may give the same finding twice. */
const keyed = (findings: CfgFinding[]) => {
    const seen = new Map<string, number>();
    return findings.map((finding) => {
        const text = JSON.stringify(finding);
        const nth = (seen.get(text) ?? 0) + 1;
        seen.set(text, nth);
        return { finding, key: `${text}#${nth}` };
    });
};

/** The findings grouped blocking, warning, note: each its severity, its sentence and where it is written. */
function Findings({ findings }: { findings: CfgFinding[] }) {
    const { t, format } = useI18n();
    const words = t.serverSetup.cfg;
    const groups = groupFindings(findings);
    if (groups.length === 0) {
        return (
            <p className="flex items-center gap-2 text-body text-success">
                <CircleCheck size={16} strokeWidth={1.5} className="flex-none" aria-hidden="true" />
                {words.clean}
            </p>
        );
    }
    return (
        <div className="flex flex-col gap-4">
            {groups.map(({ severity, findings: group }) => (
                <section key={severity} className="flex flex-col gap-1.5">
                    <h3 className="text-ui font-medium text-ink">{words.groups[severity](group.length)}</h3>
                    <ul className="flex flex-col">
                        {keyed(group).map(({ finding, key }) => (
                            <li
                                key={key}
                                className="flex flex-wrap items-baseline gap-x-3 gap-y-1 border-b border-line py-2 last:border-b-0"
                            >
                                <Chip tone={SEVERITY[severity].tone} icon={SEVERITY[severity].icon}>
                                    {words.severities[severity]}
                                </Chip>
                                <span className="min-w-60 flex-1 text-body text-ink max-md:min-w-0 max-md:basis-full">
                                    {words.kinds[finding.kind](finding.params)}
                                </span>
                                <span className="font-mono text-small text-ink-3 [overflow-wrap:anywhere]">
                                    {finding.line === null
                                        ? finding.file
                                        : `${finding.file}:${format.number(finding.line, { useGrouping: false })}`}
                                </span>
                            </li>
                        ))}
                    </ul>
                </section>
            ))}
        </div>
    );
}
