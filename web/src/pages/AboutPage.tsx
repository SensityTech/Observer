import { useQuery } from '@tanstack/react-query';
import { Info, Package, Search } from 'lucide-react';
import { useDeferredValue, useMemo, useState, type ReactNode } from 'react';

import { aboutQuery, licencesQuery } from '../api/about';
import { ApiError } from '../api/client';
import type { About } from '../api/generated/About';
import type { Ecosystem } from '../api/generated/Ecosystem';
import type { ThirdPartyLicences } from '../api/generated/ThirdPartyLicences';
import type { ThirdPartyPackage } from '../api/generated/ThirdPartyPackage';
import { useMe } from '../auth/me';
import { ATTRIBUTION } from '../charts/PlayersChart';
import { Button } from '../components/Button';
import { Dialog } from '../components/Dialog';
import { inputClass } from '../components/Field';
import { Notice } from '../components/Notice';
import { table } from '../components/table';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { UpdatesCard } from '../updates/UpdatesCard';

const ECOSYSTEMS: Ecosystem[] = ['crate', 'npm'];

export function AboutPage() {
    const { t } = useI18n();
    const me = useMe();

    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            <div className="page-head">
                <h1 className="page-title">
                    {t.about.title} <span className="gloss">{t.about.gloss}</span>
                </h1>
            </div>
            <div className="flex flex-col gap-3">
                {me.isOwner && <UpdatesCard />}
                <BuildCard />
                <LicencesCard />
            </div>
        </div>
    );
}

function CardHead({
    id,
    icon: Icon,
    title,
    meta,
}: {
    id: string;
    icon: typeof Info;
    title: string;
    meta?: string;
}) {
    return (
        <div className="card-head wrap-push">
            <span className="grid size-6 place-items-center rounded-[6px] border border-line bg-surface-2 text-ink-2">
                <Icon size={14} strokeWidth={1.5} aria-hidden="true" />
            </span>
            <h2 id={id} className="text-card text-ink">
                {title}
            </h2>
            {meta !== undefined && (
                <span className="text-meta text-ink-3" data-push>
                    {meta}
                </span>
            )}
        </div>
    );
}

function BuildCard() {
    const { t } = useI18n();
    const about = useQuery(aboutQuery);

    return (
        <section className="card flex flex-col" aria-labelledby="about-build">
            <CardHead id="about-build" icon={Info} title={t.about.build} />
            {about.isPending && (
                <div className="flex flex-col gap-3 p-5" aria-busy="true">
                    <span className="sr-only">{t.common.loading}</span>
                    <span className="skeleton w-1/2" />
                </div>
            )}
            {about.isError && (
                <div className="p-5 max-md:p-4">
                    <Notice tone="danger">{errorMessage(t, about.error)}</Notice>
                </div>
            )}
            {about.data && <BuildFacts about={about.data} />}
        </section>
    );
}

const faint = (text: string) => <span className="text-ink-3">{text}</span>;
const mono = (text: string) => <span className="font-mono">{text}</span>;

function BuildFacts({ about }: { about: About }) {
    const { t, format } = useI18n();
    // The commit's own day, whatever the reader's zone: it names a date, not an instant.
    const date =
        about.commitDate === null
            ? null
            : format.dateTime(new Date(`${about.commitDate}T12:00:00Z`), {
                  dateStyle: 'medium',
                  timeStyle: undefined,
                  timeZone: 'UTC',
              });
    const facts: { term: string; value: ReactNode }[] = [
        { term: t.about.version, value: mono(about.version) },
        {
            term: t.about.commit,
            value: about.commit === null ? faint(t.about.development) : mono(about.commit),
        },
        { term: t.about.commitDate, value: date ?? faint(t.status.unknown) },
        { term: t.about.platform, value: `${t.about.systems[about.os]} ${about.arch}` },
        { term: t.about.profile, value: t.about.profiles[about.profile] },
    ];
    return (
        <dl className="grid grid-cols-2 gap-x-6 gap-y-3 p-5 max-md:grid-cols-1 max-md:p-4">
            {facts.map(({ term, value }) => (
                <div key={term} className="flex flex-col gap-0.5">
                    <dt className="text-meta text-ink-3">{term}</dt>
                    <dd className="text-body text-ink">{value}</dd>
                </div>
            ))}
        </dl>
    );
}

