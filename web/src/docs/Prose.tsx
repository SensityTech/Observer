import { Fragment, createContext, use, type ReactNode } from 'react';

import { table } from '../components/table';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { parseInline, plainText, type Block, type Inline } from '../lib/markdown';
import { Anchor } from './Anchor';
import { CodeBlock } from './CodeBlock';
import { filesAnchor } from './model';

/** What the text of the documentation is read against. */
export type Reading = {
    /** Where a name written in code leads on the page. */
    anchors: Map<string, string>;
    /** The paths whose files the page shows. */
    paths: Set<string>;
    /** The title of the section or entry the text belongs to: its own name is not a link to itself. */
    own: string;
};

export const ReadingContext = createContext<Reading>({ anchors: new Map(), paths: new Set(), own: '' });

const link =
    'font-medium text-ink underline decoration-line-strong underline-offset-3 hover:decoration-ink-3';

/** A long name folds after a colon before it folds anywhere else. */
const foldable = (text: string) => {
    let at = 0;
    return text.split(/(?<=:)(?=\w)/).map((piece) => {
        const start = at;
        at += piece.length;
        return (
            <Fragment key={start}>
                {start > 0 && <wbr />}
                {piece}
            </Fragment>
        );
    });
};

function Nodes({ nodes, folding }: { nodes: Inline[]; folding: boolean }): ReactNode {
    const { anchors, paths, own } = use(ReadingContext);
    return nodes.map((node) => {
        switch (node.kind) {
            case 'text':
                return <Fragment key={node.key}>{node.text}</Fragment>;
            case 'code': {
                const code = (
                    <code
                        className={cn(
                            'code-inline',
                            folding && 'box-decoration-clone whitespace-normal [overflow-wrap:anywhere]',
                        )}
                    >
                        {folding ? foldable(node.text) : node.text}
                    </code>
                );
                const anchor = node.text === own ? undefined : anchors.get(node.text);
                return anchor === undefined ? (
                    <Fragment key={node.key}>{code}</Fragment>
                ) : (
                    <Anchor
                        key={node.key}
                        id={anchor}
                        className="rounded-xs underline decoration-line-strong underline-offset-4 hover:decoration-ink-3"
                    >
                        {code}
                    </Anchor>
                );
            }
            case 'strong':
                return (
                    <strong key={node.key} className="font-semibold text-ink">
                        <Nodes nodes={node.children} folding={folding} />
                    </strong>
                );
            case 'em':
                return (
                    <em key={node.key}>
                        <Nodes nodes={node.children} folding={folding} />
                    </em>
                );
            case 'link':
                return (
                    <a
                        key={node.key}
                        href={node.href}
                        target="_blank"
                        rel="noreferrer noopener"
                        className={link}
                    >
                        <Nodes nodes={node.children} folding={folding} />
                    </a>
                );
            case 'local': {
                const hash = node.target.startsWith('#')
                    ? node.target.slice(1)
                    : paths.has(node.target)
                      ? filesAnchor(node.target)
                      : null;
                // A path the page shows nothing of leads nowhere: its words stay.
                return hash === null ? (
                    <Fragment key={node.key}>
                        <Nodes nodes={node.children} folding={folding} />
                    </Fragment>
                ) : (
                    <Anchor key={node.key} id={hash} className={link}>
                        <Nodes nodes={node.children} folding={folding} />
                    </Anchor>
                );
            }
        }
    });
}

/** A line of the contract: its code, its emphasis and its links, drawn as elements, never as HTML. */
export function Text({ text, folding = false }: { text: string; folding?: boolean }) {
    return <Nodes nodes={parseInline(text)} folding={folding} />;
}

/** Longer than this, a table's second column no longer rides beside the first on a phone. */
const BESIDE_CHARS = 24;
const short = (text: string) => plainText(parseInline(text)).length <= BESIDE_CHARS;

/**
 * A table of the contract, to the card's edges. On a phone each row is a block: its first cell is
 * its title, a short second one rides beside it, and the cells after them are named by their column
 * once there are several.
 */
