import { Plus, TriangleAlert, X } from 'lucide-react';
import { useId, useState } from 'react';

import type { CfgOutline } from '../api/generated/CfgOutline';
import { Button } from '../components/Button';
import { Chip } from '../components/Chip';
import { PasswordField, TextField } from '../components/Field';
import { Switch } from '../components/Switch';
import { useI18n } from '../i18n/I18nProvider';
import {
    FIELDS,
    pairedEndpoints,
    resourceName,
    settingOf,
    settingsOf,
    writable,
    type FieldId,
    type Guided,
} from './guided';

type FormProps = {
    outline: CfgOutline;
    guided: Guided;
    onChange: (guided: Guided) => void;
};

/**
 * The cfg by field: each one a line of the file, which it names, and a switch for each line that
 * starts a resource. What it changes is written into the text; the rest of the file is the File
 * view's.
 */
export function GuidedForm(props: FormProps) {
    const { t } = useI18n();
    const words = t.cfg.guided;
    const listId = useId();
    const accessId = useId();
    return (
        <div className="grid grid-cols-1 items-start gap-3 xl:grid-cols-2">
            <div className="flex min-w-0 flex-col gap-3">
                <section className="card" aria-labelledby={listId}>
                    <div className="card-head">
                        <h2 id={listId} className="card-title">
                            {words.list}
                        </h2>
                    </div>
                    <div className="card-body flex flex-col gap-4 max-md:p-4">
                        <Setting {...props} field="sv_hostname" />
                        <Setting {...props} field="sv_projectName" />
                        <Setting {...props} field="sv_projectDesc" />
                        <Setting {...props} field="tags" />
                        <Setting {...props} field="locale" />
                    </div>
                </section>
                <section className="card" aria-labelledby={accessId}>
                    <div className="card-head">
                        <h2 id={accessId} className="card-title">
                            {words.access}
                        </h2>
                    </div>
                    <div className="card-body flex flex-col gap-4 max-md:p-4">
                        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
                            <Setting {...props} field="sv_maxclients" />
                            <Setting {...props} field="sv_enforceGameBuild" />
                        </div>
                        <Endpoint {...props} />
                        <Secret {...props} field="sv_licenseKey" />
                        <Secret {...props} field="steam_webApiKey" />
                    </div>
                </section>
            </div>
            <Resources {...props} />
        </div>
    );
}

/** Where a field's value is written: its line, or the line a save adds. */
const useWhere = (outline: CfgOutline, field: FieldId) => {
    const { t, format } = useI18n();
    const words = t.cfg.guided;
    const setting = settingOf(outline, field);
    if (setting === null) {
        return words.absent(FIELDS[field].added);
    }
    const written = setting.command === null ? setting.name : `${setting.command} ${setting.name}`;
    const line = format.number(setting.line, { useGrouping: false });
    const times = settingsOf(outline, field);
    return times > 1 ? words.lastOf(written, line, times) : words.at(written, line);
};

function Setting({ outline, guided, onChange, field }: FormProps & { field: FieldId }) {
    const { t } = useI18n();
    const words = t.cfg.guided;
    const where = useWhere(outline, field);
    const value = guided.values[field] ?? settingOf(outline, field)?.value ?? '';
    return (
        <TextField
            label={words.fields[field]}
            value={value}
            inputMode={FIELDS[field].number ? 'numeric' : undefined}
            spellCheck={false}
            autoComplete="off"
            hint={<span className="font-mono text-meta [font-variant-ligatures:none]">{where}</span>}
            error={writable(value) ? undefined : words.unwritable}
            onChange={(event) =>
                onChange({ ...guided, values: { ...guided.values, [field]: event.target.value } })
            }
        />
    );
}

