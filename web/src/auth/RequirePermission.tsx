import { Link } from '@tanstack/react-router';
import { ShieldAlert } from 'lucide-react';
import type { ReactNode } from 'react';

import { Permission } from '../api/generated/Permission';
import { buttonVariants } from '../components/Button';
import { useI18n } from '../i18n/I18nProvider';
import { allows, useMe } from './me';
import { keyOf, type PermissionName } from './permissions';

/**
 * Shows a page only to a user who holds its permission, or any one of a list of them. The API
 * refuses the data anyway; this says why in place of an error.
 */
export function RequirePermission({
    name,
    children,
}: {
    name: PermissionName | PermissionName[];
    children: ReactNode;
}) {
    const { t } = useI18n();
    const me = useMe();
    const names = Array.isArray(name) ? name : [name];

    if (names.some((each) => allows(me, Permission[each]))) {
        return children;
    }
    const held = names.map((each) => t.permissions[keyOf(each)].name);
    return <Forbidden body={held.length === 1 ? t.forbidden.body(held[0] ?? '') : t.forbidden.anyOf(held)} />;
}

/** Shows a page only to the owner, for what no permission grants. */
export function RequireOwner({ children }: { children: ReactNode }) {
    const { t } = useI18n();
    const me = useMe();
    return me.isOwner ? children : <Forbidden body={t.forbidden.ownerOnly} />;
}

function Forbidden({ body }: { body: string }) {
    const { t } = useI18n();
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
                <p className="max-w-empty text-body text-ink-3">{body}</p>
                <Link
                    to="/settings/$category"
                    params={{ category: 'account' }}
                    className={buttonVariants({ tone: 'ink', size: 'sm' })}
                >
                    {t.forbidden.back}
                </Link>
            </section>
        </div>
    );
}
