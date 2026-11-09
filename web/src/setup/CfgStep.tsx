import { CircleCheck, HardDrive, Info, OctagonX, RefreshCw, Search, TriangleAlert } from 'lucide-react';
import { useState, type ReactNode, type Ref } from 'react';

import type { CfgCheck } from '../api/generated/CfgCheck';
import type { CfgFacts } from '../api/generated/CfgFacts';
import type { CfgFinding } from '../api/generated/CfgFinding';
import type { CfgReport } from '../api/generated/CfgReport';
import type { FolderReport } from '../api/generated/FolderReport';
import type { LaunchSettings } from '../api/generated/LaunchSettings';
import type { Severity } from '../api/generated/Severity';
import { Button } from '../components/Button';
import { Chip } from '../components/Chip';
import { Notice } from '../components/Notice';
import { PathField } from '../components/PathField';
import { Switch } from '../components/Switch';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { CfgPreview, FileTree, foldersOf } from './CfgBrowser';
import { Fact } from './Fact';
import { cfgPasses, groupFindings, type Checked } from './state';
import { StepActions } from './StepActions';
import { StepPage } from './StepPage';

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
    banner,
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
    banner?: ReactNode;
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

    const browser = words.browser;
    const dataPath = draft.dataPath.trim();
    const chosen = draft.cfgFile.trim();
    const [previewed, setPreviewed] = useState(chosen);
    const [focus, setFocus] = useState<number | null>(null);
    const [onlyCfg, setOnlyCfg] = useState(true);
    const [filter, setFilter] = useState('');
    // The tree opens on the folders that hold the cfg files the folder check found.
    const [expanded, setExpanded] = useState(() => foldersOf([...(folder?.cfgFiles ?? []), chosen]));
    const toggle = (path: string) =>
        setExpanded((open) => {
            const next = new Set(open);
            if (!next.delete(path)) {
                next.add(path);
            }
            return next;
        });
    // A file the chosen cfg runs is only shown: FXServer starts from the first file of the chain.
    const runByChosen = (path: string) =>
        path !== chosen &&
        (report?.files.some((file) => file.path === path && file.state === 'read') ?? false);
    const open = (path: string) => {
        setPreviewed(path);
        setFocus(null);
        if (!runByChosen(path)) {
            onCfg(path);
            check(path);
        }
    };
    const runBy = previewed !== '' && runByChosen(previewed);

    return (
        <StepPage
            step="cfg"
            titleRef={titleRef}
            title={t.serverSetup.headings.cfg}
            lead={words.lead}
            banner={banner}
            actions={
                <StepActions onBack={onBack} note={note}>
                    <Button icon={RefreshCw} pending={checking} onClick={() => check(draft.cfgFile)}>
                        {words.checkAgain}
                    </Button>
                    <Button tone="ink" disabled={!passes || checking} onClick={onContinue}>
                        {t.serverSetup.continue}
                    </Button>
                </StepActions>
            }
        >
            <section aria-label={browser.label} className="card flex flex-col overflow-hidden">
                <div className="flex flex-wrap items-center gap-2.5 border-b border-line px-3.5 py-3">
                    <span className="flex min-w-0 items-center gap-1.5 font-mono text-small text-ink-3">
                        <HardDrive size={14} strokeWidth={1.75} className="flex-none" aria-hidden="true" />
                        <span className="truncate" title={dataPath}>
                            {dataPath}
                        </span>
                    </span>
                    <span className="flex-1" />
                    <label className="flex h-8 w-56 items-center gap-2 rounded-sm border border-line-strong bg-bg-subtle px-2.5 max-md:w-full">
                        <Search
                            size={14}
                            strokeWidth={1.75}
                            className="flex-none text-ink-3"
                            aria-hidden="true"
                        />
                        <span className="sr-only">{browser.filter}</span>
                        <input
                            type="search"
                            placeholder={browser.filter}
                            className="min-w-0 flex-1 bg-transparent text-ui text-ink outline-none placeholder:text-ink-3"
                            value={filter}
                            onChange={(event) => setFilter(event.target.value)}
                        />
                    </label>
                    <label className="flex h-8 cursor-pointer items-center gap-2 text-ui text-ink-2">
                        <Switch checked={onlyCfg} onChange={(event) => setOnlyCfg(event.target.checked)} />
                        {browser.onlyCfg}
                    </label>
                </div>
                <div className="flex min-h-130 flex-wrap items-stretch">
                    <nav
                        aria-label={browser.tree}
                        className="max-h-160 min-w-0 flex-[1_1_220px] overflow-auto border-r border-line bg-bg-subtle p-2 max-md:max-h-80 max-md:border-r-0 max-md:border-b lg:max-w-75"
                    >
                        <FileTree
                            dataPath={dataPath}
                            chosen={chosen}
                            previewed={previewed}
                            expanded={expanded}
                            onlyCfg={onlyCfg}
                            filter={filter}
                            onToggle={toggle}
                            onOpen={open}
                        />
                    </nav>
                    <div className="flex min-w-0 flex-[999_1_380px] flex-col">
                        {previewed === '' ? (
                            <p className="m-auto p-10 text-center text-body text-ink-3">{browser.pick}</p>
                        ) : (
                            <CfgPreview
                                dataPath={dataPath}
                                path={previewed}
                                findings={report?.findings ?? []}
                                focus={focus}
                            />
                        )}
                    </div>
                    <aside
                        aria-label={browser.side}
                        className={cn(
                            'flex min-w-0 flex-[1_1_260px] flex-col gap-5 border-l border-line p-4 max-md:border-t max-md:border-l-0 lg:max-w-80',
                            checking && 'opacity-50',
                        )}
                        aria-busy={checking || undefined}
                    >
                        {runBy && (
                            <Notice tone="neutral" live={false}>
                                <span className="flex flex-col items-start gap-2">
                                    {browser.runBy(chosen)}
                                    <Button size="sm" onClick={() => setPreviewed(chosen)}>
                                        {browser.openMain(chosen)}
                                    </Button>
                                </span>
                            </Notice>
                        )}
                        {report === null && checking && (
                            <div className="flex flex-col gap-2.5" aria-busy="true">
                                <span className="sr-only">{words.checking}</span>
                                <span className="skeleton w-1/2" />
                                <span className="skeleton w-2/3" />
                                <span className="skeleton w-1/3" />
                            </div>
                        )}
                        {report?.folder != null && (
                            <Notice tone="danger">
                                {words.folderRefused} {t.serverSetup.folder.problems[report.folder]}
                            </Notice>
                        )}
                        {report !== null && report.folder === null && error !== undefined && (
                            <Notice tone="danger">{error}</Notice>
                        )}
                        {report !== null && report.problem === null && report.folder === null && (
                            <SidePanel
                                report={report}
                                onFile={(file) => {
                                    setPreviewed(file);
                                    setFocus(null);
                                }}
                                onFinding={(finding) => {
                                    setPreviewed(finding.file);
                                    setFocus(finding.line);
                                }}
                            />
                        )}
                    </aside>
                </div>
            </section>
            <details className="group self-start">
                <summary className="cursor-pointer list-none text-small font-medium text-ink-2 underline-offset-2 hover:underline [&::-webkit-details-marker]:hidden">
                    {browser.typeInstead}
                </summary>
                <form
                    className="mt-3 flex max-w-2xl flex-col gap-3"
                    noValidate
                    onSubmit={(event) => {
                        event.preventDefault();
                        open(draft.cfgFile.trim());
                    }}
                >
                    <PathField
                        label={words.label}
                        hint={words.hint}
                        value={draft.cfgFile}
                        error={error}
                        onChange={onCfg}
                    />
                </form>
            </details>
        </StepPage>
    );
}

