import { Check, Link as LinkIcon } from 'lucide-react';
import { useMemo, type ReactNode } from 'react';

import type { ExampleFile } from '../api/generated/ExampleFile';
import type { IntegrationDocs } from '../api/generated/IntegrationDocs';
import { buttonVariants } from '../components/Button';
import { Chip } from '../components/Chip';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { useCopy } from '../lib/useCopy';
import { CodeBlock } from './CodeBlock';
import { filesAnchor, handlerOf, type DocEntry, type DocModel, type DocSection } from './model';
import { SectionNote } from './Notes';
import { DocTable, Prose, ReadingContext, Text } from './Prose';

/**
 * A card a link leads to stops under what stays at the top of the screen: nothing, the phone's bar,
 * or that bar and the contents button.
 */
export const ANCHORED = 'scroll-mt-3 max-xl:scroll-mt-[4.5rem] max-md:scroll-mt-[8.25rem]';

const body = 'flex flex-col gap-3 px-5 pt-4 pb-5 text-body text-ink-2 max-md:px-4 max-md:pb-4';

/** The folder a file of the contract's own folder lies in: `observer_example`. */
export const folderOf = (file: ExampleFile) => file.name.slice(0, Math.max(file.name.lastIndexOf('/'), 0));

/**
 * A card's head: its title, what says its kind, and the button that copies its address, so that a
 * section can be sent to someone. The button keeps its corner when the title folds.
 */
function Head({ id, title, children }: { id: string; title: string; children: ReactNode }) {
    const { t } = useI18n();
    const { state, copy } = useCopy();
    const address = `${location.origin}/docs#${id}`;
    return (
        <>
            <div className="card-head flex-nowrap items-start">
                <div className="flex min-h-8 min-w-0 flex-1 flex-wrap items-center gap-x-3 gap-y-1 max-md:min-h-11">
                    {children}
                </div>
                <button
                    type="button"
                    className={cn(buttonVariants({ tone: 'ghost', size: 'icon-sm' }), 'flex-none text-ink-3')}
                    aria-label={t.docs.copyLink(title)}
                    title={t.docs.copyLink(title)}
                    onClick={() => void copy(address)}
                >
                    {state === 'copied' ? (
                        <Check size={14} strokeWidth={1.75} aria-hidden="true" />
                    ) : (
                        <LinkIcon size={14} strokeWidth={1.5} aria-hidden="true" />
                    )}
                </button>
            </div>
            <output
                className={cn(
                    'block border-b border-line px-5 py-2 text-meta text-ink-3 [overflow-wrap:anywhere] max-md:px-4',
                    state !== 'blocked' && 'sr-only',
                )}
            >
                {state === 'copied'
                    ? t.docs.linkCopied
                    : state === 'blocked'
                      ? t.common.copyBlockedValue(address)
                      : ''}
            </output>
        </>
    );
}

/** The files a section's link leads to, each folded under its name. */
function Files({ files }: { files: ExampleFile[] }) {
    const folders = [...new Set(files.map(folderOf))];
    return folders.map((folder) => (
        <div key={folder} id={filesAnchor(folder)} className={cn('flex flex-col gap-2', ANCHORED)}>
            {files
                .filter((file) => folderOf(file) === folder)
                .map((file) => (
                    <CodeBlock
                        key={file.name}
                        code={file.text.trimEnd()}
                        label={file.name}
                        name={file.name}
                        folded
                    />
                ))}
        </div>
    ));
}

/** A section of the contract: its text, the files it links to, and what the panel adds to it. */
export function SectionCard({
    section,
    model,
    docs,
}: {
    section: DocSection;
    model: DocModel;
    docs: IntegrationDocs;
}) {
    const { t } = useI18n();
    const titleId = `${section.id}-title`;
    const reading = useMemo(
        () => ({ anchors: model.anchors, paths: new Set(docs.example.map(folderOf)), own: section.title }),
        [model.anchors, docs.example, section.title],
    );
    return (
        <ReadingContext value={reading}>
            <section id={section.id} aria-labelledby={titleId} className={cn('card', ANCHORED)}>
                <Head id={section.id} title={section.title}>
                    <h2 id={titleId} className="min-w-0 text-card text-ink">
                        {section.title}
                    </h2>
                </Head>
                {(section.blocks.length > 0 || section.files.length > 0) && (
                    <div className={body}>
                        {section.exports.length > 0 && (
                            // As the bridge's script declares them: the parameters are its own names.
                            <p className="flex flex-wrap items-center gap-x-2 gap-y-1.5 text-meta text-ink-3">
                                {t.docs.exports}
                                {section.exports.map(({ name, parameters }) => (
                                    <code key={name} className="code-inline">
                                        {name}({parameters.join(', ')})
                                    </code>
                                ))}
                            </p>
                        )}
                        <Prose blocks={section.blocks} codeName={t.docs.codeOf(section.title)} />
                        <Files files={section.files} />
                    </div>
                )}
                <SectionNote id={section.id} server={docs.server} />
            </section>
        </ReadingContext>
    );
}

/**
 * An event, as a rule: when it comes, what it carries, what the contract adds, and a handler to
 * copy. Another third-level heading of the contract is drawn the same way, without the handler.
 */
export function EntryCard({
    entry,
    model,
    docs,
}: {
    entry: DocEntry;
    model: DocModel;
    docs: IntegrationDocs;
}) {
    const { t } = useI18n();
    const titleId = `${entry.id}-title`;
    const reading = useMemo(
        () => ({ anchors: model.anchors, paths: new Set(docs.example.map(folderOf)), own: entry.title }),
        [model.anchors, docs.example, entry.title],
    );
    const handler = useMemo(
        () => handlerOf(entry, docs.example.map((file) => file.text).join('\n')),
        [entry, docs.example],
    );
    // What comes before the fields introduces them; the rest follows them.
    const at = entry.blocks.findIndex((block) => block.kind === 'table');
    const fields = entry.blocks[at];
    const intro = at < 0 ? [] : entry.blocks.slice(0, at);
    const rest = at < 0 ? entry.blocks : entry.blocks.slice(at + 1);
    return (
        <ReadingContext value={reading}>
            <section id={entry.id} aria-labelledby={titleId} className={cn('card', ANCHORED)}>
                <Head id={entry.id} title={entry.title}>
                    <h3
                        id={titleId}
                        className={cn(
                            'min-w-0 text-ink [overflow-wrap:anywhere]',
                            entry.event
                                ? 'font-mono text-nav font-medium [font-variant-ligatures:none]'
                                : 'text-card',
                        )}
                    >
                        {entry.title}
                    </h3>
                    {entry.event && <Chip>{t.docs.event}</Chip>}
                </Head>
                {(entry.when !== null || intro.length > 0) && (
                    <div className="flex flex-col gap-2 px-5 py-3.5 text-body text-ink-2 max-md:px-4">
                        {entry.when !== null && (
                            <p className="max-w-[68ch] text-pretty">
                                <Text text={entry.when} />
                            </p>
                        )}
                        <Prose blocks={intro} codeName={t.docs.codeOf(entry.title)} />
                    </div>
                )}
                {fields?.kind === 'table' && <DocTable block={fields} />}
                {(rest.length > 0 || entry.event) && (
                    <div className={body}>
                        <Prose blocks={rest} codeName={t.docs.codeOf(entry.title)} />
                        {entry.event && (
                            <CodeBlock
                                code={handler}
                                label={t.docs.lua}
                                name={t.docs.handlerOf(entry.title)}
                            />
                        )}
                    </div>
                )}
                <SectionNote id={entry.id} server={docs.server} />
            </section>
        </ReadingContext>
    );
}