/** A secret of the file: said to be there, never shown, and replaced by what is typed over it. */
function Secret({ outline, guided, onChange, field }: FormProps & { field: FieldId }) {
    const { t } = useI18n();
    const words = t.cfg.guided;
    const where = useWhere(outline, field);
    const typed = guided.values[field];
    const [replacing, setReplacing] = useState(false);
    const setting = settingOf(outline, field);
    // A value the file shows is no secret to hide: an empty key, or a placeholder.
    const kept = setting !== null && setting.value === null;

    if (kept && typed === undefined && !replacing) {
        return (
            <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 rounded-box border border-line bg-surface-2 px-3.5 py-3">
                <div className="flex min-w-0 flex-col gap-0.5">
                    <span className="text-ui font-medium text-ink">{words.fields[field]}</span>
                    <span className="text-small text-ink-3">{words.secretKept}</span>
                    <span className="font-mono text-meta text-ink-3 [font-variant-ligatures:none]">
                        {where}
                    </span>
                </div>
                <Button size="sm" onClick={() => setReplacing(true)}>
                    {words.replace}
                </Button>
            </div>
        );
    }
    const value = typed ?? (kept ? '' : (setting?.value ?? ''));
    const forget = () => {
        const { [field]: _, ...values } = guided.values;
        onChange({ ...guided, values });
        setReplacing(false);
    };
    return (
        <PasswordField
            label={words.fields[field]}
            value={value}
            autoComplete="off"
            spellCheck={false}
            hint={<span className="font-mono text-meta [font-variant-ligatures:none]">{where}</span>}
            error={writable(value) ? undefined : words.unwritable}
            below={
                kept && (
                    <button
                        type="button"
                        className="self-start text-small font-medium text-ink-2 underline-offset-2 hover:text-ink hover:underline max-md:min-h-11"
                        onClick={forget}
                    >
                        {words.keepSecret}
                    </button>
                )
            }
            onChange={(event) =>
                onChange({ ...guided, values: { ...guided.values, [field]: event.target.value } })
            }
        />
    );
}

/** The game address: one field when the cfg opens one address on TCP and UDP, else what it opens. */
function Endpoint({ outline, guided, onChange }: FormProps) {
    const { t, format } = useI18n();
    const words = t.cfg.guided;
    const pair = pairedEndpoints(outline);
    const line = (number: number) => format.number(number, { useGrouping: false });
    if (pair === null) {
        return (
            <div className="flex flex-col gap-1.5">
                <span className="text-ui font-medium text-ink">{words.endpoint}</span>
                {outline.endpoints.length === 0 ? (
                    <p className="text-small text-ink-3">{words.noEndpoint}</p>
                ) : (
                    <ul className="flex flex-col gap-1 font-mono text-small text-ink-2 [font-variant-ligatures:none]">
                        {outline.endpoints.map((endpoint) => (
                            <li key={`${endpoint.line}:${endpoint.start}`}>
                                {words.endpointLine(endpoint.protocol, endpoint.address, line(endpoint.line))}
                            </li>
                        ))}
                    </ul>
                )}
                <p className="text-small text-ink-3">{words.endpointsInFile}</p>
            </div>
        );
    }
    const value = guided.endpoint ?? pair[0].address;
    return (
        <TextField
            label={words.endpoint}
            value={value}
            spellCheck={false}
            autoComplete="off"
            inputClassName="font-mono [font-variant-ligatures:none]"
            hint={
                <span className="font-mono text-meta [font-variant-ligatures:none]">
                    {words.endpointAt(line(pair[0].line), line(pair[1].line))}
                </span>
            }
            error={writable(value) ? undefined : words.unwritable}
            onChange={(event) => onChange({ ...guided, endpoint: event.target.value })}
        />
    );
}

