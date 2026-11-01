import { Settings2 } from 'lucide-react';
import { useState, type ReactNode } from 'react';

import type { Attribution } from '../api/generated/Attribution';
import type { Comparison } from '../api/generated/Comparison';
import type { NumberChange } from '../api/generated/NumberChange';
import { useMe } from '../auth/me';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { Chip } from '../components/Chip';
import { useI18n } from '../i18n/I18nProvider';
import { DiffView } from './DiffView';
import { escapeInvisible } from './invisible';
import { FILE_CHIP } from './words';

const LAUNCH_ACTIONS = new Set<string>(['launch_settings_changed']);

/**
 * What the server ran with besides its resources, before and after, with who changed it. The cfg
 * chain's files are the owner's: anyone else reads how many changed.
 */
export function EnvironmentCard({ comparison }: { comparison: Comparison }) {
    const { t, format } = useI18n();
    const me = useMe();
    const words = t.changes.environment;
    const { environment, from, to } = comparison;
    const [open, setOpen] = useState<string | null>(null);
    const unknown = t.status.unknown;

    const value = (setting: unknown) => {
        if (setting === null || setting === undefined) {
            return words.notSet;
        }
        if (typeof setting === 'boolean') {
            return setting ? words.yes : words.no;
        }
        if (typeof setting === 'string' && Object.hasOwn(t.server.onesyncModes, setting)) {
            return t.server.onesyncModes[setting as keyof typeof t.server.onesyncModes];
        }
        return typeof setting === 'string' || typeof setting === 'number'
            ? String(setting)
            : JSON.stringify(setting);
    };
    const numbers = (change: NumberChange) =>
        t.changes.fromTo(
            change.from === null ? unknown : String(change.from),
            change.to === null ? unknown : String(change.to),
        );
    const attributed = (entries: Attribution[]) =>
        entries.length === 0 ? null : (
            <ul className="mt-1 flex flex-col gap-0.5 text-meta text-ink-3">
                {entries.map((entry) => (
                    <li key={`${entry.action} ${entry.at}`}>
                        {words.attribution(
                            t.audit.actions[entry.action],
                            entry.by ?? words.observer,
                            format.timestamp(entry.at, { seconds: false }),
                        )}
                    </li>
                ))}
            </ul>
        );
    const launchBy = environment.attributions.filter((entry) => LAUNCH_ACTIONS.has(entry.action));
    const buildBy = environment.attributions.filter((entry) => !LAUNCH_ACTIONS.has(entry.action));
    const cfg = environment.cfg;

    const rows: { key: string; label: string; body: ReactNode }[] = [];
    if (environment.build !== null) {
        rows.push({
            key: 'build',
            label: words.build,
            body: (
                <>
                    <span className="font-mono text-small text-ink">{numbers(environment.build)}</span>
                    {attributed(buildBy)}
                </>
            ),
        });
    }
    if (environment.gameBuild !== null) {
        rows.push({
            key: 'gameBuild',
            label: words.gameBuild,
            body: <span className="font-mono text-small text-ink">{numbers(environment.gameBuild)}</span>,
        });
    }
    if (environment.game !== null) {
        const games = t.serverSetup.cfg.games;
        rows.push({
            key: 'game',
            label: words.game,
            body: t.changes.fromTo(
                environment.game.from === null ? unknown : games[environment.game.from],
                environment.game.to === null ? unknown : games[environment.game.to],
            ),
        });
    }
    if (environment.settings.length > 0) {
        rows.push({
            key: 'settings',
            label: words.settings,
            body: (
                <>
                    <ul className="flex flex-col gap-0.5">
                        {environment.settings.map((setting) => (
                            <li key={setting.setting}>
                                {words.settingNames[setting.setting] ?? setting.setting}:{' '}
                                <span className="text-ink">
                                    {t.changes.fromTo(value(setting.from), value(setting.to))}
                                </span>
                            </li>
                        ))}
                    </ul>
                    {attributed(launchBy)}
                </>
            ),
        });
    }
    if (environment.arguments.length > 0) {
        rows.push({
            key: 'arguments',
            label: words.arguments,
            body: (
                <>
                    <ul className="flex flex-col gap-0.5">
                        {environment.arguments.map((argument) => (
                            <li key={argument.name}>
                                <code className="code-inline">{argument.name}</code>{' '}
                                {words.argumentChanges[argument.change]}
                            </li>
                        ))}
                    </ul>
                    {environment.settings.length === 0 && attributed(launchBy)}
                </>
            ),
        });
    }
    if (cfg.changed > 0) {
        const lines = t.changes.lines(format.number(cfg.lines.added), format.number(cfg.lines.removed));
        rows.push({
            key: 'cfg',
            label: words.cfg,
            body:
                me.isOwner && cfg.files.length > 0 ? (
                    <ul className="flex flex-col gap-2">
                        {cfg.files.map((file) => (
                            <li key={file.path} className="flex flex-col gap-2">
                                <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                                    <span className="font-mono text-small text-ink [overflow-wrap:anywhere]">
                                        {escapeInvisible(file.path)}
                                    </span>
                                    <Chip icon={FILE_CHIP[file.change]}>
                                        {t.changes.diff.fileChanges[file.change]}
                                    </Chip>
                                    {file.lines !== null && (
                                        <span className="font-mono text-small text-ink-2">
                                            {t.changes.lines(
                                                format.number(file.lines.added),
                                                format.number(file.lines.removed),
                                            )}
                                        </span>
                                    )}
                                    <Button
                                        size="sm"
                                        tone="ghost"
                                        aria-expanded={open === file.path}
                                        onClick={() => setOpen(open === file.path ? null : file.path)}
                                    >
                                        {open === file.path ? words.hideDiff : words.showDiff}
                                    </Button>
                                </div>
                                {open === file.path && (
                                    <DiffView from={from.id} to={to.id} path={file.path} file={null} />
                                )}
                            </li>
                        ))}
                    </ul>
                ) : (
                    `${words.cfgCount(cfg.changed)} · ${lines}`
                ),
        });
    }
    for (const [key, label, names] of [
        ['started', words.cfgStarted, cfg.started],
        ['stopped', words.cfgStopped, cfg.stopped],
    ] as const) {
        if (names.length > 0) {
            rows.push({
                key,
                label,
                body: (
                    <span className="flex flex-wrap gap-1.5">
                        {names.map((name) => (
                            <code key={name} className="code-inline">
                                {escapeInvisible(name)}
                            </code>
                        ))}
                    </span>
                ),
            });
        }
    }

    return (
        <Card id="changes-environment" icon={Settings2} title={words.title} meta={words.meta}>
            {rows.length === 0 ? (
                <p className="text-ui text-ink-3">{words.same}</p>
            ) : (
                <dl className="grid grid-cols-1 gap-x-6 gap-y-4 text-ui text-ink-2 md:grid-cols-[minmax(0,12rem)_minmax(0,1fr)]">
                    {rows.map((row) => (
                        <div key={row.key} className="contents">
                            <dt className="text-ink-3 max-md:-mb-3">{row.label}</dt>
                            <dd className="min-w-0">{row.body}</dd>
                        </div>
                    ))}
                </dl>
            )}
        </Card>
    );
}
