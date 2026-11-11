import { Check, ChevronRight, Copy } from 'lucide-react';
import { useId, useMemo, useRef, useState } from 'react';

import { Button } from '../components/Button';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { useCopy } from '../lib/useCopy';
import { luaTokens, type LuaKind } from './lua';

const INK: Record<LuaKind, string> = {
    comment: 'text-term-dim',
    string: 'text-term-green',
    keyword: 'text-term-magenta',
    number: 'text-term-yellow',
    plain: '',
};

/**
 * Code to copy, on the console's ground: a line never folds, the block scrolls. `name` says what the
 * code is, for its copy button. A `folded` block is a file: it opens on its name. Lua is coloured
 * unless `lua` says the code is something else.
 */
export function CodeBlock({
    code,
    label,
    name,
    folded = false,
    lua = true,
}: {
    code: string;
    label: string;
    name: string;
    folded?: boolean;
    lua?: boolean;
}) {
    const { t } = useI18n();
    const id = useId();
    const { state, copy } = useCopy();
    const [open, setOpen] = useState(!folded);
    const pre = useRef<HTMLPreElement>(null);
    const tokens = useMemo(
        () => (lua ? luaTokens(code) : [{ at: 0, text: code, kind: 'plain' as const }]),
        [code, lua],
    );

    const copyCode = async () => {
        const text = pre.current;
        // Where the browser refuses, the code is selected instead, so the keyboard can copy it.
        if (!(await copy(code)) && text !== null) {
            const range = document.createRange();
            range.selectNodeContents(text);
            const selection = getSelection();
            selection?.removeAllRanges();
            selection?.addRange(range);
        }
    };

    return (
        <div className="min-w-0 overflow-hidden rounded-box border border-line bg-console">
            <div className="flex min-h-9 items-center gap-2 pr-1.5 text-meta text-ink-3">
                {folded ? (
                    <button
                        type="button"
                        aria-expanded={open}
                        aria-controls={id}
                        className="flex min-h-9 min-w-0 flex-1 items-center gap-2 rounded-xs pl-2.5 text-left hover:text-ink-2 max-md:min-h-11"
                        onClick={() => setOpen(!open)}
                    >
                        <ChevronRight
                            size={14}
                            strokeWidth={1.75}
                            className={cn('flex-none transition-transform', open && 'rotate-90')}
                            aria-hidden="true"
                        />
                        <span className="min-w-0 truncate font-mono text-small text-ink [font-variant-ligatures:none]">
                            {label}
                        </span>
                        <span className="flex-none">{t.docs.lines(code.trimEnd().split('\n').length)}</span>
                    </button>
                ) : (
                    <span className="flex-1 pl-3">{label}</span>
                )}
                <Button
                    size="sm"
                    tone="ghost"
                    icon={state === 'copied' ? Check : Copy}
                    className="h-7 flex-none px-2 text-meta"
                    aria-label={t.docs.copyNamed(name)}
                    onClick={() => {
                        setOpen(true);
                        void copyCode();
                    }}
                >
                    {state === 'copied' ? t.common.copied : t.common.copy}
                </Button>
            </div>
            <pre
                ref={pre}
                id={id}
                hidden={!open}
                // oxlint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- the keyboard scrolls the code once it can focus it
                tabIndex={0}
                className="max-h-[32rem] overflow-auto border-t border-line px-3.5 py-3 font-mono text-small leading-[1.6] text-term-fg scrollbar-thin [font-variant-ligatures:none] focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent"
            >
                <code>
                    {tokens.map((token) =>
                        token.kind === 'plain' ? (
                            token.text
                        ) : (
                            <span key={token.at} className={INK[token.kind]}>
                                {token.text}
                            </span>
                        ),
                    )}
                </code>
            </pre>
            <output
                className={cn(
                    'block border-t border-line px-3 py-2 text-small text-ink-3',
                    state !== 'blocked' && 'sr-only',
                )}
            >
                {state === 'copied' ? t.common.copied : state === 'blocked' ? t.common.copyBlocked : ''}
            </output>
        </div>
    );
}
