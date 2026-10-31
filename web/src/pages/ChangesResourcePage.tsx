import { useQuery } from '@tanstack/react-query';
import { Link, useSearch } from '@tanstack/react-router';
import { ArrowLeft } from 'lucide-react';
import type { ReactNode } from 'react';

import { comparisonQuery, resourceQuery } from '../api/changes';
import { buttonVariants } from '../components/Button';
import { Notice } from '../components/Notice';
import { ResourceView } from '../changes/ResourceView';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';

/** One resource between two snapshots: `/changes/resource?from=&to=&path=`, its search checked by the route. */
export function ChangesResourcePage() {
    const { t } = useI18n();
    const { from, to, path = '' } = useSearch({ strict: false });
    // The page links with both ids; a link that leaves one to the API's defaults reads which they are.
    const pinned = typeof from === 'number' && typeof to === 'number';
    const comparison = useQuery({ ...comparisonQuery(from, to), enabled: !pinned });
    const detail = useQuery({ ...resourceQuery(from, to, path), enabled: path !== '' });
    const ids = pinned
        ? { from, to }
        : comparison.data === undefined
          ? null
          : { from: comparison.data.from.id, to: comparison.data.to.id };
    const name = detail.data?.change.name ?? path.slice(path.lastIndexOf('/') + 1);

    let body: ReactNode;
    const failure = detail.error ?? comparison.error;
    if (failure !== null) {
        body = <Notice tone="danger">{errorMessage(t, failure)}</Notice>;
    } else if (detail.data === undefined || ids === null) {
        body = (
            <div className="flex flex-col gap-3" aria-busy="true" aria-label={t.changes.resource.loading}>
                <div className="skeleton h-32 w-full rounded-card" />
                <div className="skeleton h-80 w-full rounded-card" />
            </div>
        );
    } else {
        body = <ResourceView detail={detail.data} from={ids.from} to={ids.to} />;
    }

    return (
        <>
            <div className="flex flex-col items-start gap-2">
                <Link
                    to="/changes"
                    search={{ from, to }}
                    className={cn(buttonVariants({ tone: 'ghost', size: 'sm' }), '-ml-2.5')}
                >
                    <ArrowLeft size={14} strokeWidth={1.5} aria-hidden="true" />
                    {t.changes.resource.back}
                </Link>
                <div className="page-head w-full">
                    <h1 className="page-title [overflow-wrap:anywhere]">
                        {name} <span className="gloss">{t.changes.title}</span>
                    </h1>
                    <p className="text-meta text-ink-3">{t.changes.timeNote}</p>
                </div>
            </div>
            {path === '' ? <Notice tone="danger">{t.errors.not_found}</Notice> : body}
        </>
    );
}
