import { expect, test, type Locator, type Page } from '@playwright/test';

import { en } from '../src/i18n/en';

const PASSWORD = 'e2e-owner-passphrase';
const setup = en.serverSetup;
const artifacts = en.artifacts;
const windows = process.platform === 'win32';
/** The one build the test catalog lists, which the fake serves as an archive of itself. */
const CATALOG_BUILD = '99001';

/** The step a job's card marks as the one it is at. */
const currentStep = (job: Locator) => job.locator('li[aria-current="step"]');

const env = (name: string) => {
    const value = process.env[name];
    if (value === undefined) {
        throw new Error(`${name} is not set: the global setup did not run`);
    }
    return value;
};

test.describe.serial('Observer in a browser', () => {
    let page: Page;

    test.beforeAll(async ({ browser }) => {
        page = await browser.newPage();
    });

    test.afterAll(async () => {
        await page.close();
    });

    const status = () => page.locator('dl.status');
    const controls = () => page.getByRole('group', { name: en.controls.label });
    /** Answers the confirmation titled `title` with its button `answer`. */
    const confirm = async (title: string, answer: string) => {
        const dialog = page.locator('dialog[open]');
        await expect(dialog).toContainText(title);
        await dialog.getByRole('button', { name: answer, exact: true }).click();
    };
    const overview = async () => {
        await page.getByRole('link', { name: en.nav.overview, exact: true }).click();
        await expect(page.getByRole('heading', { level: 1 })).toContainText(en.overview.title);
    };
    const buildCell = () =>
        status()
            .locator('.status-cell', { has: page.getByText(en.status.build, { exact: true }) })
            .locator('dd');
    /** Starts the install of the catalog's build and sees its download under way. */
    const installFromCatalog = async () => {
        await page.getByRole('link', { name: en.nav.artifacts, exact: true }).click();
        await expect(page.getByRole('heading', { level: 1 })).toContainText(artifacts.title);
        await page
            .getByRole('region', { name: artifacts.available.title, exact: true })
            .getByRole('row', { name: new RegExp(CATALOG_BUILD) })
            .getByRole('button', { name: artifacts.available.install })
            .click();
        const dialog = page.getByRole('dialog', { name: artifacts.install.title(CATALOG_BUILD) });
        await dialog.getByRole('button', { name: artifacts.install.confirm, exact: true }).click();

        const job = page.getByRole('region', { name: artifacts.job.install(CATALOG_BUILD) });
        // The fake spreads its archive over a few seconds, as a real download takes them.
        await expect(currentStep(job)).toHaveText(artifacts.job.steps.download);
        await expect(job.getByRole('progressbar', { name: artifacts.job.progress })).toBeVisible();
        return job;
    };

    test('the owner sets up Observer from the printed link, then the server through its setup', async () => {
        await page.goto(env('E2E_SETUP_LINK'));
        await page.getByLabel(en.setup.username).fill('ada');
        await page.getByLabel(en.setup.password, { exact: true }).fill(PASSWORD);
        await page.getByLabel(en.setup.confirm).fill(PASSWORD);
        await page.getByRole('button', { name: en.setup.submit }).click();
        await expect(page.getByRole('heading', { level: 1 })).toContainText(setup.title);

        await page.getByRole('textbox', { name: setup.folder.label }).fill(env('E2E_SERVER_DATA'));
        await page.getByRole('button', { name: setup.folder.check }).click();
        await expect(page.getByText(setup.folder.accepted)).toBeVisible();
        await page.getByRole('button', { name: setup.continue }).click();

        // Entering the step checks the cfg: without a licence key it warns, and nothing blocks.
        await expect(page.getByText(setup.cfg.kinds.licenceMissing())).toBeVisible();
        await page.getByRole('button', { name: setup.continue }).click();

        const build = env('E2E_BUILD');
        await page.getByRole('button', { name: setup.build.use(build) }).click();
        const dialog = page.getByRole('dialog', { name: en.artifacts.use.title(build) });
        await expect(
            dialog.getByRole('radio', { name: new RegExp(en.artifacts.use.nextStart) }),
        ).toBeChecked();
        await dialog.getByRole('button', { name: en.artifacts.use.confirm, exact: true }).click();
        await expect(page.getByText(setup.build.selected(build))).toBeVisible();
        await page.getByRole('button', { name: setup.continue }).click();

        await page.getByRole('button', { name: setup.start.saveAndStart }).click();
        // A sign-in a moment ago may stand for the password; otherwise the panel asks for it.
        const reauth = page.locator('dialog[open]').filter({ hasText: en.reauth.title });
        const starting = page.getByText(setup.start.starting);
        await expect(starting.or(reauth)).toBeVisible();
        if (await reauth.isVisible()) {
            await reauth.getByLabel(en.reauth.password, { exact: true }).fill(PASSWORD);
            await reauth.getByRole('button', { name: en.reauth.submit }).click();
        }
        await expect(starting).toBeVisible();
        // Ready once the fake's bridge has reported in.
        await expect(page.getByText(setup.launch.ready)).toBeVisible();
        await expect(status()).toContainText(en.server_state.states.online);
    });

    test('the owner signs in again and lands on the Overview, the server being set up', async () => {
        await page.getByRole('button', { name: en.account.menu('ada') }).click();
        await page.getByRole('menuitem', { name: en.account.signOut }).click();
        await page.getByLabel(en.login.username).fill('ada');
        await page.getByLabel(en.login.password, { exact: true }).fill(PASSWORD);
        await page.getByRole('button', { name: en.login.submit }).click();
        await expect(page.getByRole('heading', { level: 1 })).toContainText(en.overview.title);
    });

    test('FXServer restarts, stops and starts from the top bar', async () => {
        await overview();
        await expect(status()).toContainText(en.server_state.states.online);

        await controls().getByRole('button', { name: en.controls.restart, exact: true }).click();
        await confirm(en.controls.restartTitle, en.controls.confirmRestart);
        await expect(page.getByText(en.server_state.restarted('ada'))).toBeVisible();
        await expect(status()).toContainText(en.server_state.states.online);

        await controls().getByRole('button', { name: en.controls.stop, exact: true }).click();
        await confirm(en.controls.stopTitle, en.controls.confirmStop);
        await expect(status()).toContainText(en.server_state.states.stopped);
        await expect(page.getByText(en.server_state.stopped('ada'))).toBeVisible();

        await controls().getByRole('button', { name: en.controls.start, exact: true }).click();
        await expect(status()).toContainText(en.server_state.states.online);
    });

    test('a console command reaches FXServer and its output comes back', async () => {
        await page.getByRole('link', { name: en.nav.console, exact: true }).click();
        const command = page.getByRole('textbox', { name: en.console.commandLabel });

        await command.fill('fake_print hello from the browser');
        await command.press('Enter');
        // The line FXServer printed, not the echo of the command.
        await expect(page.getByText(/^(?!.*fake_print).*hello from the browser/)).toBeVisible();

        await command.fill('fake_players 5');
        await command.press('Enter');
        await expect(page.getByText(en.console.sent('fake_players 5'))).toBeVisible();
    });

    test('the statistics tiles show what FXServer reports', async () => {
        await overview();
        const players = page.getByRole('region', { name: en.tiles.players.label, exact: true });
        await expect(players).toContainText(en.tiles.players.of('32'));
        await expect(players.locator('p').first()).toHaveText(/^5/, { timeout: 15_000 });
        const tiles = [
            en.tiles.tick.label,
            en.tiles.cpu.label,
            en.tiles.memory.label,
            en.tiles.host.label,
            en.tiles.observer.label,
        ];
        await Promise.all(
            tiles.map((label) =>
                expect(
                    page.getByRole('region', { name: label, exact: true }).locator('p').first(),
                    label,
                ).not.toHaveText(en.tiles.unknown, { timeout: 15_000 }),
            ),
        );
    });

    test('a player who joins shows online once FXServer lists them, with a profile of their own', async () => {
        await page.getByRole('link', { name: en.nav.console, exact: true }).click();
        const command = page.getByRole('textbox', { name: en.console.commandLabel });
        await command.fill('fake_join Ada');
        await command.press('Enter');
        await expect(page.getByText(en.console.sent('fake_join Ada'))).toBeVisible();

        await page.getByRole('link', { name: en.nav.players, exact: true }).click();
        await expect(page.getByRole('heading', { level: 1 })).toContainText(en.players.title);
        const online = page.getByRole('region', { name: en.players.now.title, exact: true });
        // Confirmed against FXServer's player list, read every five seconds, then shown at the
        // page's next refresh.
        const ada = online.getByRole('link', { name: 'Ada', exact: true });
        await expect(ada).toBeVisible({ timeout: 30_000 });
        await ada.click();

        await expect(page.getByRole('heading', { level: 1 })).toContainText('Ada');
        await expect(page.getByText(en.players.online, { exact: true }).first()).toBeVisible();
    });

    test('a one-off restart happens at its time', async () => {
        test.setTimeout(120_000);
        await page.getByRole('link', { name: en.nav.schedule, exact: true }).click();
        await page.getByRole('button', { name: en.schedule.restartOnce }).click();
        const dialog = page.locator('dialog[open]');
        await expect(dialog).toContainText(en.schedule.onceTitle);
        await dialog.getByLabel(en.schedule.onceIn, { exact: true }).check();
        await dialog.getByLabel(en.schedule.onceMinutes).fill('1');
        await dialog.getByRole('button', { name: en.schedule.onceSubmit }).click();
        await expect(page.getByText(en.schedule.oneOff('ada'))).toBeVisible();

        await overview();
        await expect(page.getByText(en.server_state.restartedAsPlanned('ada'))).toBeVisible({
            timeout: 100_000,
        });
        await expect(status()).toContainText(en.server_state.states.online);
    });

    test('a build from the catalog downloads, unpacks and installs', async () => {
        test.skip(windows, 'Windows refuses the unsigned fake at validation, as the next flow shows');
        const job = await installFromCatalog();

        await expect(job.getByText(artifacts.job.installed, { exact: true })).toBeVisible();
        const steps = artifacts.job.steps;
        await expect(job.getByRole('list', { name: artifacts.job.title }).getByRole('listitem')).toHaveText([
            steps.preCheck,
            steps.download,
            steps.verify,
            steps.extract,
            steps.validate,
            steps.commit,
        ]);
        await expect(currentStep(job)).toHaveText(steps.commit);
    });

    test('Windows refuses a build from the catalog whose FXServer.exe Rockstar Games did not sign', async () => {
        test.skip(!windows, 'Linux builds carry no signature');
        const job = await installFromCatalog();

        const words = artifacts.job;
        await expect(
            job.getByText(words.failed(words.during.validate, words.failures.signature)),
        ).toBeVisible();
        await expect(currentStep(job)).toHaveText(words.steps.validate);
    });

    test('the installed build starts at the next start, on probation', async () => {
        test.skip(windows, 'Windows refuses the unsigned fake at validation');
        // Wide enough for the status bar to say why the build is shown.
        await page.setViewportSize({ width: 1680, height: 900 });
        const job = page.getByRole('region', { name: artifacts.job.install(CATALOG_BUILD) });
        await job.getByRole('button', { name: artifacts.job.use }).click();
        const dialog = page.getByRole('dialog', { name: artifacts.use.title(CATALOG_BUILD) });
        await dialog.getByRole('radio', { name: new RegExp(artifacts.use.nextStart) }).check();
        await dialog.getByRole('button', { name: artifacts.use.confirm, exact: true }).click();
        await expect(page.getByText(artifacts.use.done.nextStart(CATALOG_BUILD))).toBeVisible();
        await expect(buildCell()).toHaveText(`${env('E2E_BUILD')} · ${en.status.nextBuild(CATALOG_BUILD)}`, {
            useInnerText: true,
        });

        await controls().getByRole('button', { name: en.controls.restart, exact: true }).click();
        await confirm(en.controls.restartTitle, en.controls.confirmRestart);
        await expect(buildCell()).toHaveText(`${CATALOG_BUILD} · ${en.status.probation}`, {
            useInnerText: true,
        });
        await expect(status()).toContainText(en.server_state.states.online);
        await page.setViewportSize({ width: 1440, height: 900 });
    });
});
