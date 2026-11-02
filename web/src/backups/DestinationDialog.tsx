import { useState } from 'react';

import { ApiError } from '../api/client';
import {
    addDestination,
    awsEndpoint,
    changeDestination,
    kindOf,
    pathStyleFor,
    regionOf,
    testDestination,
    type DestinationKind,
} from '../api/backups';
import type { BackupDestination } from '../api/generated/BackupDestination';
import type { DestinationDraft } from '../api/generated/DestinationDraft';
import type { DestinationField } from '../api/generated/DestinationField';
import type { DestinationIssue } from '../api/generated/DestinationIssue';
import type { DestinationTarget } from '../api/generated/DestinationTarget';
import type { DestinationTestResult } from '../api/generated/DestinationTestResult';
import type { SftpAuth } from '../api/generated/SftpAuth';
import { CheckRow } from '../components/CheckRow';
import { CopyField } from '../components/CopyField';
import { Button } from '../components/Button';
import { Dialog } from '../components/Dialog';
import { PasswordField, TextField } from '../components/Field';
import { Notice } from '../components/Notice';
import { Segmented } from '../components/Segmented';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { KIND_ICONS } from './icons';

const KINDS: DestinationKind[] = ['local', 's3', 'sftp', 'share'];
const AUTHS: SftpAuth[] = ['key', 'password'];
const UNITS = ['last', 'days'] as const;
const DEFAULT_KEEP = 14;

/** A destination as the dialog holds it: every kind's fields, as typed. */
type Draft = {
    kind: DestinationKind;
    name: string;
    enabled: boolean;
    folder: string;
    endpoint: string;
    region: string;
    bucket: string;
    prefix: string;
    accessKeyId: string;
    host: string;
    port: string;
    user: string;
    auth: SftpAuth;
    hostKey: string | null;
    secret: string;
    keep: string;
    unit: (typeof UNITS)[number];
};

const draftOf = (destination: BackupDestination | null): Draft => {
    const target = destination?.target;
    const retention = destination?.retention;
    return {
        kind: target === undefined ? 'local' : kindOf(target),
        name: destination?.name ?? '',
        enabled: destination?.enabled ?? true,
        folder: target?.kind === 'local' || target?.kind === 'sftp' ? target.folder : '',
        endpoint: target?.kind === 's3' ? target.endpoint : '',
        region: target?.kind === 's3' ? target.region : '',
        bucket: target?.kind === 's3' ? target.bucket : '',
        prefix: target?.kind === 's3' ? target.prefix : '',
        accessKeyId: target?.kind === 's3' ? target.accessKeyId : '',
        host: target?.kind === 'sftp' ? target.host : '',
        port: target?.kind === 'sftp' ? String(target.port) : '22',
        user: target?.kind === 'sftp' ? target.user : '',
        auth: target?.kind === 'sftp' ? target.auth : 'key',
        hostKey: target?.kind === 'sftp' ? target.hostKey : null,
        secret: '',
        keep: String(retention?.keepLast ?? retention?.keepDays ?? DEFAULT_KEEP),
        unit: retention !== undefined && retention.keepLast === null ? 'days' : 'last',
    };
};

const targetOf = (draft: Draft): DestinationTarget => {
    switch (draft.kind) {
        case 'local':
        case 'share':
            return { kind: 'local', folder: draft.folder.trim() };
        case 'sftp':
            return {
                kind: 'sftp',
                host: draft.host.trim(),
                port: Number(draft.port),
                user: draft.user.trim(),
                folder: draft.folder.trim(),
                auth: draft.auth,
                hostKey: draft.hostKey,
            };
        case 's3': {
            const typed = draft.endpoint.trim();
            const region = draft.region.trim() || regionOf(typed);
            const endpoint = typed === '' ? awsEndpoint(region) : typed;
            const prefix = draft.prefix.trim();
            return {
                kind: 's3',
                endpoint,
                region,
                bucket: draft.bucket.trim(),
                prefix: prefix === '' || prefix.endsWith('/') ? prefix : `${prefix}/`,
                accessKeyId: draft.accessKeyId.trim(),
                pathStyle: pathStyleFor(endpoint),
            };
        }
    }
};

/** What the API takes; an empty secret keeps the one stored. */
const toApi = (draft: Draft): DestinationDraft => {
    const keep = Number(draft.keep);
    const takesSecret = draft.kind === 's3' || (draft.kind === 'sftp' && draft.auth === 'password');
    return {
        name: draft.name.trim(),
        enabled: draft.enabled,
        target: targetOf(draft),
        retention:
            draft.unit === 'last' ? { keepLast: keep, keepDays: null } : { keepLast: null, keepDays: keep },
        secret: takesSecret && draft.secret !== '' ? draft.secret : null,
    };
};

