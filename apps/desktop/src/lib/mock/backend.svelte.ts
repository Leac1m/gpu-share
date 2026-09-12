// The mock GPU Share backend.
//
// This module is the single place that owns the fake policy loop, fake timers,
// fake pairing and fake streaming replies for the Phase 1 prototype port. It exposes
// two small interfaces, `HostBackend` and `ClientBackend`, which is the boundary a real
// Phase 2+ implementation (Tauri commands talking to the engine supervisor on the host,
// and the gatekeeper HTTP API on the client) should implement in place of this file —
// the .svelte components only ever talk to `hostBackend` / `clientBackend`, never to the
// state shape or timers directly, so a real backend can be swapped in without touching
// the components.
//
// The simulated behavior (policy precedence, timings, catalog, reply matching, pairing)
// is ported as closely as possible from docs/gpu-share-preview.html so the two behave
// identically; see that file's `policy()`/`tick()` for the reference implementation.

import type {
	ClientState,
	ClientTab,
	HostState,
	HostTab,
	PolicyResult,
	RuntimeState,
	WorldState
} from '../types';
import { CARD, CATALOG, model, vramFor } from './catalog';
import { replyFor } from './replies';

export const SIM_MINUTE = 1200; // one simulated minute, in real milliseconds
export const IDLE_FREE_MS = 90000; // shortened for the preview
export const GPU_HEAVY = [
	'Blender',
	'DaVinci Resolve',
	'Photoshop',
	'Premiere Pro',
	'After Effects',
	'Unreal Engine',
	'Cinema 4D',
	'Steam',
	'OBS Studio'
];
export const APPS_ALL = [...GPU_HEAVY];
export const BENCH_APPS = ['Blender', 'Photoshop', 'Steam', 'Chrome'];

export { CATALOG, CARD, model, vramFor };

let toastSeq = 0;

function freshHost(): HostState {
	return {
		welcomed: false,
		tab: 'share',
		sharing: false,
		resumeAfterRestart: false,
		budget: 6.5,
		ctx: 4096,
		pauseWhenInUse: true,
		awayMin: 5,
		pauseApps: ['Blender', 'DaVinci Resolve', 'Steam'],
		pauseOnGpuUse: true,
		tempPause: 83,
		tempResume: 75,
		pluggedOnly: true,
		hoursMode: 'any',
		hoursFrom: 19,
		hoursTo: 8,
		maxReply: 1024,
		installed: ['llama32-3b', 'qwen25-7b'],
		disabled: [],
		downloads: {},
		paired: [],
		pending: null,
		usage: {},
		trayOpen: false,
		toasts: []
	};
}

function freshWorld(): WorldState {
	return {
		atDesk: false,
		awayAt: Date.now() - 999999,
		apps: [],
		battery: false,
		heat: false,
		hour: 15
	};
}

function freshRt(): RuntimeState {
	return {
		state: 'off',
		reason: '',
		loaded: null,
		loading: null,
		temp: 38,
		tempBlocked: false,
		lastReq: 0,
		streaming: false,
		aborted: false
	};
}

function freshClient(): ClientState {
	return {
		tab: 'devices',
		scanning: true,
		paired: false,
		model: 'qwen25-7b',
		msgs: [],
		waitingCode: null,
		wantChat: false
	};
}

// ---------- reactive state ----------
let host = $state<HostState>(freshHost());
let world = $state<WorldState>(freshWorld());
let rt = $state<RuntimeState>(freshRt());
let client = $state<ClientState>(freshClient());

let tickTimer: ReturnType<typeof setInterval> | null = null;
let streamTimer: ReturnType<typeof setInterval> | null = null;
let loadTimer: ReturnType<typeof setInterval> | null = null;

function offered(): string[] {
	return host.installed.filter(
		(id) => !host.disabled.includes(id) && vramFor(model(id)!, host.ctx) <= host.budget
	);
}

function fmtH(h: number): string {
	return (h % 12 === 0 ? 12 : h % 12) + (h < 12 ? 'am' : 'pm');
}

function inHours(): boolean {
	const { hoursFrom: f, hoursTo: t } = host;
	const h = world.hour;
	return f <= t ? h >= f && h < t : h >= f || h < t;
}

function awayLeft(): number {
	if (world.atDesk) return host.awayMin * SIM_MINUTE;
	return Math.max(0, host.awayMin * SIM_MINUTE - (Date.now() - world.awayAt));
}