/** What the chosen cfg runs, sets and gets wrong, beside the tree; each file and finding opens in the preview. */
function SidePanel({
    report,
    onFile,
    onFinding,
}: {
    report: CfgReport;
    onFile: (file: string) => void;
    onFinding: (finding: CfgFinding) => void;
}) {
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
            <section className="flex flex-col gap-2">
                <h2 className="font-mono text-eyebrow tracking-[0.08em] text-ink-3 uppercase">
                    {words.browser.chain}
                </h2>
                <ol className="flex flex-col gap-1">
                    {files.map((file) => (
                        <li key={`${file.state}:${file.path}`}>
                            {file.state === 'read' ? (
                                <button
                                    type="button"
                                    className="flex w-full items-center gap-2 rounded-xs py-0.5 text-left font-mono text-small text-ink-2 hover:text-ink"
                                    onClick={() => onFile(file.path)}
                                >
                                    <CircleCheck
                                        size={13}
                                        strokeWidth={2}
                                        className="flex-none text-success"
                                        aria-hidden="true"
                                    />
                                    <span className="min-w-0 [overflow-wrap:anywhere]">{file.path}</span>
                                </button>
                            ) : (
                                <span className="flex items-center gap-2 py-0.5 font-mono text-small text-ink-3">
                                    <Info
                                        size={13}
                                        strokeWidth={2}
                                        className="flex-none"
                                        aria-hidden="true"
                                    />
                                    <span className="min-w-0 [overflow-wrap:anywhere]">
                                        {file.path} · {words.fileStates[file.state]}
                                    </span>
                                </span>
                            )}
                        </li>
                    ))}
                </ol>
            </section>
            {report.facts !== null && (
                <section className="flex flex-col gap-1">
                    <h2 className="font-mono text-eyebrow tracking-[0.08em] text-ink-3 uppercase">
                        {words.facts}
                    </h2>
                    <dl className="flex flex-col">
                        <FactRows facts={report.facts} compact />
                        <Fact compact label={words.onesync}>
                            {report.facts.onesync === null
                                ? words.notSet
                                : t.server.onesyncModes[report.facts.onesync]}
                        </Fact>
                    </dl>
                </section>
            )}
            <Findings findings={report.findings} onOpen={onFinding} />
        </>
    );
}

