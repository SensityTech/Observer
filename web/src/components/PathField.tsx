import { TextField } from './Field';

/** A path on the host, in mono, which no spelling, capital or autofill of the browser may alter. */
export function PathField({
    label,
    hint,
    value,
    error,
    onChange,
}: {
    label: string;
    hint: string;
    value: string;
    error: string | undefined;
    onChange: (value: string) => void;
}) {
    return (
        <TextField
            label={label}
            hint={hint}
            value={value}
            error={error}
            spellCheck={false}
            autoCapitalize="none"
            autoComplete="off"
            inputClassName="font-mono text-ui"
            onChange={(event) => onChange(event.target.value)}
        />
    );
}
