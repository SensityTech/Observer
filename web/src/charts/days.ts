import type { ChartOptions, DeepPartial, Time } from 'lightweight-charts';

import { formatCalendarDay } from '../i18n/format';
import { chartOptions } from './theme';

/**
 * A chart of days. Its times are `YYYY-MM-DD` strings, which the library keeps as calendar days:
 * a day in the schedule's zone stays that day in any browser, never an instant moved by its zone.
 */
export function dayChartOptions(host: Element, locale: string): DeepPartial<ChartOptions> {
    const base = chartOptions(host, locale);
    const day = (time: Time) =>
        typeof time === 'string' ? formatCalendarDay(locale, time, { weekday: false }) : '';
    return {
        ...base,
        timeScale: { ...base.timeScale, timeVisible: false, tickMarkFormatter: day },
        localization: { ...base.localization, timeFormatter: day },
    };
}
