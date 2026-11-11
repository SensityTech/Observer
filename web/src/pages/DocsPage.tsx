import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useLocation } from '@tanstack/react-router';
import { Search } from 'lucide-react';
import { Fragment, useEffect, useMemo, useRef } from 'react';

import { integrationDocsQuery } from '../api/docs';
import type { IntegrationDocs } from '../api/generated/IntegrationDocs';
import { buttonVariants } from '../components/Button';
import { Notice } from '../components/Notice';
import { EntryCard, SectionCard, folderOf } from '../docs/Cards';
import { buildDocs } from '../docs/model';
import { Prose, ReadingContext } from '../docs/Prose';
import { THIS_SERVER, ThisServer } from '../docs/ThisServer';
import { Toc, type TocGroup } from '../docs/Toc';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { useServerStatus } from '../server/context';
import { useOpenPalette } from '../shell/palette';

/**
 * What Observer gives the server's resources, for whoever writes them: the integration contract as
 * this build ships it, drawn section by section, with what this server does of it and where the
 * panel shows its effect. The text is the binary's own: this page writes none of it.
 */
export function DocsPage() {
    const { t } = useI18n();
    const docs = useQuery(integrationDocsQuery);
    const openPalette = useOpenPalette();
    useServerSide();

    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            <div className="page-head items-center">
                <h1 className="page-title">
                    {t.docs.title} <span className="gloss">{t.docs.gloss}</span>
                </h1>
                <button
                    type="button"
                    className={cn(
                        buttonVariants({ tone: 'secondary', size: 'sm' }),
                        'text-ink-2 max-md:w-full max-md:justify-start',
                    )}
                    aria-keyshortcuts="Control+K"
                    onClick={openPalette}
                >
                    <Search size={14} strokeWidth={1.5} aria-hidden="true" />
                    {t.docs.find}
                    <kbd className="kbd max-md:hidden">Ctrl K</kbd>
                </button>
            </div>
            {/* What was read stays on the page when reading it again fails. */}
            {docs.data !== undefined ? (
                <Documentation docs={docs.data} />
            ) : docs.isError ? (
                <Notice tone="danger">
                    {t.docs.failed} {errorMessage(t, docs.error)}
                </Notice>
            ) : (
                <div className="flex flex-col gap-3" aria-busy="true">
                    <span className="sr-only">{t.docs.loading}</span>
                    <div className="skeleton h-4 w-2/3" />
                    <div className="skeleton h-40 w-full rounded-card" />
                    <div className="skeleton h-64 w-full rounded-card" />
                </div>
            )}
        </div>
    );
}

/** The server's side of the documentation changes with each start and stop: it is read again then. */
const useServerSide = () => {
    const client = useQueryClient();
    const { status } = useServerStatus();
    const run = status === null ? null : `${status.generation} ${status.state}`;
    const last = useRef(run);
    useEffect(() => {
        if (run === null) {
            return;
        }
        if (last.current !== null && last.current !== run) {
            void client.invalidateQueries({ queryKey: integrationDocsQuery.queryKey });
        }
        last.current = run;
    }, [client, run]);
};

/** `observer:shuttingDown` is `shuttingDown` under its section, where the prefix says nothing more. */
const short = (event: string) => event.slice(event.indexOf(':') + 1);

function Documentation({ docs }: { docs: IntegrationDocs }) {
    const { t } = useI18n();
    const { contract, events, exports, example, server } = docs;
    const situated = server !== null;
    const model = useMemo(
        () => buildDocs({ contract, events, exports, example }),
        [contract, events, exports, example],
    );
    const groups = useMemo<TocGroup[]>(
        () => [
            ...(situated
                ? [
                      {
                          label: t.docs.toc,
                          items: [
                              { key: THIS_SERVER, id: THIS_SERVER, label: t.docs.server.title, sub: false },
                          ],
                      },
                  ]
                : []),
            {
                label: t.docs.contract,
                items: model.sections.flatMap((section) => [
                    { key: section.id, id: section.id, label: section.title, sub: false },
                    ...section.exports.map(({ name }) => ({
                        key: `${section.id} ${name}`,
                        id: section.id,
                        label: name,
                        sub: true,
                    })),
                    ...section.entries.map((entry) => ({
                        key: entry.id,
                        id: entry.id,
                        label: entry.event ? short(entry.title) : entry.title,
                        sub: true,
                    })),
                ]),
            },
        ],
        [model, t, situated],
    );
    const reading = useMemo(
        () => ({ anchors: model.anchors, paths: new Set(example.map(folderOf)), own: '' }),
        [model.anchors, example],
    );

    // A link to a section opens on it, once the page has drawn what it leads to.
    const hash = useLocation({ select: (location) => location.hash });
    useEffect(() => {
        if (hash !== '') {
            document.getElementById(hash)?.scrollIntoView();
        }
    }, [hash]);

    return (
        <>
            <div className="flex max-w-desc flex-col gap-2 text-body text-pretty text-ink-2">
                <ReadingContext value={reading}>
                    <Prose blocks={model.lead} codeName={t.docs.title} />
                </ReadingContext>
            </div>
            <div className="flex items-start gap-6 max-md:flex-col max-md:items-stretch max-md:gap-3">
                <Toc groups={groups} />
                <div className="flex min-w-0 flex-[1_1_0] flex-col gap-3 max-md:flex-none">
                    {server !== null && <ThisServer contractVersion={docs.contractVersion} server={server} />}
                    {model.sections.map((section) => (
                        <Fragment key={section.id}>
                            <SectionCard section={section} model={model} docs={docs} />
                            {section.entries.map((entry) => (
                                <EntryCard key={entry.id} entry={entry} model={model} docs={docs} />
                            ))}
                        </Fragment>
                    ))}
                </div>
            </div>
        </>
    );
}
