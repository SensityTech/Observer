import { ShieldAlert, TriangleAlert } from 'lucide-react';
import { useId } from 'react';

import type { PermissionArea } from '../api/generated/PermissionArea';
import type { PermissionInfo } from '../api/generated/PermissionInfo';
import { bitMask, holds, impliedBy, parsePermissions, setPermission } from '../auth/permissions';
import { Checkbox } from '../components/Checkbox';
import { Chip } from '../components/Chip';
import { useI18n } from '../i18n/I18nProvider';
import type { Messages } from '../i18n/en';

type PermissionEditorProps = {
    catalog: PermissionInfo[];
    value: bigint;
    onChange: (value: bigint) => void;
    /** What the editor may grant or withdraw; the rest shows but cannot change. */
    grantable: bigint;
};

const name = (t: Messages, info: PermissionInfo) =>
    Object.hasOwn(t.permissions, info.key) ? t.permissions[info.key as keyof Messages['permissions']] : null;

/** The catalog grouped by area, with what each permission implies and how dangerous it is. */
export function PermissionEditor({ catalog, value, onChange, grantable }: PermissionEditorProps) {
    const { t } = useI18n();
    const areas = [...new Set(catalog.map((info) => info.area))];

    return (
        <fieldset className="flex flex-col gap-4">
            <legend className="mb-2 text-ui font-medium text-ink">{t.users.editor.legend}</legend>
            {areas.map((area) => (
                <Area
                    key={area}
                    area={area}
                    catalog={catalog}
                    value={value}
                    onChange={onChange}
                    grantable={grantable}
                />
            ))}
        </fieldset>
    );
}

function Area({
    area,
    catalog,
    value,
    onChange,
    grantable,
}: PermissionEditorProps & { area: PermissionArea }) {
    const { t } = useI18n();

    return (
        <fieldset className="flex flex-col gap-1">
            <legend className="eyebrow mb-1">{t.permissionAreas[area]}</legend>
            {catalog
                .filter((info) => info.area === area)
                .map((info) => (
                    <PermissionRow
                        key={info.bit}
                        info={info}
                        catalog={catalog}
                        value={value}
                        onChange={onChange}
                        grantable={grantable}
                    />
                ))}
        </fieldset>
    );
}

function PermissionRow({
    info,
    catalog,
    value,
    onChange,
    grantable,
}: PermissionEditorProps & { info: PermissionInfo }) {
    const { t } = useI18n();
    const id = useId();
    const words = name(t, info);
    const granters = impliedBy(value, info, catalog);
    const implied = parsePermissions(info.implies);
    const impliedNames = catalog
        .filter((other) => holds(implied, bitMask(other.bit)))
        .map((other) => name(t, other)?.name ?? other.key);
    const held = holds(grantable, bitMask(info.bit));
    const notes = [
        impliedNames.length > 0 && t.users.editor.implies(impliedNames),
        granters.length > 0 &&
            t.users.editor.grantedWith(granters.map((other) => name(t, other)?.name ?? other.key)),
        !held && t.users.editor.notHeld,
    ].filter((note) => note !== false);

    // Named by the permission alone; its description, danger and notes describe it.
    return (
        <label
            htmlFor={`${id}-box`}
            className="group flex cursor-pointer items-start gap-3 rounded-sm px-2 py-2 hover:bg-hover has-disabled:cursor-default has-disabled:hover:bg-transparent"
        >
            <Checkbox
                id={`${id}-box`}
                checked={holds(value, bitMask(info.bit))}
                disabled={!held || granters.length > 0}
                aria-labelledby={`${id}-name`}
                aria-describedby={`${id}-about`}
                onChange={(event) => onChange(setPermission(value, info, event.target.checked, catalog))}
            />
            <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span className="flex flex-wrap items-center gap-x-2 gap-y-1 text-body font-medium text-ink">
                    <span id={`${id}-name`}>{words?.name ?? info.key}</span>
                    {info.danger === 'sensitive' && <Chip icon={ShieldAlert}>{t.danger.sensitive}</Chip>}
                    {info.danger === 'critical' && (
                        <Chip tone="warning" icon={TriangleAlert}>
                            {t.danger.critical}
                        </Chip>
                    )}
                </span>
                <span id={`${id}-about`} className="flex flex-col gap-0.5">
                    {info.danger !== 'normal' && <span className="sr-only">{t.danger[info.danger]}.</span>}
                    {words && <span className="text-small text-ink-2">{words.description}</span>}
                    {notes.map((note) => (
                        <span key={note} className="text-small text-ink-3 group-hover:text-ink-2">
                            {note}
                        </span>
                    ))}
                </span>
            </span>
        </label>
    );
}