/** What the cfg sets, but its OneSync, which the start step's summary repeats. */
export function FactRows({ facts, compact = false }: { facts: CfgFacts; compact?: boolean }) {
    const { t, format } = useI18n();
    const words = t.serverSetup.cfg;
    return (
        <>
            <Fact compact={compact} label={words.game}>
                {facts.game === null ? words.defaultGame : words.games[facts.game]}
            </Fact>
            <Fact compact={compact} label={words.hostname}>
                {facts.hostname ?? words.notSet}
            </Fact>
            <Fact compact={compact} label={words.endpoints}>
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
            <Fact compact={compact} label={words.slots}>
                {facts.maxClients === null ? words.notSet : format.number(facts.maxClients)}
            </Fact>
            <Fact compact={compact} label={words.licence}>
                {words.licences[facts.licence]}
            </Fact>
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
function Findings({ findings, onOpen }: { findings: CfgFinding[]; onOpen: (finding: CfgFinding) => void }) {
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
                    <h3 className="font-mono text-eyebrow tracking-[0.08em] text-ink-3 uppercase">
                        {words.groups[severity](group.length)}
                    </h3>
                    <ul className="flex flex-col gap-2">
                        {keyed(group).map(({ finding, key }) => (
                            <li
                                key={key}
                                className="flex flex-col items-start gap-1.5 rounded-box border border-line bg-bg-subtle p-3"
                            >
                                <Chip tone={SEVERITY[severity].tone} icon={SEVERITY[severity].icon}>
                                    {words.severities[severity]}
                                </Chip>
                                <span className="text-small text-ink">
                                    {words.kinds[finding.kind](finding.params)}
                                </span>
                                <button
                                    type="button"
                                    className="font-mono text-meta text-ink-3 underline-offset-2 [overflow-wrap:anywhere] hover:text-ink-2 hover:underline"
                                    onClick={() => onOpen(finding)}
                                >
                                    {finding.line === null
                                        ? finding.file
                                        : `${finding.file}:${format.number(finding.line, { useGrouping: false })}`}
                                </button>
                            </li>
                        ))}
                    </ul>
                </section>
            ))}
        </div>
    );
}
