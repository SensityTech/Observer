import { useQuery, useQueryClient } from '@tanstack/react-query';
import { FolderLock, KeySquare, Plus, Users, Workflow } from 'lucide-react';
import { useState } from 'react';

import { send } from '../api/client';
import { fileAccessesQuery, fileGroupsQuery } from '../api/files';
import type { CreatedServiceAccess } from '../api/generated/CreatedServiceAccess';
import type { FileGroup } from '../api/generated/FileGroup';
import type { ServiceAccess } from '../api/generated/ServiceAccess';
import type { UserAccess } from '../api/generated/UserAccess';
import { useMe } from '../auth/me';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { Chip } from '../components/Chip';
import { Notice } from '../components/Notice';
import { table } from '../components/table';
import {
    AssignDialog,
    ConfirmDialog,
    GroupDialog,
    PrivateKeyDialog,
    ServiceDialog,
} from '../files/FileDialogs';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';

type Open =
    | { dialog: 'group'; group: FileGroup | null }
    | { dialog: 'deleteGroup'; group: FileGroup }
    | { dialog: 'assign'; user: UserAccess }
    | { dialog: 'service' }
    | { dialog: 'key'; created: CreatedServiceAccess }
    | { dialog: 'deleteService'; service: ServiceAccess };

export function FilesPage() {
    const { t } = useI18n();
    const me = useMe();
    const queryClient = useQueryClient();
    const groups = useQuery(fileGroupsQuery);
    const accesses = useQuery(fileAccessesQuery);
    const [open, setOpen] = useState<Open | null>(null);

    const close = () => setOpen(null);
    const changed = () => {
        setOpen(null);
        void queryClient.invalidateQueries({ queryKey: ['files'] });
    };
    const names = new Map((groups.data ?? []).map((group) => [group.id, group.name]));

    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            <div className="page-head">
                <h1 className="page-title">
                    {t.files.title} <span className="gloss">{t.files.gloss}</span>
                </h1>
            </div>
            <Notice tone="neutral">{t.files.intro}</Notice>

            <Card
                id="file-groups"
                icon={FolderLock}
                title={t.files.groups}
                aside={
                    me.isOwner && (
                        <Button
                            tone="ink"
                            icon={Plus}
                            onClick={() => setOpen({ dialog: 'group', group: null })}
                        >
                            {t.files.newGroup}
                        </Button>
                    )
                }
                flush
            >
                {groups.isError && <Notice tone="danger">{errorMessage(t, groups.error)}</Notice>}
                {groups.data?.length === 0 && <p className="p-5 text-body text-ink-2">{t.files.noGroups}</p>}
                {groups.data && groups.data.length > 0 && (
                    <div className={table.scroll}>
                        <table className={table.table}>
                            <caption className="sr-only">{t.files.groups}</caption>
                            <thead>
                                <tr>
                                    <th scope="col" className={table.th}>
                                        {t.files.group.name}
                                    </th>
                                    <th scope="col" className={table.th}>
                                        {t.files.reachTitle}
                                    </th>
                                    <th scope="col" className={table.th}>
                                        {t.files.rulesTitle}
                                    </th>
                                    <th scope="col" className={table.th}>
                                        <span className="sr-only">{t.files.actions}</span>
                                    </th>
                                </tr>
                            </thead>
                            <tbody>
                                {groups.data.map((group) => (
                                    <tr key={group.id} className={table.row}>
                                        <td className={table.td}>
                                            <span className="font-medium text-ink">{group.name}</span>
                                            {group.description && (
                                                <div className={table.sub}>{group.description}</div>
                                            )}
                                        </td>
                                        <td className={table.td}>
                                            <Chip tone={group.reach === 'code' ? 'danger' : 'neutral'}>
                                                {t.files.reach[group.reach]}
                                            </Chip>
                                        </td>
                                        <td className={cn(table.td, 'text-ink-2')}>
                                            {t.files.ruleCount(group.rules.length)}
                                        </td>
                                        <td className={cn(table.td, 'w-[1%] text-right whitespace-nowrap')}>
                                            {me.isOwner && (
                                                <div className="flex gap-2">
                                                    <Button
                                                        tone="ghost"
                                                        onClick={() => setOpen({ dialog: 'group', group })}
                                                    >
                                                        {t.files.edit}
                                                    </Button>
                                                    <Button
                                                        tone="ghost"
                                                        onClick={() =>
                                                            setOpen({ dialog: 'deleteGroup', group })
                                                        }
                                                    >
                                                        {t.files.delete}
                                                    </Button>
                                                </div>
                                            )}
                                        </td>
                                    </tr>
                                ))}
                            </tbody>
                        </table>
                    </div>
                )}
            </Card>

            <Card id="file-people" icon={Users} title={t.files.people} flush>
                {accesses.isError && <Notice tone="danger">{errorMessage(t, accesses.error)}</Notice>}
                {accesses.data && (
                    <div className={table.scroll}>
                        <table className={table.table}>
                            <caption className="sr-only">{t.files.people}</caption>
                            <thead>
                                <tr>
                                    <th scope="col" className={table.th}>
                                        {t.files.account}
                                    </th>
                                    <th scope="col" className={table.th}>
                                        {t.files.groups}
                                    </th>
                                    <th scope="col" className={table.th}>
                                        {t.files.keys}
                                    </th>
                                    <th scope="col" className={table.th}>
                                        <span className="sr-only">{t.files.actions}</span>
                                    </th>
                                </tr>
                            </thead>
                            <tbody>
                                {accesses.data.users.map((user) => (
                                    <tr key={user.userId} className={table.row}>
                                        <td className={table.td}>
                                            <span className="font-medium text-ink">{user.username}</span>
                                        </td>
                                        <td className={cn(table.td, 'text-ink-2')}>
                                            {user.groups.length === 0
                                                ? t.files.noAccess
                                                : user.groups.map((id) => names.get(id) ?? id).join(', ')}
                                        </td>
                                        <td className={cn(table.td, table.mono)}>{user.keys}</td>
                                        <td className={cn(table.td, 'w-[1%] text-right whitespace-nowrap')}>
                                            {(me.isOwner || (user.userId !== me.id && !user.isOwner)) && (
                                                <Button
                                                    tone="ghost"
                                                    onClick={() => setOpen({ dialog: 'assign', user })}
                                                >
                                                    {t.files.assignGroups}
                                                </Button>
                                            )}
                                        </td>
                                    </tr>
                                ))}
                            </tbody>
                        </table>
                    </div>
                )}
            </Card>

            <Card
                id="file-pipelines"
                icon={Workflow}
                title={t.files.pipelines}
                aside={
                    me.isOwner && (
                        <Button tone="ink" icon={KeySquare} onClick={() => setOpen({ dialog: 'service' })}>
                            {t.files.newPipeline}
                        </Button>
                    )
                }
                flush
            >
                {accesses.data?.services.length === 0 && (
                    <p className="p-5 text-body text-ink-2">{t.files.noPipelines}</p>
                )}
                {accesses.data && accesses.data.services.length > 0 && (
                    <div className={table.scroll}>
                        <table className={table.table}>
                            <caption className="sr-only">{t.files.pipelines}</caption>
                            <thead>
                                <tr>
                                    <th scope="col" className={table.th}>
                                        {t.files.service.login}
                                    </th>
                                    <th scope="col" className={table.th}>
                                        {t.files.groups}
                                    </th>
                                    <th scope="col" className={table.th}>
                                        {t.files.keyEnds}
                                    </th>
                                    <th scope="col" className={table.th}>
                                        <span className="sr-only">{t.files.actions}</span>
                                    </th>
                                </tr>
                            </thead>
                            <tbody>
                                {accesses.data.services.map((service) => (
                                    <ServiceRow
                                        key={service.id}
                                        service={service}
                                        onDelete={() => setOpen({ dialog: 'deleteService', service })}
                                    />
                                ))}
                            </tbody>
                        </table>
                    </div>
                )}
            </Card>

            {open?.dialog === 'group' && <GroupDialog group={open.group} onClose={close} onSaved={changed} />}
            {open?.dialog === 'deleteGroup' && (
                <ConfirmDialog
                    title={t.files.deleteGroup(open.group.name)}
                    message={t.files.deleteGroupMessage}
                    action={t.files.delete}
                    onClose={close}
                    onConfirm={async () => {
                        await send('DELETE', `/servers/default/files/groups/${open.group.id}`);
                        changed();
                    }}
                />
            )}
            {open?.dialog === 'assign' && (
                <AssignDialog
                    username={open.user.username}
                    userId={open.user.userId}
                    held={open.user.groups}
                    onClose={close}
                    onSaved={changed}
                />
            )}
            {open?.dialog === 'service' && (
                <ServiceDialog
                    onClose={close}
                    onCreated={(created) => {
                        void queryClient.invalidateQueries({ queryKey: ['files'] });
                        setOpen({ dialog: 'key', created });
                    }}
                />
            )}
            {open?.dialog === 'key' && <PrivateKeyDialog created={open.created} onClose={close} />}
            {open?.dialog === 'deleteService' && (
                <ConfirmDialog
                    title={t.files.deletePipeline(open.service.name)}
                    message={t.files.deletePipelineMessage}
                    action={t.files.delete}
                    onClose={close}
                    onConfirm={async () => {
                        await send('DELETE', `/files/services/${open.service.id}`);
                        changed();
                    }}
                />
            )}
        </div>
    );
}

function ServiceRow({ service, onDelete }: { service: ServiceAccess; onDelete: () => void }) {
    const { t, format } = useI18n();
    const ends = service.keys.map((key) => key.expiresAt).find((end) => end !== null) ?? null;
    return (
        <tr className={table.row}>
            <td className={cn(table.td, table.mono)}>+{service.name}</td>
            <td className={cn(table.td, 'text-ink-2')}>
                {service.groups.map((group) => group.name).join(', ')}
            </td>
            <td className={cn(table.td, table.mono)}>
                {ends === null ? t.files.never : format.timestamp(ends, { seconds: false })}
            </td>
            <td className={cn(table.td, 'w-[1%] text-right whitespace-nowrap')}>
                <Button tone="ghost" onClick={onDelete}>
                    {t.files.delete}
                </Button>
            </td>
        </tr>
    );
}
