import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { FolderLock } from 'lucide-react';
import { useState } from 'react';

import { send } from '../api/client';
import { myFileAccessQuery } from '../api/files';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { CopyField } from '../components/CopyField';
import { TextField } from '../components/Field';
import { Notice } from '../components/Notice';
import { table } from '../components/table';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';

/** My account's file access: the login, the groups it holds, and the keys only its holder adds. */
export function FilesCard() {
    const { t, format } = useI18n();
    const queryClient = useQueryClient();
    const mine = useQuery(myFileAccessQuery);
    const [key, setKey] = useState('');
    const [failure, setFailure] = useState<string | null>(null);

    const refresh = () => void queryClient.invalidateQueries({ queryKey: myFileAccessQuery.queryKey });
    const add = useMutation({
        mutationFn: () => send('POST', '/account/files/keys', { publicKey: key }),
        onSuccess: () => {
            setKey('');
            setFailure(null);
            refresh();
        },
        onError: (error) => setFailure(errorMessage(t, error)),
    });
    const remove = useMutation({
        mutationFn: (id: number) => send('DELETE', `/account/files/keys/${id}`),
        onSuccess: refresh,
        onError: (error) => setFailure(errorMessage(t, error)),
    });

    return (
        <Card id="account-files" icon={FolderLock} title={t.myAccount.files.title}>
            {mine.isError && <Notice tone="danger">{errorMessage(t, mine.error)}</Notice>}
            {mine.data && (
                <div className="flex flex-col gap-4">
                    <CopyField label={t.myAccount.files.login} value={mine.data.login} />
                    <p className="text-body text-ink-2">
                        {mine.data.groups.length === 0
                            ? t.myAccount.files.noGroups
                            : t.myAccount.files.groups(
                                  mine.data.groups.map((group) => group.name).join(', '),
                              )}
                    </p>
                    {mine.data.keys.length > 0 && (
                        <div className={table.scroll}>
                            <table className={table.table}>
                                <caption className="sr-only">{t.myAccount.files.keys}</caption>
                                <thead>
                                    <tr>
                                        <th scope="col" className={table.th}>
                                            {t.myAccount.files.fingerprint}
                                        </th>
                                        <th scope="col" className={table.th}>
                                            {t.myAccount.files.lastUsed}
                                        </th>
                                        <th scope="col" className={table.th}>
                                            <span className="sr-only">{t.myAccount.files.remove}</span>
                                        </th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {mine.data.keys.map((held) => (
                                        <tr key={held.id} className={table.row}>
                                            <td className={cn(table.td, table.mono)}>
                                                {held.fingerprint}
                                                {held.comment && (
                                                    <div className={table.sub}>{held.comment}</div>
                                                )}
                                            </td>
                                            <td className={cn(table.td, table.mono)}>
                                                {held.lastUsedAt === null
                                                    ? t.myAccount.files.never
                                                    : format.timestamp(held.lastUsedAt, { seconds: false })}
                                            </td>
                                            <td
                                                className={cn(
                                                    table.td,
                                                    'w-[1%] text-right whitespace-nowrap',
                                                )}
                                            >
                                                <Button
                                                    tone="ghost"
                                                    pending={remove.isPending && remove.variables === held.id}
                                                    onClick={() => remove.mutate(held.id)}
                                                >
                                                    {t.myAccount.files.remove}
                                                </Button>
                                            </td>
                                        </tr>
                                    ))}
                                </tbody>
                            </table>
                        </div>
                    )}
                    {failure && <Notice tone="danger">{failure}</Notice>}
                    <form
                        className="flex flex-col gap-3"
                        onSubmit={(event) => {
                            event.preventDefault();
                            add.mutate();
                        }}
                    >
                        <TextField
                            label={t.myAccount.files.addKey}
                            hint={t.myAccount.files.addKeyHint}
                            spellCheck={false}
                            autoComplete="off"
                            value={key}
                            onChange={(event) => setKey(event.target.value)}
                        />
                        <Button type="submit" tone="ink" className="self-start" pending={add.isPending}>
                            {t.myAccount.files.add}
                        </Button>
                    </form>
                </div>
            )}
        </Card>
    );
}
