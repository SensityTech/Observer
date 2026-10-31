import { useQuery } from '@tanstack/react-query';
import { Plus, Trash2 } from 'lucide-react';
import { useState } from 'react';

import { send } from '../api/client';
import { fileGroupsQuery } from '../api/files';
import type { CreatedServiceAccess } from '../api/generated/CreatedServiceAccess';
import type { Depth } from '../api/generated/Depth';
import type { Effect } from '../api/generated/Effect';
import type { FileGroup } from '../api/generated/FileGroup';
import type { Right } from '../api/generated/Right';
import type { Rule } from '../api/generated/Rule';
import { Button } from '../components/Button';
import { Checkbox } from '../components/Checkbox';
import { CopyField } from '../components/CopyField';
import { Dialog } from '../components/Dialog';
import { TextField } from '../components/Field';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';

type Close = { onClose: () => void };

export const RIGHTS: Right[] = [
    'list',
    'read',
    'create',
    'modify',
    'rename',
    'delete',
    'makeFolder',
    'removeFolder',
];
const MEGABYTE = 1024 * 1024;

/** A rule as the editor holds it: names and size as typed. */
type Draft = {
    key: number;
    effect: Effect;
    folder: string;
    depth: Depth;
    names: string;
    rights: Right[];
    maxMegabytes: string;
};

/** Keys rules apart while they are edited, whatever their order. */
let drafted = 0;

const draft = (rule: Rule): Draft => ({
    key: ++drafted,
    effect: rule.effect,
    folder: rule.folder,
    depth: rule.depth,
    names: rule.names.join(', '),
    rights: rule.rights,
    maxMegabytes: rule.maxFileSize === null ? '' : String(Math.round(rule.maxFileSize / MEGABYTE)),
});

const rule = (value: Draft): Rule => ({
    effect: value.effect,
    folder: value.folder.trim().replace(/^\/+|\/+$/g, ''),
    depth: value.depth,
    names: value.names
        .split(',')
        .map((name) => name.trim())
        .filter((name) => name !== ''),
    rights: value.rights,
    maxFileSize:
        value.effect === 'allow' && value.maxMegabytes.trim() !== ''
            ? Number(value.maxMegabytes) * MEGABYTE
            : null,
});

const blank = (): Draft => ({
    key: ++drafted,
    effect: 'allow',
    folder: 'resources',
    depth: 'tree',
    names: '',
    rights: ['list', 'read'],
    maxMegabytes: '',
});

