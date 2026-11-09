import { useParams } from '@tanstack/react-router';
import type { ReactNode } from 'react';

import { useMe } from '../auth/me';
import { keyOf } from '../auth/permissions';
import { CardHeading } from '../components/Card';
import { Notice } from '../components/Notice';
import { useI18n } from '../i18n/I18nProvider';
import { AccountSettings } from './Account';
import { BackupsSettings } from './Backups';
import { BansSettings } from './Bans';
import { BuildSettings } from './Builds';
import { CATEGORIES, opens, type Category, type CategoryId } from './catalog';
import { DataSettings } from './Data';
import { LiveMapSettings } from './LiveMap';
import { FeatureSettings } from './Features';
import { FilesSettings } from './Files';
import { LaunchSettings } from './Launch';
import { PanelSettings } from './Panel';
import { RestartSettings } from './Restarts';
import { CategoryHead } from './Row';
import { ToolSettings } from './Tools';
import { UsersSettings } from './Users';

const CONTENT = {
    launch: LaunchSettings,
    restarts: RestartSettings,
    builds: BuildSettings,
    backups: BackupsSettings,
    bans: BansSettings,
    map: LiveMapSettings,
    features: FeatureSettings,
    data: DataSettings,
    tools: ToolSettings,
    panel: PanelSettings,
    users: UsersSettings,
    files: FilesSettings,
    account: AccountSettings,
} satisfies Record<CategoryId, () => ReactNode>;

/** The category in the address, to whoever may open it; its route turned any other name away. */
export function SettingsCategory() {
    const { category: id } = useParams({ strict: false });
    const me = useMe();
    const category = CATEGORIES.find((each) => each.id === id);
    if (category === undefined) {
        return null;
    }
    if (!opens(me, category)) {
        return <Refused category={category} />;
    }
    const Content = CONTENT[category.id];
    // Under the page's title and the category's own: the cards of a category are third.
    return (
        <CardHeading value="h3">
            <Content />
        </CardHeading>
    );
}

/** Said in place of the settings: the API refuses them anyway, and the address may come from a link. */
function Refused({ category }: { category: Category }) {
    const { t } = useI18n();
    const { access } = category;
    const held =
        access === 'owner' || access === 'everyone'
            ? []
            : access.map((name) => t.permissions[keyOf(name)].name);
    return (
        <>
            <CategoryHead category={category.id} />
            <Notice tone="warning">
                {access === 'owner'
                    ? t.settings.ownerOnly
                    : held.length === 1
                      ? t.forbidden.body(held[0] ?? '')
                      : t.forbidden.anyOf(held)}
            </Notice>
        </>
    );
}
