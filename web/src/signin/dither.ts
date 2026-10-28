/*
 * The sign-in stage's dither iris, alive. The stylesheet draws the iris alone: a 3px dot every 6px,
 * split into interleaved lattices whose radial masks make it dense at the centre, ringed in the
 * middle and sparse at the edge. That stays the picture without script and under reduced motion.
 * Here a canvas takes over the same lattice and moves the field instead of the dots: the rings
 * travel outward, the dense core opens and closes like an aperture, turning a little as it does,
 * and dots fade in and out where the field crosses them. The dots themselves never move.
 *
 * One canvas pixel per dot, only their alpha changes; 30 frames a second at most; the loop stops
 * off screen and in a hidden tab.
 */

const CELL = 6;
const DOT = 3;
const TILE = 12;
const LAYER_BASE = 0;
const LAYER_RING = 1;
const LAYER_CORE = 2;
const FRAME_MS = 1000 / 30;
const FRAME_SLACK_MS = 4;
const TIMER_LEAD_MS = 4;
const MAX_STEP_MS = 100;
const RING_PERIOD = 48;
const RING_HALF_WIDTH = 15 / RING_PERIOD;
const RING_CENTRE = RING_HALF_WIDTH;
const RING_SOFTNESS = 2.4 / RING_PERIOD;
const RING_SPEED = 6;
const BREATH_SECONDS = 18;
const CORE_MIDDLE = 0.5;
const CORE_SWING = 0.08;
const CORE_FADE = 0.32;
const BREATH_PHASE = Math.asin(-0.25);
const CORE_TWIST = 0.1;
const CORE_HEXAGON = 0.035;
const SHIMMER = 0.3;
const SHIMMER_FADE_IN_SECONDS = 2;
const SHIMMER_SPEED_A = 0.11;
const SHIMMER_SPEED_B = 0.07;

const smoothstep = (from: number, to: number, value: number) => {
    const t = Math.min(1, Math.max(0, (value - from) / (to - from)));
    return t * t * (3 - 2 * t);
};

const positiveModulo = (value: number, modulus: number) => ((value % modulus) + modulus) % modulus;

