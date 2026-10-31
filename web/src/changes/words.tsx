import {
    ArrowUpRight,
    CalendarClock,
    CircleQuestionMark,
    FilePen,
    FolderInput,
    Hourglass,
    LifeBuoy,
    Minus,
    PenLine,
    Play,
    Plus,
    PowerOff,
    RotateCw,
    TriangleAlert,
    type LucideIcon,
} from 'lucide-react';

import type { ChangeCounts } from '../api/generated/ChangeCounts';
import type { FileChangeKind } from '../api/generated/FileChangeKind';
import type { ResourceChangeKind } from '../api/generated/ResourceChangeKind';
import type { SnapshotSummary } from '../api/generated/SnapshotSummary';
import type { StartCause } from '../api/generated/StartCause';
import type { Messages } from '../i18n/en';

type Tone = 'neutral' | 'warning';

/**
 * Each change by its icon and word; files changed under the same version and a resource that did
 * not start warn.
 */
export const RESOURCE_CHIP = {
    added: { tone: 'neutral', icon: Plus },
    removed: { tone: 'neutral', icon: Minus },
    renamed: { tone: 'neutral', icon: PenLine },
    moved: { tone: 'neutral', icon: FolderInput },
    updated: { tone: 'neutral', icon: ArrowUpRight },
    sameVersion: { tone: 'warning', icon: TriangleAlert },
    checking: { tone: 'neutral', icon: Hourglass },
    unknown: { tone: 'neutral', icon: CircleQuestionMark },
    didNotStart: { tone: 'warning', icon: PowerOff },
    unchanged: { tone: 'neutral', icon: Minus },
} satisfies Record<ResourceChangeKind, { tone: Tone; icon: LucideIcon }>;

export const FILE_CHIP = {
    added: Plus,
    removed: Minus,
    renamed: PenLine,
    modified: FilePen,
    unchanged: Minus,
} satisfies Record<FileChangeKind, LucideIcon>;

export const CAUSE_ICON = {
    requested: Play,
    restart: RotateCw,
    scheduled: CalendarClock,
    recovery: LifeBuoy,
} satisfies Record<StartCause, LucideIcon>;

/** The kinds the resources table filters by, in the order of its chips. */
export const FILTERED = [
    'updated',
    'sameVersion',
    'added',
    'removed',
    'renamed',
    'moved',
    'didNotStart',
    'checking',
] as const satisfies readonly (keyof ChangeCounts & ResourceChangeKind)[];

/** Resources that changed one way or another; those still checking are neither changed nor not. */
export const changedResources = (counts: ChangeCounts) =>
    counts.added +
    counts.removed +
    counts.updated +
    counts.sameVersion +
    counts.renamed +
    counts.moved +
    counts.didNotStart;

/** A snapshot whose job still runs: its counts move as its files are read. */
export const isBusy = (snapshot: SnapshotSummary) =>
    snapshot.state === 'listing' || snapshot.state === 'hashing';

/** How a start came about, `Restarted by ada`, or that it is a check of the disk. */
export const startWords = (t: Messages, snapshot: SnapshotSummary) => {
    if (snapshot.kind === 'check') {
        return t.changes.check;
    }
    const cause = snapshot.cause === null ? t.changes.start : t.changes.causes[snapshot.cause];
    return snapshot.by === null ? cause : t.changes.startBy(cause, snapshot.by);
};

/** What changed against the start before, as a start's line says it: `3 resources changed · build`. */
export const countsWords = (t: Messages, snapshot: SnapshotSummary) => {
    if (snapshot.baseline) {
        return t.changes.baseline;
    }
    const counts = snapshot.counts;
    if (counts === null) {
        return t.changes.beingListed;
    }
    const resources = changedResources(counts);
    const parts = [
        resources > 0 ? t.changes.resourcesChanged(resources) : null,
        ...counts.environment.map((fact) => t.changes.facts[fact]),
    ].filter((part) => part !== null);
    return parts.length === 0 ? t.changes.noChange : parts.join(' · ');
};