const issuesOf = (error: unknown): DestinationIssue[] =>
    error instanceof ApiError &&
    error.code === 'invalid_backup_destination' &&
    Array.isArray(error.params.issues)
        ? (error.params.issues as DestinationIssue[])
        : [];

/**
 * Adds a destination, or changes one. A new SFTP destination that logs in with Observer's key stays
 * open once added: its key exists from then on, to put on the server before the test.
 */
export function DestinationDialog({
    destination,
    onClose,
    onSaved,
}: {
    destination: BackupDestination | null;
    onClose: () => void;
    onSaved: () => void;
}) {
    const { t } = useI18n();
    const words = t.backups.dialog;
    const [current, setCurrent] = useState(destination);
    const [draft, setDraft] = useState(() => draftOf(destination));
    const [issues, setIssues] = useState<DestinationIssue[]>([]);
    const [failure, setFailure] = useState<string | null>(null);
    const [pending, setPending] = useState(false);
    const [tested, setTested] = useState<{ result: DestinationTestResult; ms: number } | null>(null);
    const [testing, setTesting] = useState(false);
    const [justAdded, setJustAdded] = useState(false);

    const edit = (change: Partial<Draft>) => {
        setDraft((value) => ({ ...value, ...change }));
        setIssues([]);
    };
    const issue = (field: DestinationField) => {
        const found = issues.find((each) => each.field === field);
        return found === undefined ? undefined : t.backups.fieldProblems[found.problem];
    };
    const keyLogin = draft.kind === 'sftp' && draft.auth === 'key';

    const submit = async () => {
        if (pending) {
            return;
        }
        setPending(true);
        setFailure(null);
        setIssues([]);
        try {
            const saved =
                current === null
                    ? await addDestination(toApi(draft))
                    : await changeDestination(current.id, {
                          destination: toApi(draft),
                          version: current.version,
                      });
            onSaved();
            if (current === null && keyLogin) {
                setCurrent(saved);
                setDraft(draftOf(saved));
                setJustAdded(true);
                setPending(false);
                return;
            }
            onClose();
        } catch (error) {
            setPending(false);
            const refused = issuesOf(error);
            setIssues(refused);
            setFailure(refused.length > 0 ? null : errorMessage(t, error));
        }
    };

    const test = async () => {
        setTesting(true);
        setTested(null);
        setFailure(null);
        const started = performance.now();
        try {
            const result = await testDestination({ destination: toApi(draft), id: current?.id ?? null });
            setTested({ result, ms: performance.now() - started });
        } catch (error) {
            const refused = issuesOf(error);
            setIssues(refused);
            setFailure(refused.length > 0 ? null : errorMessage(t, error));
        } finally {
            setTesting(false);
        }
    };

    const shown = tested?.result.hostKey ?? null;
    const pinned = current?.target.kind === 'sftp' ? current.target.hostKey : null;
    const lastFailure = current?.last?.state === 'failed' ? current.last : null;

    return (
        <Dialog
            title={current === null ? words.addTitle : words.editTitle(current.name)}
            onClose={onClose}
            onSubmit={() => void submit()}
            closeOnBackdrop={false}
            className="max-w-2xl"
            footer={
                <>
                    <span className="mr-auto self-center text-small text-ink-3 max-md:w-full">
                        {words.reauth}
                    </span>
                    <Button tone="ghost" onClick={onClose}>
                        {justAdded ? t.common.close : t.common.cancel}
                    </Button>
                    <Button type="submit" tone="ink" pending={pending}>
                        {current === null ? words.add : words.save}
                    </Button>
                </>
            }
        >
            {failure !== null && <Notice tone="danger">{failure}</Notice>}
            {lastFailure !== null && !justAdded && (
                <Notice tone="danger" live={false}>
                    {words.lastFailed(
                        lastFailure.error === null ? '' : t.backups.problems[lastFailure.error],
                    )}
                    {lastFailure.errorDetail !== null && (
                        <span className="mt-1 block font-mono text-meta [overflow-wrap:anywhere]">
                            {lastFailure.errorDetail}
                        </span>
                    )}
                </Notice>
            )}
            {justAdded && <Notice tone="success">{words.added}</Notice>}
            <fieldset disabled={current !== null} className="flex flex-col gap-2">
                <legend className="mb-2 text-ui font-medium text-ink">{words.where}</legend>
                <div className="grid grid-cols-4 gap-2 max-md:grid-cols-2">
                    {KINDS.map((kind) => {
                        const Icon = KIND_ICONS[kind];
                        return (
                            <label
                                key={kind}
                                className="flex cursor-pointer flex-col gap-2 rounded-box border border-line-strong p-3 has-checked:border-accent has-checked:bg-accent-soft has-disabled:cursor-default has-focus-visible:outline-2 has-focus-visible:outline-offset-2 has-focus-visible:outline-accent"
                            >
                                <input
                                    type="radio"
                                    name="destination-kind"
                                    className="sr-only"
                                    checked={draft.kind === kind}
                                    onChange={() => edit({ kind })}
                                />
                                <Icon size={16} strokeWidth={1.5} className="text-ink-2" aria-hidden="true" />
                                <span className="text-ui font-medium text-ink">
                                    {t.backups.destinations.kinds[kind]}
                                </span>
                                <span className="text-small text-ink-3">{words.kindHints[kind]}</span>
                            </label>
                        );
                    })}
                </div>
            </fieldset>
            <div className="grid grid-cols-2 gap-3.5 max-md:grid-cols-1">
                <TextField
                    label={words.name}
                    className="col-span-full"
                    value={draft.name}
                    error={issue('name')}
                    onChange={(event) => edit({ name: event.target.value })}
                />
                {(draft.kind === 'local' || draft.kind === 'share') && (
                    <TextField
                        label={words.folder}
                        className="col-span-full"
                        inputClassName="font-mono text-ui"
                        spellCheck={false}
                        value={draft.folder}
                        placeholder={draft.kind === 'share' ? '\\\\nas\\backups' : undefined}
                        hint={draft.kind === 'share' ? words.shareHint : words.localHint}
                        error={issue('folder')}
                        onChange={(event) => edit({ folder: event.target.value })}
                    />
                )}
                {draft.kind === 's3' && (
                    <>
                        <TextField
                            label={words.endpoint}
                            className="col-span-full"
                            inputClassName="font-mono text-ui"
                            spellCheck={false}
                            value={draft.endpoint}
                            placeholder="https://"
                            hint={words.endpointHint}
                            error={issue('endpoint')}
                            onChange={(event) => edit({ endpoint: event.target.value })}
                        />
                        <TextField
                            label={words.bucket}
                            inputClassName="font-mono text-ui"
                            spellCheck={false}
                            value={draft.bucket}
                            error={issue('bucket')}
                            onChange={(event) => edit({ bucket: event.target.value })}
                        />
                        <TextField
                            label={words.prefix}
                            inputClassName="font-mono text-ui"
                            spellCheck={false}
                            value={draft.prefix}
                            error={issue('prefix')}
                            onChange={(event) => edit({ prefix: event.target.value })}
                        />
                        <TextField
                            label={words.region}
                            inputClassName="font-mono text-ui"
                            spellCheck={false}
                            value={draft.region}
                            placeholder={regionOf(draft.endpoint)}
                            hint={words.regionHint}
                            error={issue('region')}
                            onChange={(event) => edit({ region: event.target.value })}
                        />
                        <TextField
                            label={words.accessKey}
                            inputClassName="font-mono text-ui"
                            spellCheck={false}
                            autoComplete="off"
                            value={draft.accessKeyId}
                            error={issue('accessKeyId')}
                            onChange={(event) => edit({ accessKeyId: event.target.value })}
                        />
                        <PasswordField
                            label={words.secretKey}
                            className="col-span-full"
                            autoComplete="new-password"
                            value={draft.secret}
                            placeholder={current?.hasSecret === true ? words.secretKept : undefined}
                            hint={words.secretHint}
                            error={issue('secret')}
                            onChange={(event) => edit({ secret: event.target.value })}
                        />
                    </>
                )}
                {draft.kind === 'sftp' && (
                    <>
                        <TextField
                            label={words.host}
                            inputClassName="font-mono text-ui"
                            spellCheck={false}
                            value={draft.host}
                            error={issue('host')}
                            onChange={(event) => edit({ host: event.target.value, hostKey: null })}
                        />
                        <TextField
                            label={words.port}
                            inputMode="numeric"
                            inputClassName="font-mono text-ui"
                            value={draft.port}
                            error={issue('port')}
                            onChange={(event) => edit({ port: event.target.value, hostKey: null })}
                        />
                        <TextField
                            label={words.user}
                            inputClassName="font-mono text-ui"
                            spellCheck={false}
                            autoComplete="off"
                            value={draft.user}
                            error={issue('user')}
                            onChange={(event) => edit({ user: event.target.value })}
                        />
                        <TextField
                            label={words.remoteFolder}
                            inputClassName="font-mono text-ui"
                            spellCheck={false}
                            value={draft.folder}
                            hint={words.remoteFolderHint}
                            error={issue('folder')}
                            onChange={(event) => edit({ folder: event.target.value })}
                        />
                        <div className="col-span-full flex flex-col gap-1.5">
                            <span className="text-ui font-medium text-ink">{words.auth}</span>
                            <Segmented
                                className="self-start"
                                label={words.auth}
                                choices={AUTHS}
                                value={draft.auth}
                                words={words.auths}
                                onChange={(auth) => edit({ auth })}
                            />
                        </div>
                        {draft.auth === 'password' ? (
                            <PasswordField
                                label={words.password}
                                className="col-span-full"
                                autoComplete="new-password"
                                value={draft.secret}
                                placeholder={current?.hasSecret === true ? words.secretKept : undefined}
                                hint={words.secretHint}
                                error={issue('secret')}
                                onChange={(event) => edit({ secret: event.target.value })}
                            />
                        ) : current?.publicKey != null &&
                          current.target.kind === 'sftp' &&
                          current.target.auth === 'key' ? (
                            <div className="col-span-full flex flex-col gap-1.5">
                                <CopyField label={words.publicKey} value={current.publicKey} />
                                <p className="text-small text-ink-3">{words.publicKeyHint}</p>
                            </div>
                        ) : (
                            <p className="col-span-full text-small text-ink-3">{words.keyAfterAdd}</p>
                        )}
                    </>
                )}
                <div className="col-span-full flex flex-col gap-1.5">
                    <span className="text-ui font-medium text-ink">{words.keep}</span>
                    <div className="flex flex-wrap items-center gap-2">
                        <TextField
                            label={<span className="sr-only">{words.keepCount}</span>}
                            className="w-24"
                            inputMode="numeric"
                            inputClassName="font-mono"
                            value={draft.keep}
                            error={issue('keepLast') ?? issue('keepDays') ?? issue('retention')}
                            onChange={(event) => edit({ keep: event.target.value })}
                        />
                        <Segmented
                            label={words.keepUnit}
                            choices={UNITS}
                            value={draft.unit}
                            words={words.units}
                            onChange={(unit) => edit({ unit })}
                        />
                    </div>
                    <p className="text-small text-ink-3">{words.keepHint}</p>
                </div>
                {current !== null && (
                    <div className="col-span-full -mx-2">
                        <CheckRow
                            label={words.enabled}
                            description={words.enabledHint}
                            checked={draft.enabled}
                            onChange={(enabled) => edit({ enabled })}
                        />
                    </div>
                )}
            </div>
            <TestResult
                tested={tested}
                testing={testing}
                disabled={keyLogin && current === null}
                onTest={() => void test()}
            />
            {draft.kind === 'sftp' && shown !== null && (
                <div className="-mx-2">
                    <CheckRow
                        label={words.trust(shown.fingerprint)}
                        description={words.trustHint}
                        checked={draft.hostKey === shown.key}
                        onChange={(trust) => edit({ hostKey: trust ? shown.key : pinned })}
                    />
                </div>
            )}
        </Dialog>
    );
}