/** The canvas parses any CSS colour; this reads back its channels. */
const parseColour = (context: CanvasRenderingContext2D, value: string): [number, number, number] | null => {
    context.fillStyle = '#000';
    context.fillStyle = value;
    const normalised = String(context.fillStyle);
    const hex = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(normalised);
    if (hex !== null) {
        return [parseInt(hex[1] ?? '0', 16), parseInt(hex[2] ?? '0', 16), parseInt(hex[3] ?? '0', 16)];
    }
    const channels = /^rgba?\((\d+),\s*(\d+),\s*(\d+)/.exec(normalised);
    return channels === null ? null : [Number(channels[1]), Number(channels[2]), Number(channels[3])];
};

type Field = {
    edge: number;
    inner: number;
    shift: number;
    shimmer: number;
    cosA: number;
    sinA: number;
    cosB: number;
    sinB: number;
    cosTwist: number;
    sinTwist: number;
};

class DitherRenderer {
    private readonly canvas: HTMLCanvasElement;
    private image: ImageData;
    private colour: [number, number, number] = [0, 0, 0];
    private radius = new Float32Array(0);
    private distance = new Float32Array(0);
    private cos3 = new Float32Array(0);
    private sin3 = new Float32Array(0);
    private cos5 = new Float32Array(0);
    private sin5 = new Float32Array(0);
    private cos6 = new Float32Array(0);
    private sin6 = new Float32Array(0);
    private layer = new Uint8Array(0);
    private seconds = 0;
    private lastFrame = 0;
    private running = false;
    private frame = 0;
    private timer = 0;
    private onScreen = false;
    private readonly intersection: IntersectionObserver;
    private readonly resize: ResizeObserver;
    private readonly theme: MutationObserver;

    constructor(
        private readonly host: HTMLElement,
        private readonly fallback: HTMLElement,
        private readonly context: CanvasRenderingContext2D,
    ) {
        this.canvas = context.canvas;
        this.image = context.createImageData(1, 1);

        this.intersection = new IntersectionObserver((entries) => {
            const latest = entries.at(-1);
            if (latest !== undefined) {
                this.onScreen = latest.isIntersecting;
                this.sync();
            }
        });
        this.resize = new ResizeObserver(() => this.measure());
        this.theme = new MutationObserver(() => {
            if (this.readColour()) {
                this.paintColour();
                this.draw();
            }
        });

        this.readColour();
        this.host.append(this.canvas);
        this.measure();
        this.intersection.observe(host);
        this.resize.observe(host);
        this.theme.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
        document.addEventListener('visibilitychange', this.sync);
    }

    dispose() {
        this.halt();
        this.intersection.disconnect();
        this.resize.disconnect();
        this.theme.disconnect();
        document.removeEventListener('visibilitychange', this.sync);
        this.canvas.remove();
        delete this.host.dataset.dither;
    }

    // Runs the loop only while someone can see it.
    private readonly sync = () => {
        const shouldRun = this.onScreen && document.visibilityState === 'visible';
        if (shouldRun === this.running) {
            return;
        }
        if (!shouldRun) {
            this.halt();
            return;
        }
        this.running = true;
        this.lastFrame = performance.now();
        this.frame = requestAnimationFrame(this.tick);
    };

    private halt() {
        cancelAnimationFrame(this.frame);
        window.clearTimeout(this.timer);
        this.frame = 0;
        this.timer = 0;
        this.running = false;
    }

    private readonly tick = (now: number) => {
        this.frame = 0;
        const elapsed = now - this.lastFrame;
        // Woken a vsync early (the first frame, a fast timer): the next vsync draws.
        if (elapsed < FRAME_MS - FRAME_SLACK_MS) {
            this.frame = requestAnimationFrame(this.tick);
            return;
        }
        this.lastFrame = now;
        this.seconds += Math.min(elapsed, MAX_STEP_MS) / 1000;
        this.draw();
        // Sleep on a timer between draws, and ask for a frame only shortly before the next is due.
        const wakeAt = now + FRAME_MS - FRAME_SLACK_MS - TIMER_LEAD_MS;
        this.timer = window.setTimeout(this.wake, Math.max(0, wakeAt - performance.now()));
    };

    private readonly wake = () => {
        this.timer = 0;
        this.frame = requestAnimationFrame(this.tick);
    };

    // Read on a theme switch only: reading computed style every frame would force a recalculation.
    private readColour() {
        const colour = parseColour(
            this.context,
            getComputedStyle(this.host).getPropertyValue('--dither').trim(),
        );
        if (colour === null || colour.join() === this.colour.join()) {
            return false;
        }
        this.colour = colour;
        return true;
    }

    private paintColour() {
        const [red, green, blue] = this.colour;
        const data = this.image.data;
        for (let offset = 0; offset < data.length; offset += 4) {
            data[offset] = red;
            data[offset + 1] = green;
            data[offset + 2] = blue;
        }
    }

    // Lays the canvas lattice exactly on the stylesheet's: same 12px phase, same centre and radius
    // as the iris square. Layout sizes, not bounding boxes, give the scale.
    private measure() {
        const width = this.host.offsetWidth;
        const height = this.host.offsetHeight;
        const hostBox = this.host.getBoundingClientRect();
        const irisBox = this.fallback.getBoundingClientRect();
        if (width === 0 || height === 0 || hostBox.width === 0 || irisBox.width === 0) {
            return;
        }

        const scale = hostBox.width / width;
        const irisLeft = (irisBox.left - hostBox.left) / scale;
        const irisTop = (irisBox.top - hostBox.top) / scale;
        const irisRadius = irisBox.width / scale / 2;
        const originX = positiveModulo(irisLeft, TILE) - TILE;
        const originY = positiveModulo(irisTop, TILE) - TILE;
        const columns = Math.ceil((width - originX) / CELL);
        const rows = Math.ceil((height - originY) / CELL);
        const centreX = irisLeft + irisRadius - originX;
        const centreY = irisTop + irisRadius - originY;
        const count = columns * rows;

        this.canvas.width = columns;
        this.canvas.height = rows;
        Object.assign(this.canvas.style, {
            left: `${originX}px`,
            top: `${originY}px`,
            width: `${columns * CELL}px`,
            height: `${rows * CELL}px`,
        });

        this.radius = new Float32Array(count);
        this.distance = new Float32Array(count);
        this.cos3 = new Float32Array(count);
        this.sin3 = new Float32Array(count);
        this.cos5 = new Float32Array(count);
        this.sin5 = new Float32Array(count);
        this.cos6 = new Float32Array(count);
        this.sin6 = new Float32Array(count);
        this.layer = new Uint8Array(count);

        for (let row = 0; row < rows; row++) {
            for (let column = 0; column < columns; column++) {
                const index = row * columns + column;
                const x = column * CELL + DOT / 2 - centreX;
                const y = row * CELL + DOT / 2 - centreY;
                const distance = Math.hypot(x, y);
                const angle = Math.atan2(y, x);
                const oddColumn = column % 2 === 1;
                const oddRow = row % 2 === 1;
                this.distance[index] = distance;
                this.radius[index] = distance / irisRadius;
                this.cos3[index] = Math.cos(3 * angle);
                this.sin3[index] = Math.sin(3 * angle);
                this.cos5[index] = Math.cos(5 * angle);
                this.sin5[index] = Math.sin(5 * angle);
                this.cos6[index] = Math.cos(6 * angle);
                this.sin6[index] = Math.sin(6 * angle);
                this.layer[index] = oddColumn === oddRow ? (oddColumn ? LAYER_RING : LAYER_BASE) : LAYER_CORE;
            }
        }

        this.image = this.context.createImageData(columns, rows);
        this.paintColour();
        this.draw();
        // Only now does the canvas show what the stylesheet showed: hand over.
        this.host.dataset.dither = 'live';
    }

    private draw() {
        const time = this.seconds;
        const breath = Math.sin((2 * Math.PI * time) / BREATH_SECONDS + BREATH_PHASE);
        const edge = CORE_MIDDLE + CORE_SWING * breath;
        const twist = CORE_TWIST * breath;
        const field: Field = {
            edge,
            inner: edge - CORE_FADE,
            shift: (time * RING_SPEED) % RING_PERIOD,
            shimmer: SHIMMER * Math.min(1, time / SHIMMER_FADE_IN_SECONDS),
            cosA: Math.cos(time * SHIMMER_SPEED_A),
            sinA: Math.sin(time * SHIMMER_SPEED_A),
            cosB: Math.cos(time * SHIMMER_SPEED_B),
            sinB: Math.sin(time * SHIMMER_SPEED_B),
            cosTwist: Math.cos(6 * twist),
            sinTwist: Math.sin(6 * twist),
        };
        const data = this.image.data;
        const count = this.radius.length;

        for (let index = 0; index < count; index++) {
            const radius = this.radius[index] ?? 1;
            let alpha = 0;
            if (radius < 1) {
                // The stylesheet's outer mask: full at the centre, 90% at 40%, gone at the edge.
                const envelope = radius < 0.4 ? 1 - 0.25 * radius : (0.9 * (1 - radius)) / 0.6;
                alpha = envelope * this.coverage(index, radius, field);
            }
            data[index * 4 + 3] = alpha * 255;
        }

        this.context.putImageData(this.image, 0, 0);
    }

    // How much of one dot its layer shows, before the outer envelope.
    private coverage(index: number, radius: number, field: Field) {
        switch (this.layer[index]) {
            case LAYER_BASE:
                return 1;
            case LAYER_RING: {
                // The ring mask: full to 40% of the radius, gone at 80%.
                const ringReach = radius < 0.4 ? 1 : Math.max(0, (0.8 - radius) / 0.4);
                if (ringReach === 0) {
                    return 0;
                }
                const phase =
                    positiveModulo((this.distance[index] ?? 0) - field.shift, RING_PERIOD) / RING_PERIOD;
                const offset = Math.abs(phase - RING_CENTRE);
                const fromCentre = offset > 0.5 ? 1 - offset : offset;
                const band =
                    1 -
                    smoothstep(RING_HALF_WIDTH - RING_SOFTNESS, RING_HALF_WIDTH + RING_SOFTNESS, fromCentre);
                // cos(3a + A) and cos(5a - B), expanded so each dot's angle is computed once, not every frame.
                const wave =
                    0.6 * ((this.cos3[index] ?? 0) * field.cosA - (this.sin3[index] ?? 0) * field.sinA) +
                    0.4 * ((this.cos5[index] ?? 0) * field.cosB + (this.sin5[index] ?? 0) * field.sinB);
                return ringReach * band * (1 - field.shimmer * (0.5 - 0.5 * wave));
            }
            default: {
                // cos(6 (a - twist)): a hexagonal radius that turns with the breath.
                const hexagon =
                    1 +
                    CORE_HEXAGON *
                        ((this.cos6[index] ?? 0) * field.cosTwist + (this.sin6[index] ?? 0) * field.sinTwist);
                return Math.min(1, Math.max(0, (field.edge - radius * hexagon) / (field.edge - field.inner)));
            }
        }
    }
}

/**
 * Brings the iris of a host to life while motion is allowed, and gives the still picture back
 * when the preference flips. Returns the cleanup.
 */
export const ditherField = (host: HTMLElement) => {
    const reducedMotion = matchMedia('(prefers-reduced-motion: reduce)');
    let renderer: DitherRenderer | null = null;

    const start = () => {
        const fallback = host.querySelector<HTMLElement>(':scope > .signin-dither-still');
        const canvas = document.createElement('canvas');
        const context = canvas.getContext('2d');
        if (renderer !== null || fallback === null || context === null) {
            return;
        }
        canvas.className = 'signin-dither-canvas';
        canvas.setAttribute('aria-hidden', 'true');
        renderer = new DitherRenderer(host, fallback, context);
    };
    const stop = () => {
        renderer?.dispose();
        renderer = null;
    };
    const follow = (event: MediaQueryListEvent) => (event.matches ? stop() : start());

    if (!reducedMotion.matches) {
        start();
    }
    reducedMotion.addEventListener('change', follow);
    return () => {
        reducedMotion.removeEventListener('change', follow);
        stop();
    };
};
