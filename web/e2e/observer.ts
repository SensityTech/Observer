// The Observer the browser tests run against: the debug binary on a data folder of its own, with
// the fake FXServer installed as a build no one has selected yet, and the test catalog in place of
// Cfx.re's, whose one build is the fake as `fake-archive` serves it. Global setup starts both; the
// function it returns stops them.
import { spawn } from 'node:child_process';
import {
    chmodSync,
    copyFileSync,
    createWriteStream,
    existsSync,
    mkdirSync,
    mkdtempSync,
    rmSync,
    statSync,
    writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';

const windows = process.platform === 'win32';
const port = Number(process.env.E2E_PORT ?? 41966);
const fxserverPort = Number(process.env.E2E_FXSERVER_PORT ?? 30153);
const catalogPort = Number(process.env.E2E_CATALOG_PORT ?? 41955);
const web = resolve(import.meta.dirname, '..');
const results = join(import.meta.dirname, 'results');
const patience = 30_000;
const SETUP_LINK = /http:\/\/\S+\/setup#\S+/;
/** What Observer logs when it lists and downloads builds from the test catalog. */
const TEST_CATALOG = 'builds come from the test catalog';
/** The build the fake stands for: the one it reports in `/info.json`. */
const BUILD = 35245;
/**
 * How long a new build stays up to pass its probation, instead of ten minutes: the fake passes its
 * own before the flows choose the next build, and the next build is still on it when it starts.
 */
const PROBATION_SECONDS = 30;

export const panelUrl = `http://127.0.0.1:${port}`;

const binary = (variable: string, name: string) => {
    const path =
        process.env[variable] ?? resolve(web, '..', 'target', 'debug', windows ? `${name}.exe` : name);
    if (!existsSync(path)) {
        throw new Error(
            `${path} is missing: run \`cargo xtask e2e\`, or \`cargo build -p observer -p fake-fxserver --features observer/test-catalog\` first, or set ${variable}`,
        );
    }
    return path;
};

const executable = (path: string, text: string) => {
    writeFileSync(path, text);
    chmodSync(path, 0o755);
};

/**
 * The fake as build `BUILD` of Observer's artifacts folder: the layout Observer checks, and the
 * manifest an import writes. Windows only takes a signed FXServer.exe through an import, so the
 * build is put in place rather than imported.
 */
const installFake = (artifacts: string, fake: string) => {
    const build = join(artifacts, String(BUILD));
    const server = windows ? build : join(build, 'alpine/opt/cfx-server');
    const program = join(server, windows ? 'FXServer.exe' : 'ld-musl-x86_64.so.1');
    mkdirSync(join(server, 'citizen'), { recursive: true });
    copyFileSync(fake, program);
    chmodSync(program, 0o755);
    if (!windows) {
        executable(join(build, 'run.sh'), '#!/bin/sh\n');
        executable(join(server, 'FXServer'), '#!/bin/sh\n');
    }
    const manifest = {
        format: 2,
        build: BUILD,
        platform: windows ? 'windows' : 'linux',
        installedAt: new Date().toISOString(),
        installedBy: null,
        files: 1,
        bytes: statSync(fake).size,
        origin: { kind: 'import', folder: dirname(fake) },
    };
    writeFileSync(join(build, '.observer-build.json'), JSON.stringify(manifest, null, 2));
};

/** Whether `done` held before `deadline`, asked every 100 ms. */
const until = async (done: () => boolean | Promise<boolean>, deadline: number): Promise<boolean> => {
    if (await done()) {
        return true;
    }
    if (Date.now() > deadline) {
        return false;
    }
    await sleep(100);
    return until(done, deadline);
};

const answers = async () => {
    try {
        return (await fetch(`${panelUrl}/api/health`)).ok;
    } catch {
        return false;
    }
};

export default async function setup() {
    const observer = binary('OBSERVER_BIN', 'observer');
    const fake = binary('FAKE_FXSERVER_BIN', 'fake-fxserver');
    const archiveServer = binary('FAKE_ARCHIVE_BIN', 'fake-archive');
    // A debug binary serves the panel from disk.
    if (!existsSync(join(web, 'dist', 'index.html'))) {
        throw new Error('web/dist is missing: run `cargo xtask web` (or `npm run build`) first');
    }

    const dir = mkdtempSync(join(tmpdir(), 'observer-e2e-'));
    const serverData = join(dir, 'server-data');
    installFake(join(dir, 'data', 'artifacts'), fake);
    // The server setup takes a folder only once its resources folder holds a resource. A second
    // one depends on the first, which FXServer therefore stops and starts it with.
    const resources = join(serverData, 'resources', '[local]');
    const manifest = "fx_version 'cerulean'\ngame 'gta5'\n";
    mkdirSync(join(resources, 'hello'), { recursive: true });
    writeFileSync(join(resources, 'hello', 'fxmanifest.lua'), manifest);
    mkdirSync(join(resources, 'greeter'));
    writeFileSync(join(resources, 'greeter', 'fxmanifest.lua'), `${manifest}dependency 'hello'\n`);
    writeFileSync(
        join(serverData, 'server.cfg'),
        `endpoint_add_tcp "127.0.0.1:${fxserverPort}"\nendpoint_add_udp "127.0.0.1:${fxserverPort}"\nsv_maxclients 32\nensure hello\nensure greeter\n`,
    );
    const config = join(dir, 'observer.toml');
    writeFileSync(config, `data_dir = "data"\n\n[http]\nlisten = "127.0.0.1:${port}"\n`);

    mkdirSync(results, { recursive: true });
    const archive = spawn(archiveServer, [String(catalogPort)], {
        stdio: ['ignore', 'pipe', 'pipe'],
    });
    archive.stderr.pipe(createWriteStream(join(results, 'archive.err')));
    // It prints a line once it listens, which it does once its archive is made. An answer on the port
    // alone could come from another server, such as a fake-archive a killed run left behind.
    const served: { listening: boolean; exit: number | null } = { listening: false, exit: null };
    archive.stdout.once('data', () => {
        served.listening = true;
    });
    archive.on('exit', (code) => {
        served.exit = code ?? -1;
    });
    if (!(await until(() => served.listening || served.exit !== null, Date.now() + patience))) {
        archive.kill();
        throw new Error(`fake-archive did not serve on port ${catalogPort}`);
    }
    if (!served.listening) {
        throw new Error(`fake-archive could not serve (exit ${served.exit}), see e2e/results/archive.err`);
    }
    const stopArchive = async () => {
        if (served.exit === null) {
            archive.kill();
            await until(() => served.exit !== null, Date.now() + patience);
        }
    };

    const env: NodeJS.ProcessEnv = {
        ...process.env,
        OBSERVER_TEST_CATALOG: String(catalogPort),
        OBSERVER_TEST_PROBATION: String(PROBATION_SECONDS),
        // Nothing listens on port 1: the update checks never leave the machine.
        OBSERVER_TEST_RELEASES: '1',
    };
    // Under systemd Observer prints no setup link.
    delete env.INVOCATION_ID;
    delete env.NOTIFY_SOCKET;
    const args = ['--config', config, 'run', ...(windows ? ['--no-window'] : [])];
    const child = spawn(observer, args, { cwd: dir, env, stdio: ['ignore', 'pipe', 'pipe'] });
    let output = '';
    for (const [stream, name] of [
        [child.stdout, 'observer.out'],
        [child.stderr, 'observer.err'],
    ] as const) {
        stream.pipe(createWriteStream(join(results, name)));
        stream.on('data', (chunk: Buffer) => {
            output += chunk.toString();
        });
    }
    const ran: { exit: number | null } = { exit: null };
    child.on('exit', (code) => {
        ran.exit = code ?? -1;
    });

    const deadline = Date.now() + patience;
    await until(() => SETUP_LINK.test(output) || ran.exit !== null, deadline);
    const link = SETUP_LINK.exec(output)?.[0];
    if (link === undefined) {
        child.kill();
        await stopArchive();
        throw new Error(`Observer printed no setup link (exit ${ran.exit}):\n${output}`);
    }
    if (!(await until(answers, deadline))) {
        child.kill();
        await stopArchive();
        throw new Error(`the panel did not answer on ${panelUrl}`);
    }
    // Logged before the panel listens.
    if (!output.includes(TEST_CATALOG)) {
        child.kill();
        await stopArchive();
        throw new Error(
            'Observer was built without the test catalog: run `cargo xtask e2e`, or build it with --features observer/test-catalog',
        );
    }

    process.env.E2E_URL = panelUrl;
    process.env.E2E_SETUP_LINK = link;
    process.env.E2E_BUILD = String(BUILD);
    process.env.E2E_PROBATION = String(PROBATION_SECONDS);
    process.env.E2E_SERVER_DATA = serverData;

    // Windows holds the databases and the fake's copy a moment after the process ends.
    const removeDir = () => rmSync(dir, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });

    return async () => {
        await stopArchive();
        if (ran.exit === null && windows) {
            // No signal to send: the graceful stop is the console's or the window's, tested apart.
            child.kill();
            await until(() => ran.exit !== null, Date.now() + patience);
            removeDir();
            return;
        }
        if (ran.exit === null) {
            child.kill('SIGTERM');
            await until(() => ran.exit !== null, Date.now() + patience);
        }
        // A run that went wrong keeps its folder to look into.
        if (ran.exit !== 0 || !output.includes('Observer stopped')) {
            throw new Error(
                `Observer did not stop gracefully (exit ${ran.exit}); its folder ${dir} is kept, see e2e/results/observer.err`,
            );
        }
        removeDir();
    };
}
