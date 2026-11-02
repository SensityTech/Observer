import { useQueryClient } from '@tanstack/react-query';
import { Plus, X } from 'lucide-react';
import { useState } from 'react';

import { ApiError } from '../api/client';
import { saveBackupSettings, testBackupSource } from '../api/backups';
import type { BackupField } from '../api/generated/BackupField';
import type { BackupSettings } from '../api/generated/BackupSettings';
import type { BackupSettingsIssue } from '../api/generated/BackupSettingsIssue';
import type { BackupsView } from '../api/generated/BackupsView';
import type { BeforeRestarts } from '../api/generated/BeforeRestarts';
import type { DatabaseEngine } from '../api/generated/DatabaseEngine';
import type { SourceKind } from '../api/generated/SourceKind';
import type { SourceTestResult } from '../api/generated/SourceTestResult';
import { Button, buttonVariants } from '../components/Button';
import { Dialog } from '../components/Dialog';
import { FieldError, PasswordField, TextField } from '../components/Field';
import { Notice } from '../components/Notice';
import { Segmented } from '../components/Segmented';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { minuteOf, sortTimes } from '../schedule/times';

const BEFORE: BeforeRestarts[] = ['off', 'scheduled', 'allRestarts'];
const ENGINES: DatabaseEngine[] = ['mariaDb', 'mySql'];
const SOURCES: SourceKind[] = ['cfg', 'manual'];
const MAX_WAIT_SECONDS = 600;

/** The issues the API refused the settings with, by field and position. */
const issuesOf = (error: unknown): BackupSettingsIssue[] =>
    error instanceof ApiError &&
    error.code === 'invalid_backup_settings' &&
    Array.isArray(error.params.issues)
        ? (error.params.issues as BackupSettingsIssue[])
        : [];

/** Saves the settings over the version they were read at; the dialog stays open on a refusal. */
const useSave = (view: BackupsView, onClose: () => void) => {
    const { t } = useI18n();
    const queryClient = useQueryClient();
    const [pending, setPending] = useState(false);
    const [failure, setFailure] = useState<string | null>(null);
    const [issues, setIssues] = useState<BackupSettingsIssue[]>([]);
    const save = async (settings: BackupSettings, password?: string) => {
        if (pending) {
            return;
        }
        setPending(true);
        setFailure(null);
        setIssues([]);
        try {
            const saved = await saveBackupSettings({
                settings,
                version: view.version,
                password: password === undefined || password === '' ? null : password,
            });
            queryClient.setQueryData(['backups'], saved);
            onClose();
        } catch (error) {
            setPending(false);
            const refused = issuesOf(error);
            setIssues(refused);
            setFailure(refused.length > 0 ? null : errorMessage(t, error));
        }
    };
    const issue = (field: BackupField, index: number | null = null) => {
        const found = issues.find((each) => each.field === field && (index === null || each.index === index));
        return found === undefined ? undefined : t.backups.fieldProblems[found.problem];
    };
    return { save, pending, failure, issue };
};

