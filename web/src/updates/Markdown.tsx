import { Fragment, type ReactNode } from 'react';

import { table } from '../components/table';
import { parseBlocks, parseInline, type Block, type Inline } from '../lib/markdown';

/**
 * Release notes as GitHub writes them, drawn as React elements and never as HTML: headings,
 * lists, paragraphs, code, tables, emphasis and links. A link leads only to an http or https
 * address; a pull request, an issue or a comparison on GitHub reads as GitHub shows it, `#45`.
 */
export function Markdown({ source }: { source: string }) {
    return (
        <div className="flex flex-col gap-2.5 text-body text-ink-2">
            {parseBlocks(source).map((block) => (
                <BlockView key={block.key} block={block} />
            ))}
        </div>
    );
}

function BlockView({ block }: { block: Block }) {
    switch (block.kind) {
        case 'heading':
            return block.level <= 2 ? (
                <h3 className="pt-1 text-card font-medium text-ink">
                    <Inlines text={block.text} />
                </h3>
            ) : (
                <h4 className="pt-1 text-ui font-medium text-ink">
                    <Inlines text={block.text} />
                </h4>
            );
        case 'list': {
            const items = block.items.map((item) => (
                <li key={item.key}>
                    <Inlines text={item.text} />
                </li>
            ));
            return block.ordered ? (
                <ol className="flex list-decimal flex-col gap-1 pl-5">{items}</ol>
            ) : (
                <ul className="flex list-disc flex-col gap-1 pl-5">{items}</ul>
            );
        }
        case 'code':
            return (
                <pre className="rounded-box border border-line bg-surface-2 px-3 py-2 font-mono text-small break-words whitespace-pre-wrap text-ink">
                    {block.text}
                </pre>
            );
        case 'table':
            return (
                <div className="overflow-x-auto rounded-box border border-line">
                    <table className={table.table}>
                        <thead>
                            <tr>
                                {block.head.map((cell) => (
                                    <th key={cell.key} scope="col" className={table.th}>
                                        <Inlines text={cell.text} />
                                    </th>
                                ))}
                            </tr>
                        </thead>
                        <tbody>
                            {block.rows.map((row) => (
                                <tr key={row.key}>
                                    {row.cells.map((cell) => (
                                        <td key={cell.key} className={table.td}>
                                            <Inlines text={cell.text} />
                                        </td>
                                    ))}
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </div>
            );
        case 'quote':
            return (
                <blockquote className="rounded-box bg-surface-2 px-3 py-2">
                    <Inlines text={block.text} />
                </blockquote>
            );
        case 'rule':
            return <hr className="border-line" />;
        case 'paragraph':
            return (
                <p className="break-words">
                    <Inlines text={block.text} />
                </p>
            );
    }
}

function Inlines({ text }: { text: string }) {
    return <>{render(parseInline(text))}</>;
}

const render = (nodes: Inline[]): ReactNode =>
    nodes.map((node) => {
        switch (node.kind) {
            case 'text':
                return <Fragment key={node.key}>{node.text}</Fragment>;
            case 'code':
                return (
                    <code
                        key={node.key}
                        className="rounded-xs bg-surface-2 px-1 font-mono text-[0.92em] text-ink"
                    >
                        {node.text}
                    </code>
                );
            case 'strong':
                return (
                    <strong key={node.key} className="font-semibold text-ink">
                        {render(node.children)}
                    </strong>
                );
            case 'em':
                return <em key={node.key}>{render(node.children)}</em>;
            // Release notes stand alone: a link to a place beside them leads nowhere here.
            case 'local':
                return <Fragment key={node.key}>{render(node.children)}</Fragment>;
            case 'link':
                return (
                    <a
                        key={node.key}
                        href={node.href}
                        target="_blank"
                        rel="noreferrer noopener"
                        className="font-medium break-all text-ink underline-offset-2 hover:underline"
                    >
                        {render(node.children)}
                    </a>
                );
        }
    });
