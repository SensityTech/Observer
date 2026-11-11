import type { EntityItem } from '../api/generated/EntityItem';
import type { EntityKind } from '../api/generated/EntityKind';
import type { Messages } from '../i18n/en';
import { cn } from '../lib/cn';
import { modelHash } from './model';

/** The mark of each kind, in the list as on the map: a shape each, since a colour never says it alone. */
const MARKS: Record<EntityKind, string> = {
    vehicle: 'size-2.5 rounded-full bg-term-cyan',
    ped: 'size-[9px] rotate-45 bg-term-magenta',
    object: 'size-2 rounded-[1.5px] bg-ink-3',
};

export function EntityMark({ kind, className }: { kind: EntityKind; className?: string }) {
    return <span aria-hidden="true" className={cn('inline-block flex-none', MARKS[kind], className)} />;
}

/** What an entity is called: a vehicle by its class, the others by their kind. */
export const kindWord = (t: Messages, item: Pick<EntityItem, 'kind' | 'class'>) =>
    item.kind === 'vehicle' && item.class !== null
        ? t.entities.classes[item.class]
        : t.entities.kindNames[item.kind];

/** An entity said whole, for the controls that act on it. */
export const entityName = (t: Messages, item: Pick<EntityItem, 'kind' | 'class' | 'model' | 'netId'>) =>
    t.entities.named(kindWord(t, item), modelHash(item.model), item.netId);