/** When backups run: before restarts, how long a restart waits, the fixed times. */
export function WhenDialog({ view, onClose }: { view: BackupsView; onClose: () => void }) {
    const { t } = useI18n();
    const words = t.backups.when.dialog;
    const [before, setBefore] = useState(view.settings.beforeRestarts);
    const [wait, setWait] = useState(String(view.settings.restartWaitSeconds));
    const [times, setTimes] = useState(view.settings.times);
    const [newTime, setNewTime] = useState('');
    const [timeProblem, setTimeProblem] = useState<string | null>(null);
    const { save, pending, failure, issue } = useSave(view, onClose);
    const seconds = Number(wait);
    const waitProblem =
        wait.trim() === '' || !Number.isInteger(seconds) || seconds < 0 || seconds > MAX_WAIT_SECONDS
            ? words.waitRange
            : (issue('restartWaitSeconds') ?? null);

    const add = () => {
        const value = newTime.slice(0, 5);
        if (minuteOf(value) === null) {
            setTimeProblem(words.timeIncomplete);
        } else if (times.includes(value)) {
            setTimeProblem(words.listedAlready);
        } else {
            setTimes(sortTimes([...times, value]));
            setNewTime('');
            setTimeProblem(null);
        }
    };

    return (
        <Dialog
            title={words.title}
            onClose={onClose}
            closeOnBackdrop={false}
            onSubmit={() => {
                if (waitProblem === null) {
                    void save({
                        ...view.settings,
                        beforeRestarts: before,
                        restartWaitSeconds: seconds,
                        times,
                    });
                }
            }}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.common.cancel}
                    </Button>
                    <Button type="submit" tone="ink" pending={pending}>
                        {words.save}
                    </Button>
                </>
            }
        >
            {failure !== null && <Notice tone="danger">{failure}</Notice>}
            <div className="flex flex-col gap-1.5">
                <span className="text-ui font-medium text-ink">{words.beforeRestarts}</span>
                <Segmented
                    className="self-start"
                    label={words.beforeRestarts}
                    choices={BEFORE}
                    value={before}
                    words={words.choices}
                    onChange={setBefore}
                />
                <p className="text-small text-ink-3">{words.choiceHints[before]}</p>
            </div>
            {before !== 'off' && (
                <TextField
                    label={words.wait}
                    inputMode="numeric"
                    className="max-w-48"
                    inputClassName="font-mono"
                    value={wait}
                    hint={words.waitHint}
                    error={waitProblem ?? undefined}
                    onChange={(event) => setWait(event.target.value)}
                />
            )}
            <div className="flex flex-col gap-1.5">
                <span className="text-ui font-medium text-ink">{words.times}</span>
                {times.length === 0 ? (
                    <p className="text-small text-ink-3">{words.noTimes}</p>
                ) : (
                    <ul className="flex flex-wrap gap-1.5" aria-label={words.times}>
                        {times.map((time, index) => (
                            <li
                                key={time}
                                className={cn(
                                    'inline-flex h-7 items-center gap-1 rounded-pill border border-line bg-surface-2 pr-0.5 pl-2.5 font-mono text-small text-ink',
                                    issue('time', index) !== undefined && 'border-danger',
                                )}
                            >
                                {time}
                                <button
                                    type="button"
                                    className={cn(
                                        buttonVariants({ size: 'icon-sm', tone: 'ghost' }),
                                        'size-6',
                                    )}
                                    aria-label={words.removeTime(time)}
                                    onClick={() => setTimes(times.filter((other) => other !== time))}
                                >
                                    <X size={12} strokeWidth={2} aria-hidden="true" />
                                </button>
                            </li>
                        ))}
                    </ul>
                )}
                {times.map((time, index) => {
                    const problem = issue('time', index);
                    return problem === undefined ? null : (
                        <FieldError key={time}>{`${time}: ${problem}`}</FieldError>
                    );
                })}
                {issue('times') !== undefined && <FieldError>{issue('times')}</FieldError>}
                <p className="text-small text-ink-3">{words.zone(view.zone)}</p>
            </div>
            <div className="flex flex-wrap items-end gap-2">
                <TextField
                    label={words.addTime}
                    type="time"
                    className="w-40 max-md:flex-1"
                    inputClassName="font-mono"
                    value={newTime}
                    error={timeProblem ?? undefined}
                    onChange={(event) => {
                        setNewTime(event.target.value);
                        setTimeProblem(null);
                    }}
                    onKeyDown={(event) => {
                        if (event.key === 'Enter') {
                            event.preventDefault();
                            add();
                        }
                    }}
                />
                <Button icon={Plus} onClick={add}>
                    {words.add}
                </Button>
            </div>
        </Dialog>
    );
}

