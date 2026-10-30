// The Observer the browser tests run against: the debug binary on a data folder of its own, with
// the fake FXServer installed as a build no one has selected yet. Global setup starts it; the
// function it returns stops it.
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
const web = resolve(import.meta.dirname, '..');
const results = join(import.meta.dirname, 'results');
const patience = 30_000;
const SETUP_LINK = /http:\/\/\S+\/setup#\S+/;
/** The build the fake stands for: the one it reports in `/info.json`. */
const BUILD = 35245;

export const panelUrl = `http://127.0.0.1:${port}`;

const binary = (variable: string, name: string) => {
    const path =
        process.env[variable] ?? resolve(web, '..', 'target', 'debug', windows ? `${name}.exe` : name);
    if (!existsSync(path)) {
        throw new Error(
            `${path} is missing: run \`cargo build -p observer -p fake-fxserver\` first, or set ${variable}`,
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
    // A debug binary serves the panel from disk.
    if (!existsSync(join(web, 'dist', 'index.html'))) {
        throw new Error('web/dist is missing: run `cargo xtask web` (or `npm run build`) first');
    }

    const dir = mkdtempSync(join(tmpdir(), 'observer-e2e-'));
    const serverData = join(dir, 'server-data');
    installFake(join(dir, 'data', 'artifacts'), fake);
    // The server setup takes a folder only once its resources folder holds a resource.
    const resource = join(serverData, 'resources', '[local]', 'hello');
    mkdirSync(resource, { recursive: true });
    writeFileSync(join(resource, 'fxmanifest.lua'), "fx_version 'cerulean'\ngame 'gta5'\n");
    writeFileSync(
        join(serverData, 'server.cfg'),
        `endpoint_add_tcp "127.0.0.1:${fxserverPort}"\nendpoint_add_udp "127.0.0.1:${fxserverPort}"\nsv_maxclients 32\nensure hello\n`,
    );
    const config = join(dir, 'observer.toml');
    writeFileSync(config, `data_dir = "data"\n\n[http]\nlisten = "127.0.0.1:${port}"\n`);

    mkdirSync(results, { recursive: true });
    const env = { ...process.env };
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
        throw new Error(`Observer printed no setup link (exit ${ran.exit}):\n${output}`);
    }
    if (!(await until(answers, deadline))) {
        child.kill();
        throw new Error(`the panel did not answer on ${panelUrl}`);
    }

    process.env.E2E_URL = panelUrl;
    process.env.E2E_SETUP_LINK = link;
    process.env.E2E_BUILD = String(BUILD);
    process.env.E2E_SERVER_DATA = serverData;

    // Windows holds the databases and the fake's copy a moment after the process ends.
    const removeDir = () => rmSync(dir, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });

    return async () => {
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