function policy(): PolicyResult {
	const h = host,
		w = world;
	if (!h.sharing) return { state: 'off', reason: '' };
	if (h.pluggedOnly && w.battery) return { state: 'paused', reason: 'Your laptop is running on battery' };
	if (h.hoursMode === 'schedule' && !inHours())
		return {
			state: 'paused',
			reason: 'Outside your sharing hours, ' + fmtH(h.hoursFrom) + ' to ' + fmtH(h.hoursTo)
		};
	const listed = w.apps.filter((a) => h.pauseApps.includes(a));
	if (listed.length) return { state: 'paused', reason: listed[0] + ' is open' };
	if (h.pauseOnGpuUse) {
		const heavy = w.apps.filter((a) => GPU_HEAVY.includes(a));
		if (heavy.length) return { state: 'paused', reason: heavy[0] + ' is using your graphics card' };
	}
	if (h.pauseWhenInUse && w.atDesk) return { state: 'paused', reason: "You're using your computer" };
	if (h.pauseWhenInUse && awayLeft() > 0)
		return {
			state: 'paused',
			reason: 'Starts again in ' + Math.ceil(awayLeft() / SIM_MINUTE) + ' min',
			countdown: true
		};
	if (rt.tempBlocked) return { state: 'paused', reason: 'Your graphics card is at ' + Math.round(rt.temp) + '°C' };
	if (rt.streaming || rt.loading) return { state: 'busy', reason: '' };
	return { state: 'available', reason: '' };
}

function toast(title: string, body: string) {
	const id = 't' + toastSeq++;
	host.toasts.push({ id, title, body });
	setTimeout(() => {
		host.toasts = host.toasts.filter((t) => t.id !== id);
	}, 5200);
}

function tick() {
	// temperature
	let target = 40;
	if (rt.streaming || rt.loading) target = world.heat ? 89 : 74;
	else if (rt.loaded) target = world.heat ? 64 : 51;
	else target = world.heat ? 52 : 39;
	if (world.apps.some((a) => GPU_HEAVY.includes(a))) target = Math.max(target, world.heat ? 86 : 69);
	rt.temp += (target - rt.temp) * 0.12;
	if (rt.temp >= host.tempPause) rt.tempBlocked = true;
	if (rt.temp <= host.tempResume) rt.tempBlocked = false;

	const p = policy();
	const wasPaused = rt.state === 'paused';
	rt.state = p.state;
	rt.reason = p.reason;
	rt.countdown = !!p.countdown;

	// pausing or switching off frees the card
	if ((p.state === 'paused' || p.state === 'off') && (rt.loaded || rt.loading || rt.streaming)) {
		stopStreamInternal(true);
		rt.loaded = null;
		rt.loading = null;
		if (loadTimer) {
			clearInterval(loadTimer);
			loadTimer = null;
		}
		if (p.state === 'paused' && !wasPaused) toast('Sharing paused', p.reason + '. Your graphics card is free again.');
	}
	if (rt.loaded && vramFor(model(rt.loaded)!, host.ctx) > host.budget) {
		stopStreamInternal(true);
		rt.loaded = null;
		toast('Model unloaded', 'It needs more memory than your limit allows.');
	}
	// free the card after a quiet spell
	if (rt.loaded && !rt.streaming && !rt.loading && Date.now() - rt.lastReq > IDLE_FREE_MS) {
		rt.loaded = null;
		toast('Graphics card freed', 'No one has used it for a while, so the model was unloaded.');
	}
}

function startTickLoop() {
	if (tickTimer) clearInterval(tickTimer);
	tickTimer = setInterval(tick, 500);
	tick();
}

// ---------- pairing ----------
function askConnect() {
	const code = String(Math.floor(100000 + Math.random() * 900000));
	client.waitingCode = code;
	host.pending = { id: 'ubuntu-pc', name: 'Your PC (Ubuntu)', code };
}

function pairAllow() {
	host.paired = [{ id: 'ubuntu-pc', name: 'Your PC (Ubuntu)' }];
	host.usage['ubuntu-pc'] = { reqs: 0, tokIn: 0, tokOut: 0, gpuSec: 0 };
	host.pending = null;
	client.waitingCode = null;
	client.paired = true;
	client.tab = 'chat';
	toast('Connected', 'Your PC (Ubuntu) can now ask questions.');
}

function pairDeny() {
	host.pending = null;
	client.waitingCode = null;
}

function unpair(_id: string) {
	host.paired = [];
	client.paired = false;
	client.msgs = [];
	client.tab = 'devices';
	stopStreamInternal(false);
	rt.loaded = null;
}

// ---------- chat / streaming ----------
function countUsage(started: number, question: string, written: string) {
	const u =
		host.usage['ubuntu-pc'] ??
		(host.usage['ubuntu-pc'] = { reqs: 0, tokIn: 0, tokOut: 0, gpuSec: 0 });
	u.reqs++;
	u.tokIn += question.split(/\s+/).filter(Boolean).length;
	u.tokOut += written.split(/\s+/).filter(Boolean).length;
	u.gpuSec += (Date.now() - started) / 1000;
}