function LicencesCard() {
    const { t } = useI18n();
    const licences = useQuery(licencesQuery);
    const [search, setSearch] = useState('');
    const needle = useDeferredValue(search.trim().toLowerCase());
    const [open, setOpen] = useState<ThirdPartyPackage | null>(null);
    const missing = licences.error instanceof ApiError && licences.error.code === 'not_found';
    const counted = licences.data?.packages;
    const meta =
        counted === undefined
            ? undefined
            : t.about.counts(
                  counted.filter((item) => item.ecosystem === 'crate').length,
                  counted.filter((item) => item.ecosystem === 'npm').length,
              );

    return (
        <section className="card flex flex-col" aria-labelledby="about-licences">
            <CardHead id="about-licences" icon={Package} title={t.about.thirdParty} meta={meta} />
            {licences.isPending && (
                <div className="flex flex-col gap-3 p-5" aria-busy="true">
                    <span className="sr-only">{t.common.loading}</span>
                    <span className="skeleton w-2/3" />
                    <span className="skeleton w-1/2" />
                </div>
            )}
            {licences.isError && (
                <div className="p-5 max-md:p-4">
                    {missing ? (
                        <Notice tone="neutral">{t.about.missing}</Notice>
                    ) : (
                        <Notice tone="danger">
                            {t.about.loadFailed} {errorMessage(t, licences.error)}
                        </Notice>
                    )}
                </div>
            )}
            {licences.data && (
                <>
                    <div className="px-5 pt-4 max-md:px-4">
                        <label className="relative block max-w-search">
                            <span className="sr-only">{t.about.search}</span>
                            <Search
                                size={14}
                                strokeWidth={1.5}
                                className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-ink-3"
                                aria-hidden="true"
                            />
                            <input
                                type="search"
                                value={search}
                                onChange={(event) => setSearch(event.target.value)}
                                placeholder={t.about.search}
                                className={cn(inputClass, 'pl-8')}
                            />
                        </label>
                    </div>
                    <Packages licences={licences.data} needle={needle} onOpen={setOpen} />
                </>
            )}
            <footer className="flex flex-wrap items-center gap-1 rounded-b-card border-t border-line bg-bg-subtle py-2 pr-3 pl-5 text-meta text-ink-3">
                {t.charts.attribution}{' '}
                <a
                    href={ATTRIBUTION}
                    target="_blank"
                    rel="noreferrer"
                    className="underline-offset-2 hover:text-ink-2 hover:underline"
                >
                    {t.charts.library}
                </a>
            </footer>
            {open && licences.data && (
                <LicenceText item={open} texts={licences.data.texts} onClose={() => setOpen(null)} />
            )}
        </section>
    );
}

function Packages({
    licences,
    needle,
    onOpen,
}: {
    licences: ThirdPartyLicences;
    needle: string;
    onOpen: (item: ThirdPartyPackage) => void;
}) {
    const { t } = useI18n();
    const shown = useMemo(
        () =>
            licences.packages.filter(
                (item) =>
                    needle === '' ||
                    item.name.toLowerCase().includes(needle) ||
                    (item.licence ?? '').toLowerCase().includes(needle),
            ),
        [licences, needle],
    );
    if (shown.length === 0) {
        return <p className="px-5 py-4 text-ui text-ink-3 max-md:px-4">{t.about.noMatch}</p>;
    }
    return (
        <div className="flex flex-col gap-2 pt-3">
            {ECOSYSTEMS.map((ecosystem) => {
                const rows = shown.filter((item) => item.ecosystem === ecosystem);
                if (rows.length === 0) {
                    return null;
                }
                return (
                    <div key={ecosystem} className={table.scroll}>
                        <table className={table.table}>
                            <caption className="px-5 pb-2 text-left text-meta font-medium text-ink-2 max-md:px-4">
                                {t.about.ecosystems[ecosystem]}
                            </caption>
                            <thead>
                                <tr>
                                    <th scope="col" className={table.th}>
                                        {t.about.name}
                                    </th>
                                    <th scope="col" className={table.th}>
                                        {t.about.version}
                                    </th>
                                    <th scope="col" className={table.th}>
                                        {t.about.licence}
                                    </th>
                                    <th scope="col" className={table.th}>
                                        <span className="sr-only">{t.about.licenceText}</span>
                                    </th>
                                </tr>
                            </thead>
                            <tbody>
                                {rows.map((item) => (
                                    <tr key={`${item.name}@${item.version}`} className={table.row}>
                                        <td className={cn(table.td, table.mono)}>{item.name}</td>
                                        <td className={cn(table.td, table.mono)}>{item.version}</td>
                                        <td className={table.td}>{item.licence ?? t.about.noLicence}</td>
                                        <td className={cn(table.td, 'text-right')}>
                                            <Button
                                                size="sm"
                                                tone="ghost"
                                                aria-label={t.about.textOf(item.name, item.version)}
                                                onClick={() => onOpen(item)}
                                            >
                                                {t.about.licenceText}
                                            </Button>
                                        </td>
                                    </tr>
                                ))}
                            </tbody>
                        </table>
                    </div>
                );
            })}
        </div>
    );
}

/** The side panel of one package: what it declares, where it lives, and each of its texts. */
function LicenceText({
    item,
    texts,
    onClose,
}: {
    item: ThirdPartyPackage;
    texts: Record<string, string>;
    onClose: () => void;
}) {
    const { t } = useI18n();
    return (
        <Dialog
            title={`${item.name} ${item.version}`}
            onClose={onClose}
            className="mt-0 mr-0 mb-0 ml-auto h-dvh max-h-dvh max-w-drawer rounded-none max-md:mt-0 max-md:w-full max-md:max-w-none"
            footer={<Button onClick={onClose}>{t.common.close}</Button>}
        >
            <dl className="flex flex-col gap-2">
                <div>
                    <dt className="text-meta text-ink-3">{t.about.licence}</dt>
                    <dd className="text-ink">{item.licence ?? t.about.noLicence}</dd>
                </div>
                {item.repository !== null && (
                    <div>
                        <dt className="text-meta text-ink-3">{t.about.repository}</dt>
                        <dd className="break-all">
                            <a
                                href={item.repository}
                                target="_blank"
                                rel="noreferrer"
                                className="text-accent-ink underline-offset-2 hover:underline"
                            >
                                {item.repository}
                            </a>
                        </dd>
                    </div>
                )}
            </dl>
            {item.fromSpdx && <Notice tone="neutral">{t.about.fromSpdx}</Notice>}
            {item.files.map((file) => (
                <section key={file.path} className="flex flex-col gap-1.5">
                    <h3 className="font-mono text-small text-ink">{file.path}</h3>
                    <pre className="rounded-control border border-line bg-surface-2 p-3 font-mono text-meta whitespace-pre-wrap break-words text-ink-2">
                        {texts[file.text]}
                    </pre>
                </section>
            ))}
        </Dialog>
    );
}
