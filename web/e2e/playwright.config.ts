import { defineConfig, devices } from '@playwright/test';

import { panelUrl } from './observer';

// One Observer for the whole run, the flows one after the other on one page: each starts from
// the state the previous one left.
export default defineConfig({
    testDir: '.',
    testMatch: 'flows.spec.ts',
    outputDir: 'results/artifacts',
    globalSetup: './observer.ts',
    workers: 1,
    fullyParallel: false,
    retries: 0,
    forbidOnly: true,
    timeout: 60_000,
    expect: { timeout: 30_000 },
    reporter: [['list'], ['html', { outputFolder: 'results/report', open: 'never' }]],
    use: {
        ...devices['Desktop Chrome'],
        baseURL: panelUrl,
        viewport: { width: 1440, height: 900 },
        colorScheme: 'dark',
        trace: 'retain-on-failure',
        screenshot: 'only-on-failure',
    },
    projects: [{ name: 'chromium', use: { browserName: 'chromium' } }],
});