export function GroupDialog({
    group,
    onClose,
    onSaved,
}: Close & { group: FileGroup | null; onSaved: () => void }) {
    const { t } = useI18n();
    const [name, setName] = useState(group?.name ?? '');
    const [description, setDescription] = useState(group?.description ?? '');
    const [rules, setRules] = useState<Draft[]>(() => (group ? group.rules.map(draft) : [blank()]));
    const [failure, setFailure] = useState<string | null>(null);
    const [pending, setPending] = useState(false);

    const update = (index: number, change: Partial<Draft>) =>
        setRules((current) => current.map((value, at) => (at === index ? { ...value, ...change } : value)));

    const submit = async () => {
        if (pending) {
            return;
        }
        setPending(true);
        setFailure(null);
        const body = { name, description, rules: rules.map(rule) };
        try {
            if (group) {
                await send('PUT', `/servers/default/files/groups/${group.id}`, {
                    ...body,
                    version: group.version,
                });
            } else {
                await send('POST', '/servers/default/files/groups', body);
            }
            onSaved();
        } catch (error) {
            setPending(false);
            setFailure(errorMessage(t, error));
        }
    };

    return (
        <Dialog
            title={group ? t.files.group.edit : t.files.group.create}
            onClose={onClose}
            onSubmit={submit}
            closeOnBackdrop={false}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.common.cancel}
                    </Button>
                    <Button type="submit" tone="ink" pending={pending}>
                        {t.files.group.save}
                    </Button>
                </>
            }
        >
            {failure && <Notice tone="danger">{failure}</Notice>}
            <TextField
                label={t.files.group.name}
                value={name}
                onChange={(event) => setName(event.target.value)}
            />
            <TextField
                label={t.files.group.description}
                value={description}
                onChange={(event) => setDescription(event.target.value)}
            />
            {rules.map((value, index) => (
                <fieldset
                    key={value.key}
                    className="flex flex-col gap-3 rounded-[8px] border border-line p-3"
                >
                    <legend className="px-1 text-label text-ink-2">{t.files.group.rule(index + 1)}</legend>
                    <div className="flex flex-wrap gap-3">
                        <label className="flex flex-col gap-1 text-label text-ink-2">
                            {t.files.group.effect}
                            <select
                                className="field-input"
                                value={value.effect}
                                onChange={(event) => update(index, { effect: event.target.value as Effect })}
                            >
                                <option value="allow">{t.files.effects.allow}</option>
                                <option value="deny">{t.files.effects.deny}</option>
                            </select>
                        </label>
                        <label className="flex flex-col gap-1 text-label text-ink-2">
                            {t.files.group.depth}
                            <select
                                className="field-input"
                                value={value.depth}
                                onChange={(event) => update(index, { depth: event.target.value as Depth })}
                            >
                                <option value="tree">{t.files.depths.tree}</option>
                                <option value="folder">{t.files.depths.folder}</option>
                            </select>
                        </label>
                    </div>
                    <TextField
                        label={t.files.group.folder}
                        hint={t.files.group.folderHint}
                        spellCheck={false}
                        value={value.folder}
                        onChange={(event) => update(index, { folder: event.target.value })}
                    />
                    <TextField
                        label={t.files.group.names}
                        hint={t.files.group.namesHint}
                        spellCheck={false}
                        value={value.names}
                        onChange={(event) => update(index, { names: event.target.value })}
                    />
                    <fieldset className="flex flex-wrap gap-x-4 gap-y-2">
                        <legend className="sr-only">{t.files.group.rights}</legend>
                        {RIGHTS.map((right) => (
                            <label key={right} className="flex items-center gap-2 text-body text-ink">
                                <Checkbox
                                    checked={value.rights.includes(right)}
                                    onChange={(event) =>
                                        update(index, {
                                            rights: event.target.checked
                                                ? [...value.rights, right]
                                                : value.rights.filter((held) => held !== right),
                                        })
                                    }
                                />
                                {t.files.rights[right]}
                            </label>
                        ))}
                    </fieldset>
                    {value.effect === 'allow' && (
                        <TextField
                            label={t.files.group.maxSize}
                            inputMode="numeric"
                            value={value.maxMegabytes}
                            onChange={(event) => update(index, { maxMegabytes: event.target.value })}
                        />
                    )}
                    <Button
                        tone="ghost"
                        icon={Trash2}
                        onClick={() => setRules((current) => current.filter((_, at) => at !== index))}
                    >
                        {t.files.group.removeRule}
                    </Button>
                </fieldset>
            ))}
            <Button tone="ghost" icon={Plus} onClick={() => setRules((current) => [...current, blank()])}>
                {t.files.group.addRule}
            </Button>
        </Dialog>
    );
}

/** The groups an account holds, chosen among the server's. */
export function AssignDialog({
    username,
    userId,
    held,
    onClose,
    onSaved,
}: Close & { username: string; userId: number; held: number[]; onSaved: () => void }) {
    const { t } = useI18n();
    const groups = useQuery(fileGroupsQuery);
    const [chosen, setChosen] = useState<number[]>(held);
    const [failure, setFailure] = useState<string | null>(null);
    const [pending, setPending] = useState(false);

    const submit = async () => {
        if (pending) {
            return;
        }
        setPending(true);
        setFailure(null);
        try {
            await send('PUT', `/files/accesses/users/${userId}`, { groups: chosen });
            onSaved();
        } catch (error) {
            setPending(false);
            setFailure(errorMessage(t, error));
        }
    };

    return (
        <Dialog
            title={t.files.assign.title(username)}
            onClose={onClose}
            onSubmit={submit}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.common.cancel}
                    </Button>
                    <Button type="submit" tone="ink" pending={pending}>
                        {t.files.assign.save}
                    </Button>
                </>
            }
        >
            {failure && <Notice tone="danger">{failure}</Notice>}
            {groups.isError && <Notice tone="danger">{errorMessage(t, groups.error)}</Notice>}
            {groups.data?.length === 0 && <Notice tone="neutral">{t.files.assign.none}</Notice>}
            {groups.data?.map((group) => (
                <label key={group.id} className="flex items-start gap-2 text-body text-ink">
                    <Checkbox
                        checked={chosen.includes(group.id)}
                        onChange={(event) =>
                            setChosen((current) =>
                                event.target.checked
                                    ? [...current, group.id]
                                    : current.filter((id) => id !== group.id),
                            )
                        }
                    />
                    <span>
                        {group.name} <span className="text-ink-3">· {t.files.reach[group.reach]}</span>
                    </span>
                </label>
            ))}
        </Dialog>
    );
}