/** What is dumped: the engine, the connection and where the client tools are. */
export function WhatDialog({ view, onClose }: { view: BackupsView; onClose: () => void }) {
    const { t, format } = useI18n();
    const words = t.backups.what.dialog;
    const [engine, setEngine] = useState(view.settings.engine);
    const [source, setSource] = useState(view.settings.source);
    const manual = view.settings.manual;
    const [host, setHost] = useState(manual?.host ?? '127.0.0.1');
    const [port, setPort] = useState(String(manual?.port ?? 3306));
    const [user, setUser] = useState(manual?.user ?? '');
    const [database, setDatabase] = useState(manual?.database ?? '');
    const [password, setPassword] = useState('');
    const [clientFolder, setClientFolder] = useState(view.settings.clientFolder ?? '');
    const [tested, setTested] = useState<SourceTestResult | null>(null);
    const [testing, setTesting] = useState(false);
    const [testFailure, setTestFailure] = useState<string | null>(null);
    const { save, pending, failure, issue } = useSave(view, onClose);

    const typed = source === 'manual' || manual !== null;
    const settings = (): BackupSettings => ({
        ...view.settings,
        engine,
        source,
        manual: typed
            ? { host: host.trim(), port: Number(port), user: user.trim(), database: database.trim() }
            : null,
        clientFolder: clientFolder.trim() === '' ? null : clientFolder.trim(),
    });
    const test = async () => {
        setTesting(true);
        setTested(null);
        setTestFailure(null);
        try {
            setTested(
                await testBackupSource({ settings: settings(), password: password === '' ? null : password }),
            );
        } catch (error) {
            setTestFailure(errorMessage(t, error));
        } finally {
            setTesting(false);
        }
    };
    const result = (() => {
        if (tested === null) {
            return null;
        }
        if (tested.error !== null) {
            return (
                <Notice tone="danger">
                    {t.backups.problems[tested.error]}
                    {tested.errorDetail !== null && (
                        <span className="mt-1 block font-mono text-meta [overflow-wrap:anywhere]">
                            {tested.errorDetail}
                        </span>
                    )}
                </Notice>
            );
        }
        return (
            <Notice tone="success">
                {words.tested(
                    tested.serverVersion ?? '',
                    tested.tables ?? 0,
                    format.bytes(tested.bytes ?? 0),
                )}
                {(tested.looseTables ?? 0) > 0 && (
                    <span className="mt-1 block">{words.loose(tested.looseTables ?? 0)}</span>
                )}
            </Notice>
        );
    })();

    return (
        <Dialog
            title={words.title}
            onClose={onClose}
            closeOnBackdrop={false}
            className="max-w-xl"
            onSubmit={() => void save(settings(), password)}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.common.cancel}
                    </Button>
                    <Button type="submit" tone="ink" pending={pending}>
                        {words.save}
                    </Button>
                </>
            }
        >
            {failure !== null && <Notice tone="danger">{failure}</Notice>}
            <div className="flex flex-col gap-1.5">
                <span className="text-ui font-medium text-ink">{words.engine}</span>
                <Segmented
                    className="self-start"
                    label={words.engine}
                    choices={ENGINES}
                    value={engine}
                    words={t.backups.what.engines}
                    onChange={setEngine}
                />
            </div>
            <div className="flex flex-col gap-1.5">
                <span className="text-ui font-medium text-ink">{words.source}</span>
                <Segmented
                    className="self-start"
                    label={words.source}
                    choices={SOURCES}
                    value={source}
                    words={words.sources}
                    onChange={setSource}
                />
                <p className="text-small text-ink-3">{source === 'cfg' ? words.cfgHint : words.manualHint}</p>
            </div>
            {source === 'manual' && (
                <div className="grid grid-cols-[minmax(0,1fr)_7rem] gap-3.5 max-md:grid-cols-1">
                    <TextField
                        label={words.host}
                        inputClassName="font-mono text-ui"
                        value={host}
                        error={issue('host')}
                        onChange={(event) => setHost(event.target.value)}
                    />
                    <TextField
                        label={words.port}
                        inputMode="numeric"
                        inputClassName="font-mono text-ui"
                        value={port}
                        error={issue('port')}
                        onChange={(event) => setPort(event.target.value)}
                    />
                    <TextField
                        label={words.user}
                        inputClassName="font-mono text-ui"
                        autoComplete="off"
                        value={user}
                        error={issue('user')}
                        onChange={(event) => setUser(event.target.value)}
                    />
                    <TextField
                        label={words.database}
                        inputClassName="font-mono text-ui"
                        value={database}
                        error={issue('database')}
                        onChange={(event) => setDatabase(event.target.value)}
                    />
                    <PasswordField
                        label={words.password}
                        className="col-span-full"
                        autoComplete="new-password"
                        value={password}
                        placeholder={view.hasPassword ? words.passwordStored : undefined}
                        error={issue('password')}
                        onChange={(event) => setPassword(event.target.value)}
                    />
                </div>
            )}
            <TextField
                label={
                    <>
                        {words.clientFolder}{' '}
                        <span className="font-normal text-ink-3">{t.common.optional}</span>
                    </>
                }
                inputClassName="font-mono text-ui"
                value={clientFolder}
                hint={words.clientFolderHint}
                error={issue('clientFolder')}
                onChange={(event) => setClientFolder(event.target.value)}
            />
            <div className="flex flex-wrap items-center gap-3">
                <Button pending={testing} onClick={() => void test()}>
                    {tested === null ? words.test : words.testAgain}
                </Button>
            </div>
            {testFailure !== null && <Notice tone="danger">{testFailure}</Notice>}
            {result}
        </Dialog>
    );
}
