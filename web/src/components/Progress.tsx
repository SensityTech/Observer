import { secondsLeft } from '../api/artifacts';
import type { DownloadProgress } from '../api/generated/DownloadProgress';
import type { Messages } from '../i18n/en';
import type { useI18n } from '../i18n/I18nProvider';

/** A bar of how far something went, null for a share not known yet; its figures in `valueText`. */
export function ProgressBar({
    label,
    valueText,
    share,
}: {
    label: string;
    valueText: string;
    share: number | null;
}) {
    return (
        <progress
            aria-label={label}
            aria-valuetext={valueText}
            max={100}
            value={share === null ? undefined : Math.round(share * 100)}
            className="h-1.5 w-full appearance-none overflow-hidden rounded-pill bg-surface-2 [&::-moz-progress-bar]:bg-accent [&::-webkit-progress-bar]:bg-surface-2 [&::-webkit-progress-value]:rounded-pill [&::-webkit-progress-value]:bg-accent"
        />
    );
}

/** What a download received, how fast, and how long it has left, once each is known. */
export const downloadFigures = (
    t: Messages,
    format: ReturnType<typeof useI18n>['format'],
    download: DownloadProgress,
): string[] => {
    const left = secondsLeft(download);
    return [
        t.download.received(format.bytes(download.received), format.bytes(download.total)),
        download.rate === 0 ? null : t.download.rate(format.bytes(download.rate)),
        left === null ? null : t.download.left(format.duration(left)),
    ].filter((part) => part !== null);
};