/** The test button, and what the last test said. */
function TestResult({
    tested,
    testing,
    disabled,
    onTest,
}: {
    tested: { result: DestinationTestResult; ms: number } | null;
    testing: boolean;
    disabled: boolean;
    onTest: () => void;
}) {
    const { t, format } = useI18n();
    const words = t.backups.dialog;
    const result = tested?.result ?? null;
    return (
        <div className="flex flex-col gap-2">
            {result !== null && (
                <Notice tone={result.error === null ? 'success' : 'danger'}>
                    {result.error === null
                        ? words.testedOk(format.milliseconds(tested?.ms ?? 0))
                        : words.testedFailed(t.backups.problems[result.error])}
                    {result.errorDetail !== null && (
                        <span className="mt-1 block font-mono text-meta [overflow-wrap:anywhere]">
                            {result.errorDetail}
                        </span>
                    )}
                    {result.error === 'hostKeyMismatch' && result.hostKey !== null && (
                        <span className="mt-1 block">{words.shownKey(result.hostKey.fingerprint)}</span>
                    )}
                </Notice>
            )}
            <div className="flex flex-wrap items-center gap-3">
                <Button pending={testing} disabled={disabled} onClick={onTest}>
                    {result === null ? words.test : words.testAgain}
                </Button>
                {disabled && <span className="text-small text-ink-3">{words.testAfterAdd}</span>}
            </div>
        </div>
    );
}