export function ServiceDialog({
    onClose,
    onCreated,
}: Close & { onCreated: (created: CreatedServiceAccess) => void }) {
    const { t } = useI18n();
    const groups = useQuery(fileGroupsQuery);
    const [name, setName] = useState('');
    const [networks, setNetworks] = useState('');
    const [days, setDays] = useState('90');
    const [chosen, setChosen] = useState<number[]>([]);
    const [failure, setFailure] = useState<string | null>(null);
    const [pending, setPending] = useState(false);

    const submit = async () => {
        if (pending) {
            return;
        }
        setPending(true);
        setFailure(null);
        try {
            onCreated(
                await send<CreatedServiceAccess>('POST', '/files/services', {
                    name,
                    groups: chosen,
                    networks: networks
                        .split(/[\s,]+/)
                        .map((network) => network.trim())
                        .filter((network) => network !== ''),
                    expiresInDays: Number(days),
                }),
            );
        } catch (error) {
            setPending(false);
            setFailure(errorMessage(t, error));
        }
    };

    return (
        <Dialog
            title={t.files.service.create}
            onClose={onClose}
            onSubmit={submit}
            closeOnBackdrop={false}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.common.cancel}
                    </Button>
                    <Button type="submit" tone="ink" pending={pending}>
                        {t.files.service.submit}
                    </Button>
                </>
            }
        >
            {failure && <Notice tone="danger">{failure}</Notice>}
            <TextField
                label={t.files.service.name}
                hint={t.files.service.nameHint}
                autoCapitalize="none"
                spellCheck={false}
                value={name}
                onChange={(event) => setName(event.target.value)}
            />
            <TextField
                label={t.files.service.networks}
                hint={t.files.service.networksHint}
                spellCheck={false}
                value={networks}
                onChange={(event) => setNetworks(event.target.value)}
            />
            <TextField
                label={t.files.service.days}
                inputMode="numeric"
                value={days}
                onChange={(event) => setDays(event.target.value)}
            />
            <fieldset className="flex flex-col gap-2">
                <legend className="sr-only">{t.files.service.groups}</legend>
                {groups.data?.map((group) => (
                    <label key={group.id} className="flex items-start gap-2 text-body text-ink">
                        <Checkbox
                            checked={chosen.includes(group.id)}
                            onChange={(event) =>
                                setChosen((current) =>
                                    event.target.checked
                                        ? [...current, group.id]
                                        : current.filter((id) => id !== group.id),
                                )
                            }
                        />
                        <span>
                            {group.name} <span className="text-ink-3">· {t.files.reach[group.reach]}</span>
                        </span>
                    </label>
                ))}
            </fieldset>
        </Dialog>
    );
}

/** The private key of a new pipeline, shown once: closing the dialog is the only way out. */
export function PrivateKeyDialog({ created, onClose }: Close & { created: CreatedServiceAccess }) {
    const { t } = useI18n();
    return (
        <Dialog
            title={t.files.service.keyTitle(created.login)}
            onClose={onClose}
            closeOnBackdrop={false}
            footer={
                <Button tone="ink" onClick={onClose}>
                    {t.files.service.saved}
                </Button>
            }
        >
            <Notice tone="warning">{t.files.service.keyOnce}</Notice>
            <CopyField label={t.files.service.login} value={created.login} />
            <CopyField label={t.files.service.privateKey} value={created.privateKey} />
        </Dialog>
    );
}

export function ConfirmDialog({
    title,
    message,
    action,
    onClose,
    onConfirm,
}: Close & { title: string; message: string; action: string; onConfirm: () => Promise<void> }) {
    const { t } = useI18n();
    const [failure, setFailure] = useState<string | null>(null);
    const [pending, setPending] = useState(false);
    const submit = async () => {
        if (pending) {
            return;
        }
        setPending(true);
        try {
            await onConfirm();
        } catch (error) {
            setPending(false);
            setFailure(errorMessage(t, error));
        }
    };
    return (
        <Dialog
            title={title}
            onClose={onClose}
            onSubmit={submit}
            role="alertdialog"
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.common.cancel}
                    </Button>
                    <Button type="submit" tone="danger" pending={pending}>
                        {action}
                    </Button>
                </>
            }
        >
            {failure && <Notice tone="danger">{failure}</Notice>}
            <p className="text-body text-ink-2">{message}</p>
        </Dialog>
    );
}