/** Every line that starts a resource, in the order of the file, each with its switch. */
function Resources({ outline, guided, onChange }: FormProps) {
    const { t, format } = useI18n();
    const words = t.cfg.guided;
    const titleId = useId();
    const [name, setName] = useState('');
    const [refused, setRefused] = useState<string | null>(null);
    const missing = outline.resources.filter((resource) => resource.enabled && resource.missing).length;

    const add = () => {
        const wanted = name.trim();
        const listed =
            guided.added.includes(wanted) ||
            outline.resources.some((resource) => resource.enabled && resource.name === wanted);
        if (!resourceName(wanted)) {
            setRefused(words.notAName);
        } else if (listed) {
            setRefused(words.alreadyStarted(wanted));
        } else {
            onChange({ ...guided, added: [...guided.added, wanted] });
            setName('');
        }
    };

    return (
        <section className="card flex min-w-0 flex-col" aria-labelledby={titleId}>
            <div className="card-head">
                <h2 id={titleId} className="card-title">
                    {words.resources}
                </h2>
                <span className="text-small text-ink-3">{words.resourcesHint}</span>
            </div>
            <ol className="flex max-h-[32rem] flex-col overflow-y-auto overscroll-contain scrollbar-thin">
                {outline.resources.map((resource) => {
                    const started = guided.started[resource.line] ?? resource.enabled;
                    return (
                        <li
                            key={`${resource.line}:${resource.name}`}
                            className="flex min-h-11 items-center gap-3 border-b border-line px-5 py-2 last:border-b-0 max-md:px-4"
                        >
                            <span className="w-9 flex-none text-right font-mono text-meta text-ink-3 tabular-nums">
                                {format.number(resource.line, { useGrouping: false })}
                            </span>
                            <span className="min-w-0 flex-1 truncate font-mono text-ui text-ink [font-variant-ligatures:none]">
                                {resource.name}
                            </span>
                            {resource.missing && (
                                <Chip tone="warning" icon={TriangleAlert}>
                                    {words.missing}
                                </Chip>
                            )}
                            <Switch
                                aria-label={words.starts(resource.name)}
                                title={resource.alone ? undefined : words.shared}
                                checked={started}
                                disabled={!resource.alone}
                                onChange={(event) =>
                                    onChange({
                                        ...guided,
                                        started: { ...guided.started, [resource.line]: event.target.checked },
                                    })
                                }
                            />
                        </li>
                    );
                })}
                {guided.added.map((added) => (
                    <li
                        key={`added:${added}`}
                        className="flex min-h-11 items-center gap-3 border-b border-line px-5 py-2 last:border-b-0 max-md:px-4"
                    >
                        <span className="w-9 flex-none text-right font-mono text-meta text-ink-3">+</span>
                        <span className="min-w-0 flex-1 truncate font-mono text-ui text-ink [font-variant-ligatures:none]">
                            {added}
                        </span>
                        <Chip tone="neutral">{words.added}</Chip>
                        <button
                            type="button"
                            aria-label={words.remove(added)}
                            className="-m-1 rounded-sm p-1 text-ink-3 hover:text-ink max-md:min-h-11 max-md:min-w-11"
                            onClick={() =>
                                onChange({ ...guided, added: guided.added.filter((each) => each !== added) })
                            }
                        >
                            <X size={15} strokeWidth={1.5} aria-hidden="true" />
                        </button>
                    </li>
                ))}
                {outline.resources.length === 0 && guided.added.length === 0 && (
                    <li className="px-5 py-4 text-small text-ink-3 max-md:px-4">{words.noResource}</li>
                )}
            </ol>
            <div className="flex flex-col gap-3 border-t border-line p-5 max-md:p-4">
                {(missing > 0 || outline.truncated) && (
                    <p className="text-small text-ink-3">
                        {missing > 0 && words.missingCount(missing)} {outline.truncated && words.truncated}
                    </p>
                )}
                <form
                    className="flex flex-wrap items-end gap-2"
                    onSubmit={(event) => {
                        event.preventDefault();
                        add();
                    }}
                >
                    <TextField
                        className="min-w-0 flex-[1_1_14rem]"
                        label={words.add}
                        value={name}
                        spellCheck={false}
                        autoComplete="off"
                        inputClassName="font-mono [font-variant-ligatures:none]"
                        placeholder={words.addHint}
                        error={refused ?? undefined}
                        onChange={(event) => {
                            setName(event.target.value);
                            setRefused(null);
                        }}
                    />
                    <Button type="submit" icon={Plus} className={refused === null ? undefined : 'mb-6.5'}>
                        {words.addAction}
                    </Button>
                </form>
            </div>
        </section>
    );
}