export function DocTable({ block }: { block: Extract<Block, { kind: 'table' }> }) {
    const beside = block.head.length >= 3 && block.rows.every((row) => short(row.cells[1]?.text ?? ''));
    const named = block.head.length - (beside ? 2 : 1) > 1;
    return (
        <div className="overflow-x-auto border-y border-line">
            <table className={cn(table.table, 'max-md:block')}>
                <thead className="max-md:sr-only">
                    <tr>
                        {block.head.map((cell) => (
                            <th key={cell.key} scope="col" className={table.th}>
                                <Text text={cell.text} />
                            </th>
                        ))}
                    </tr>
                </thead>
                <tbody className="max-md:block">
                    {block.rows.map((row) => (
                        <tr
                            key={row.key}
                            className={cn(
                                'max-md:block max-md:border-b max-md:border-line max-md:px-4 max-md:py-2.5 max-md:last:border-b-0 [&:last-child>td]:border-b-0',
                                beside &&
                                    'max-md:grid max-md:grid-cols-[auto_1fr] max-md:items-baseline max-md:gap-x-2.5',
                            )}
                        >
                            {row.cells.map((cell, index) => {
                                const key = index === 0 && /^`[^`]+`$/.test(cell.text);
                                const riding = beside && index === 1;
                                const label =
                                    named && index >= (beside ? 2 : 1) ? block.head[index]?.text : undefined;
                                return (
                                    <td
                                        key={cell.key}
                                        className={cn(
                                            table.td,
                                            'h-auto py-2.5 align-top max-md:block max-md:border-b-0 max-md:p-0 max-md:first:pl-0 max-md:last:pr-0',
                                            key &&
                                                'font-mono text-small whitespace-nowrap text-ink [font-variant-ligatures:none] max-md:whitespace-normal max-md:[overflow-wrap:anywhere] [&_code]:border-0 [&_code]:bg-transparent [&_code]:p-0',
                                            riding && 'whitespace-nowrap text-ink-3',
                                            index > 0 && !riding && 'max-md:mt-1',
                                            beside && index >= 2 && 'max-md:col-span-2',
                                        )}
                                    >
                                        {label !== undefined && cell.text !== '' && (
                                            <span className="block text-caption text-ink-3 md:hidden">
                                                <Text text={label} />
                                            </span>
                                        )}
                                        <Text text={cell.text} folding={!key} />
                                    </td>
                                );
                            })}
                        </tr>
                    ))}
                </tbody>
            </table>
        </div>
    );
}

/**
 * Blocks of the contract inside a card's body: paragraphs, lists, code to copy and tables that run
 * to the card's edges. `codeName` says what a block of code is, for its copy button.
 */
export function Prose({ blocks, codeName }: { blocks: Block[]; codeName: string }) {
    const { t } = useI18n();
    return blocks.map((block) => {
        switch (block.kind) {
            case 'paragraph':
                return (
                    <p key={block.key} className="max-w-[68ch] text-pretty">
                        <Text text={block.text} />
                    </p>
                );
            case 'list': {
                const items = block.items.map((item) => (
                    <li key={item.key}>
                        <Text text={item.text} />
                    </li>
                ));
                const className = 'flex max-w-[68ch] flex-col gap-1.5 pl-5 text-pretty marker:text-ink-3';
                return block.ordered ? (
                    <ol key={block.key} className={cn(className, 'list-decimal')}>
                        {items}
                    </ol>
                ) : (
                    <ul key={block.key} className={cn(className, 'list-disc')}>
                        {items}
                    </ul>
                );
            }
            case 'code':
                return (
                    <CodeBlock
                        key={block.key}
                        code={block.text}
                        label={block.lang === 'lua' ? t.docs.lua : block.lang}
                        name={codeName}
                        lua={block.lang === 'lua'}
                    />
                );
            case 'table':
                return (
                    // The last thing in a card runs to its foot.
                    <div
                        key={block.key}
                        className="-mx-5 my-1 last:-mb-5 max-md:-mx-4 max-md:last:-mb-4 [&:last-child>div]:border-b-0"
                    >
                        <DocTable block={block} />
                    </div>
                );
            case 'quote':
                return (
                    <blockquote key={block.key} className="max-w-[68ch] rounded-box bg-surface-2 px-3 py-2">
                        <Text text={block.text} />
                    </blockquote>
                );
            case 'heading':
                return (
                    <h4 key={block.key} className="pt-1 text-ui font-medium text-ink">
                        <Text text={block.text} />
                    </h4>
                );
            case 'rule':
                return <hr key={block.key} className="border-line" />;
        }
    });
}