function stopStreamInternal(silent: boolean) {
	if (!rt.streaming) return;
	const partial = client.msgs[client.msgs.length - 1];
	countUsage(rt.streamStart ?? Date.now(), rt.streamQ ?? '', partial && partial.text ? partial.text : '');
	rt.streaming = false;
	if (streamTimer) {
		clearInterval(streamTimer);
		streamTimer = null;
	}
	const msg = client.msgs[client.msgs.length - 1];
	if (msg) msg.streaming = false;
	if (silent)
		client.msgs.push({
			role: 'note',
			text: "Dara's computer stopped part way through: " + (rt.reason || 'sharing was switched off') + '.'
		});
}

function startStream(question: string) {
	const m = model(rt.loaded)!;
	rt.streamStart = Date.now();
	rt.streamQ = question;
	const full = replyFor(question);
	const words = full.split(/(\s+)/);
	let i = 0;
	const msgIndex = client.msgs.push({ role: 'assistant', text: '', streaming: true }) - 1;
	rt.streaming = true;
	rt.aborted = false;
	const started = Date.now();
	const perChunk = 1000 / (m.tps / 1.35) / 2; // words and gaps alternate
	streamTimer = setInterval(() => {
		if (!rt.streaming) {
			if (streamTimer) clearInterval(streamTimer);
			streamTimer = null;
			return;
		}
		// re-fetch from the reactive array each tick: mutating a captured reference to the
		// object that was pushed does not trigger reactivity once it's inside a $state array.
		const msg = client.msgs[msgIndex];
		if (!msg) return;
		msg.text += words[i];
		i++;
		if (i >= words.length) {
			if (streamTimer) clearInterval(streamTimer);
			streamTimer = null;
			finishStream(started, full, question);
		}
	}, perChunk);
}

function finishStream(started: number, full: string, question: string) {
	const msg = client.msgs[client.msgs.length - 1];
	if (msg) msg.streaming = false;
	rt.streaming = false;
	rt.lastReq = Date.now();
	countUsage(started, question, full);
}

function startLoad(id: string, then: () => void) {
	const m = model(id)!;
	rt.loading = id;
	rt.loadPct = 0;
	const total = m.fileGB * 1900; // roughly 2.5 GB/s off an NVMe drive
	const t0 = Date.now();
	loadTimer = setInterval(() => {
		if (rt.loading !== id) {
			if (loadTimer) clearInterval(loadTimer);
			loadTimer = null;
			return;
		}
		const pct = Math.min(100, Math.round(((Date.now() - t0) / total) * 100));
		rt.loadPct = pct;
		if (pct >= 100) {
			if (loadTimer) clearInterval(loadTimer);
			loadTimer = null;
			rt.loading = null;
			rt.loaded = id;
			then();
		}
	}, 120);
}

function sendMessage(text: string) {
	const trimmed = text.trim();
	if (!trimmed) return;
	client.msgs.push({ role: 'user', text: trimmed });
	rt.lastReq = Date.now();
	const need = client.model;
	if (rt.loaded !== need) startLoad(need, () => startStream(trimmed));
	else startStream(trimmed);
}

// ---------- downloads ----------
function startDownload(id: string) {
	const m = model(id)!;
	host.downloads[id] = { pct: 0, stage: 'Starting…' };
	const t0 = Date.now();
	const total = m.fileGB * 1000 * 2.2; // about 45 MB/s on home broadband, shortened
	const iv = setInterval(() => {
		const pct = Math.min(100, ((Date.now() - t0) / total) * 100);
		const mb = ((m.fileGB * 1024 * pct) / 100).toFixed(0);
		host.downloads[id] = {
			pct,
			stage: pct < 100 ? `${mb} MB of ${(m.fileGB * 1024).toFixed(0)} MB · 45 MB/s` : 'Checking the file…'
		};
		if (pct >= 100) {
			clearInterval(iv);
			setTimeout(() => {
				delete host.downloads[id];
				host.installed.push(id);
				toast('Model added', m.name + ' is ready and checked.');
			}, 1400);
		}
	}, 150);
}

// ---------- setters shared by host actions ----------
function setSharing(on: boolean) {
	host.sharing = on;
	if (!on) {
		stopStreamInternal(true);
		rt.loaded = null;
		rt.loading = null;
		client.waitingCode = null;
	} else {
		world.awayAt = Math.min(world.awayAt, Date.now());
	}
}

// ============================================================
// Public interfaces — the boundary a real backend implements
// ============================================================

