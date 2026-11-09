import { useQuery } from '@tanstack/react-query';
import { ChevronDown } from 'lucide-react';
import { useId } from 'react';

import {
    cfgConvarsQuery,
    cfgLastSettings,
    convarCatalogQuery,
    sameName,
    suggestedValues,
} from '../api/convars';
import type { CfgConvar } from '../api/generated/CfgConvar';
import type { CfgConvars } from '../api/generated/CfgConvars';
import type { Convar } from '../api/generated/Convar';
import type { ConvarCatalog } from '../api/generated/ConvarCatalog';
import { Notice } from '../components/Notice';
import { useI18n } from '../i18n/I18nProvider';

/** The catalog and what the saved cfg sets; either is null while unknown, and the form works without. */
export function useConvars() {
    const catalog = useQuery(convarCatalogQuery);
    const cfg = useQuery(cfgConvarsQuery);
    return { catalog: catalog.data ?? null, cfg: cfg.data ?? null };
}

/** The catalog's names, which a name field suggests as it is typed. */
export function ConvarNames({ id, catalog }: { id: string; catalog: ConvarCatalog }) {
    const { t } = useI18n();
    const words = t.settings.launch.convars;
    return (
        <datalist id={id}>
            {catalog.convars
                .filter((convar) => convar.declared)
                .map((convar) => (
                    <option key={convar.name} value={convar.name}>
                        {convar.kind === null ? convar.name : words.kinds[convar.kind]}
                    </option>
                ))}
        </datalist>
    );
}

/** The values a value field suggests for a convar whose type names them. */
export function ConvarValues({ id, convar }: { id: string; convar: Convar }) {
    return (
        <datalist id={id}>
            {suggestedValues(convar).map((value) => (
                <option key={value} value={value}>
                    {value}
                </option>
            ))}
        </datalist>
    );
}

/** What the sources say of a convar: its type, default, flags and comment. */
export function ConvarFacts({ convar, build }: { convar: Convar | undefined; build: number | null }) {
    const { t } = useI18n();
    const words = t.settings.launch.convars;
    if (convar === undefined) {
        return build === null ? null : (
            <p className="text-small text-ink-3">{words.unknown(String(build))}</p>
        );
    }
    if (!convar.declared) {
        return <p className="text-small text-ink-3">{words.undeclared}</p>;
    }
    const facts: string[] = [];
    const values = suggestedValues(convar);
    if (convar.kind === 'enum' && values.length > 0) {
        facts.push(words.values(values.join(', ')) + (convar.open ? ` ${words.orAnyNumber}` : ''));
    } else if (convar.kind !== null) {
        facts.push(words.kinds[convar.kind]);
    }
    if (convar.default !== null) {
        facts.push(words.defaultValue(convar.default));
    } else if (convar.defaultExpression !== null) {
        facts.push(words.defaultInCode);
    }
    for (const flag of convar.flags ?? []) {
        facts.push(words.flags[flag]);
    }
    if (convar.platform !== null) {
        facts.push(words[convar.platform]);
    }
    return (
        <div className="flex flex-col gap-1 text-small text-ink-3">
            <p>{facts.join(' · ')}</p>
            {convar.description !== null && (
                <p className="break-words">{words.comment(convar.description.replaceAll('\n', ' '))}</p>
            )}
        </div>
    );
}

/**
 * The cfg's own setting of a convar the command line sets too, which is the one that applies:
 * FXServer applies every `+set` first and runs the cfg after.
 */
export function CfgConflict({ set, value }: { set: CfgConvar | undefined; value: string }) {
    const { t } = useI18n();
    const words = t.settings.launch.convars;
    if (set === undefined) {
        return null;
    }
    if (set.value === null) {
        return (
            <Notice tone="warning" live={false}>
                {words.cfgSetsSecret(set.file, set.line)}
            </Notice>
        );
    }
    if (set.value === value.trim()) {
        return (
            <p className="text-small text-ink-3" role="note">
                {words.cfgSetsSame(set.file, set.line)}
            </p>
        );
    }
    return (
        <Notice tone="warning" live={false}>
            {words.cfgSets(set.file, set.line, set.value === '' ? '""' : set.value)}
        </Notice>
    );
}

/** What the cfg sets that the command line does not: folded, as it changes nothing here. */
export function CfgOnly({ cfg, names }: { cfg: CfgConvars | null; names: string[] }) {
    const { t } = useI18n();
    const words = t.settings.launch.convars;
    const id = useId();
    if (cfg === null) {
        return null;
    }
    if (cfg.problem !== null) {
        return <p className="text-small text-ink-3">{words.unreadable(cfg.cfgFile)}</p>;
    }
    const others = cfgLastSettings(cfg).filter((set) => !names.some((name) => sameName(name, set.name)));
    return (
        <>
            {others.length > 0 && (
                <details className="group rounded-box border border-line">
                    <summary
                        id={id}
                        className="flex min-h-10 cursor-pointer list-none items-center gap-2 px-3 py-2 text-small font-medium text-ink-2 hover:bg-hover focus-visible:outline-2 focus-visible:outline-accent max-md:min-h-11 [&::-webkit-details-marker]:hidden"
                    >
                        {words.inCfg(others.length)}
                        <ChevronDown
                            size={14}
                            strokeWidth={1.5}
                            aria-hidden="true"
                            className="ml-auto text-ink-3 transition-transform group-open:rotate-180"
                        />
                    </summary>
                    <ul aria-labelledby={id} className="flex flex-col border-t border-line">
                        {others.map((set) => (
                            <li
                                key={set.name.toLowerCase()}
                                className="grid grid-cols-[minmax(0,1fr)_auto] gap-x-3 gap-y-0.5 px-3 py-2 text-small not-last:border-b not-last:border-line md:grid-cols-[minmax(0,16rem)_minmax(0,1fr)_auto]"
                            >
                                <span className="truncate font-mono text-ink">{set.name}</span>
                                <span className="text-ink-3 md:order-last">
                                    {words.place(set.file, set.line)}
                                </span>
                                <span
                                    className={
                                        set.value === null
                                            ? 'col-span-2 text-ink-3 italic md:col-span-1'
                                            : 'col-span-2 font-mono break-all text-ink-2 md:col-span-1'
                                    }
                                >
                                    {set.value ?? words.secret}
                                </span>
                            </li>
                        ))}
                    </ul>
                </details>
            )}
            {!cfg.complete && <p className="text-small text-ink-3">{words.incomplete}</p>}
        </>
    );
}
