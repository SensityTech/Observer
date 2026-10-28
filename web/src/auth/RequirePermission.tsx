import { Link } from '@tanstack/react-router';
import { ShieldAlert } from 'lucide-react';
import type { ReactNode } from 'react';

import { Permission } from '../api/generated/Permission';
import { buttonVariants } from '../components/Button';
import { useI18n } from '../i18n/I18nProvider';
import { allows, useMe } from './me';
import { keyOf, type PermissionName } from './permissions';

/**
 * Shows a page only to a user who holds its permission. The API refuses the data anyway; this
 * says why in place of an error.
 */
export function RequirePermission({ name, children }: { name: PermissionName; children: ReactNode }) {
    const { t } = useI18n();
    const me = useMe();

    if (allows(me, Permission[name])) {
        return children;
    }
    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            <div className="page-head">
                <h1 className="page-title">
                    {t.forbidden.title} <span className="gloss">{t.forbidden.gloss}</span>
                </h1>
            </div>
            <section className="flex flex-col items-center gap-3 rounded-card border border-dashed border-line-strong bg-bg-subtle px-6 py-14 text-center dot-grid">
                <span className="grid size-12 place-items-center rounded-box border border-line-strong bg-surface text-ink-2 shadow-sm">
                    <ShieldAlert size={20} strokeWidth={1.5} aria-hidden="true" />
                </span>
                <p className="max-w-empty text-body text-ink-3">
                    {t.forbidden.body(t.permissions[keyOf(name)].name)}
                </p>
                <Link to="/account" className={buttonVariants({ tone: 'ink', size: 'sm' })}>
                    {t.forbidden.back}
                </Link>
            </section>
        </div>
    );
}