export interface HostBackend {
	readonly host: HostState;
	readonly world: WorldState;
	readonly rt: RuntimeState;
	awayLeft(): number;
	fmtHour(h: number): string;
	offeredModels(): string[];
	welcomeDone(): void;
	setTab(tab: HostTab): void;
	setSharing(on: boolean): void;
	toggleBoolField(key: 'pauseWhenInUse' | 'pauseOnGpuUse' | 'pluggedOnly' | 'resumeAfterRestart'): void;
	setBudget(v: number): void;
	setCtx(v: number): void;
	setMaxReply(v: number): void;
	setAwayMin(v: number): void;
	setTempPause(v: number): void;
	setHoursMode(v: 'any' | 'schedule'): void;
	addPauseApp(app: string): void;
	removePauseApp(app: string): void;
	modelAdd(id: string): void;
	modelToggle(id: string): void;
	modelRemove(id: string): void;
	unpair(id: string): void;
	pairAllow(): void;
	pairDeny(): void;
	trayToggle(): void;
	trayShare(): void;
	traySettings(): void;
	trayQuit(): void;
	// test bench (simulated real-world signals)
	simSetAtDesk(v: boolean): void;
	simToggleApp(app: string): void;
	simSetBattery(v: boolean): void;
	simSetHeat(v: boolean): void;
	simSetHour(h: number): void;
	reset(): void;
}

export interface ClientBackend {
	readonly client: ClientState;
	readonly rt: RuntimeState;
	readonly host: HostState;
	offeredModels(): string[];
	setTab(tab: ClientTab): void;
	askConnect(): void;
	openChat(): void;
	sendMessage(text: string): void;
	stopStream(): void;
	setModel(id: string): void;
}

export const hostBackend: HostBackend = {
	get host() {
		return host;
	},
	get world() {
		return world;
	},
	get rt() {
		return rt;
	},
	awayLeft,
	fmtHour: fmtH,
	offeredModels: offered,
	welcomeDone() {
		host.welcomed = true;
	},
	setTab(tab) {
		host.tab = tab;
		host.welcomed = true;
	},
	setSharing,
	toggleBoolField(key) {
		host[key] = !host[key];
	},
	setBudget(v) {
		host.budget = v;
	},
	setCtx(v) {
		host.ctx = v;
	},
	setMaxReply(v) {
		host.maxReply = v;
	},
	setAwayMin(v) {
		host.awayMin = v;
	},
	setTempPause(v) {
		host.tempPause = v;
		host.tempResume = v - 8;
	},
	setHoursMode(v) {
		host.hoursMode = v;
	},
	addPauseApp(app) {
		if (app && !host.pauseApps.includes(app)) host.pauseApps.push(app);
	},
	removePauseApp(app) {
		host.pauseApps = host.pauseApps.filter((x) => x !== app);
	},
	modelAdd(id) {
		startDownload(id);
	},
	modelToggle(id) {
		host.disabled = host.disabled.includes(id) ? host.disabled.filter((x) => x !== id) : host.disabled.concat(id);
	},
	modelRemove(id) {
		if (rt.loaded === id) rt.loaded = null;
		host.installed = host.installed.filter((x) => x !== id);
	},
	unpair,
	pairAllow,
	pairDeny,
	trayToggle() {
		host.trayOpen = !host.trayOpen;
	},
	trayShare() {
		host.trayOpen = false;
		setSharing(!host.sharing);
	},
	traySettings() {
		host.trayOpen = false;
		host.tab = 'limits';
		host.welcomed = true;
	},
	trayQuit() {
		host.trayOpen = false;
		setSharing(false);
		toast('GPU Share closed', 'Nothing is running until you open it again.');
	},
	simSetAtDesk(v) {
		world.atDesk = v;
		if (!v) world.awayAt = Date.now();
	},
	simToggleApp(app) {
		world.apps = world.apps.includes(app) ? world.apps.filter((x) => x !== app) : world.apps.concat(app);
	},
	simSetBattery(v) {
		world.battery = v;
	},
	simSetHeat(v) {
		world.heat = v;
	},
	simSetHour(h) {
		world.hour = h;
	},
	reset() {
		resetAll();
	}
};

export const clientBackend: ClientBackend = {
	get client() {
		return client;
	},
	get rt() {
		return rt;
	},
	get host() {
		return host;
	},
	offeredModels: offered,
	setTab(tab) {
		client.tab = tab;
	},
	askConnect,
	openChat() {
		client.tab = 'chat';
	},
	sendMessage,
	stopStream() {
		stopStreamInternal(false);
		client.msgs.push({ role: 'note', text: 'You stopped the answer.' });
	},
	setModel(id) {
		client.model = id;
	}
};

function resetAll() {
	if (streamTimer) {
		clearInterval(streamTimer);
		streamTimer = null;
	}
	if (loadTimer) {
		clearInterval(loadTimer);
		loadTimer = null;
	}
	host = Object.assign(host, freshHost());
	world = Object.assign(world, freshWorld());
	rt = Object.assign(rt, freshRt());
	client = Object.assign(client, freshClient());
	startTickLoop();
}

startTickLoop();
