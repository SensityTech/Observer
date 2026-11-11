import { RotateCw, Square } from 'lucide-react';
import { useState } from 'react';

import type { ResourceRow } from '../api/generated/ResourceRow';
import { Button } from '../components/Button';
import { CheckRow } from '../components/CheckRow';
import { Chip } from '../components/Chip';
import { Dialog } from '../components/Dialog';
import { useI18n } from '../i18n/I18nProvider';

const mono = (name: string) => (
    <span key={name} className="font-mono text-ink [font-variant-ligatures:none]">
        {name}
    </span>
);

/** The started resources that depend on the one asked about, as FXServer names them. */
function Dependants({ names }: { names: readonly string[] }) {
    return (
        <ul className="flex flex-wrap gap-1.5">
            {names.map((name) => (
                <li key={name}>
                    <Chip className="font-mono [font-variant-ligatures:none]">{name}</Chip>
                </li>
            ))}
        </ul>
    );
}

/**
 * Asked before a restart that takes other resources down: FXServer stops what depends on the
 * resource and leaves it stopped, unless the order starts each of them again.
 */
export function RestartDialog({
    row,
    onClose,
    onConfirm,
}: {
    row: ResourceRow;
    onClose: () => void;
    /** `again`: start the resources that depend on it once it is back. */
    onConfirm: (again: boolean) => void;
}) {
    const { t } = useI18n();
    const words = t.resources;
    const [again, setAgain] = useState(true);
    const dependants = row.dependants.length;
    return (
        <Dialog
            role="alertdialog"
            title={words.restartTitle(mono(row.name))}
            onClose={onClose}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.common.cancel}
                    </Button>
                    <Button tone="ink" icon={RotateCw} onClick={() => onConfirm(again)}>
                        {words.confirmRestart(row.name, again ? dependants + 1 : 1)}
                    </Button>
                </>
            }
        >
            <p>{words.restartStops(dependants, mono(row.name))}</p>
            <Dependants names={row.dependants} />
            <div className="-mx-2">
                <CheckRow
                    label={words.again(dependants, row.name)}
                    description={words.againHint(dependants)}
                    checked={again}
                    onChange={setAgain}
                />
            </div>
        </Dialog>
    );
}

/** Asked before a stop that takes other resources down with it. */
export function StopDialog({
    row,
    onClose,
    onConfirm,
}: {
    row: ResourceRow;
    onClose: () => void;
    onConfirm: () => void;
}) {
    const { t } = useI18n();
    const words = t.resources;
    const dependants = row.dependants.length;
    return (
        <Dialog
            role="alertdialog"
            title={words.stopTitle(mono(row.name))}
            onClose={onClose}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.common.cancel}
                    </Button>
                    <Button tone="danger" icon={Square} onClick={onConfirm}>
                        {words.confirmStop(row.name, dependants + 1)}
                    </Button>
                </>
            }
        >
            <p>{words.stopStops(dependants, mono(row.name))}</p>
            <Dependants names={row.dependants} />
            <p className="text-small text-ink-2">{words.stayStopped(dependants)}</p>
        </Dialog>
    );
}
