// The mock GPU Share backend behind the browser preview.
//
// A port of prototype build 0.2 (docs/gpu-share-preview.html): one simulated host (Dara's
// laptop), your PC as a client, and two test-bench friends (Tunde and Ada) who share the same
// card through the queue. It implements `HostBackend` / `ClientBackend` from `../types`, the
// same interfaces the real Tauri backend implements, plus a `sim` object for the test bench,
// which only the preview has.
//
// The policy precedence here is the reference the Rust policy loop (crates/host) was ported
// from; keep them in step if either changes.

import type {
	ClientBackend,
	ClientView,
	HostBackend,
	HostSettings,
	HostView,
	RuntimeStateName
} from '../types';
import { CARD, CATALOG, model, vramFor } from './catalog';
import { FALLBACK, replyFor } from './replies';
import { fmtHour, toTokens, WORDS_PER_TOKEN } from '../format';

export const SIM_MINUTE = 1200; // one simulated minute, in real milliseconds
const IDLE_FREE_MS = 90000; // shortened for the preview
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
export const BENCH_APPS = ['Blender', 'Photoshop', 'Steam', 'Chrome'];

export const HOST_ID = 'dara-laptop';
export const ME = { id: 'ubuntu-pc', name: 'Your PC (Ubuntu)', short: 'You' };
export const FRIENDS = [
	{ id: 'tunde-pc', name: "Tunde's PC", short: 'Tunde' },
	{ id: 'ada-laptop', name: "Ada's laptop", short: 'Ada' }
];
const FRIEND_QS = [
	'How much memory does a 7B model need on a graphics card?',
	'Write a python function that splits a list into chunks',
	'Draft a short email asking for the source files',
	'Explain how a chat model writes its answers'
];

const whoShort = (id: string) =>
	id === ME.id ? 'you' : (FRIENDS.find((f) => f.id === id)?.short ?? 'a friend');

interface WorldState {
	atDesk: boolean;
	awayAt: number;
	apps: string[];
	battery: boolean;
	heat: boolean;
	hour: number;
}

interface Job {
	id: string;
	who: string;
	question: string;
	model: string;
	phase: 'queued' | 'loading' | 'streaming';
	text: string;
	started?: number;
	/** Index into client.msgs of the streaming reply, for your own job. */
	msgIndex?: number;
}

let seq = 0;
const newId = () => 'j' + seq++;

function freshSettings(): HostSettings {
	return {
		welcomed: false,
		resumeAfterRestart: false,
		budget: 6.0,
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
		idleFreeMin: 5,
		disabled: [],
		defaultModel: null
	};
}

function freshHost(): HostView {
	return {
		name: "Dara's laptop",
		card: { name: CARD.name, vramGB: CARD.vram },
		sharing: false,
		settings: freshSettings(),
		state: 'off',
		reason: '',
		clientReason: '',
		countdown: false,
		loaded: null,
		loading: null,
		loadPct: 0,
		vramUsedGB: 0,
		temp: 38,
		job: null,
		queue: [],
		paired: [],
		usage: {},
		pending: null,
		catalog: CATALOG,
		installed: ['llama32-3b', 'qwen25-7b'],
		downloads: {},
		toasts: [],
		runningApps: []
	};
}

function freshWorld(): WorldState {
	return { atDesk: false, awayAt: Date.now() - 999999, apps: [], battery: false, heat: false, hour: 15 };
}

function freshClient(): ClientView {
	return {
		scanning: true,
		hosts: [],
		pairing: null,
		chatHost: null,
		model: 'qwen25-7b',
		msgs: [],
		turn: null,
		networkHint: '',
		notice: ''
	};
}

// ---------- reactive state ----------
const host = $state<HostView>(freshHost());
const world = $state<WorldState>(freshWorld());
const client = $state<ClientView>(freshClient());
const bench = $state({ note: '' });

// Not shown anywhere directly, so plain variables.
let job: Job | null = null;
let queue: Job[] = [];
let tempBlocked = false;
let lastReq = 0;
let tickTimer: ReturnType<typeof setInterval> | null = null;
let streamTimer: ReturnType<typeof setInterval> | null = null;
let loadTimer: ReturnType<typeof setInterval> | null = null;

const s = () => host.settings;

function offered(): string[] {
	return host.installed.filter(
		(id) => !s().disabled.includes(id) && vramFor(model(id)!, s().ctx) <= s().budget
	);
}


function inHours(): boolean {
	const { hoursFrom: f, hoursTo: t } = s();
	const h = world.hour;
	return f <= t ? h >= f && h < t : h >= f || h < t;
}

export function awayLeft(): number {
	if (world.atDesk) return s().awayMin * SIM_MINUTE;
	return Math.max(0, s().awayMin * SIM_MINUTE - (Date.now() - world.awayAt));
}

interface PolicyResult {
	state: RuntimeStateName;
	reason: string;
	clientReason: string;
	countdown?: boolean;
}

/* reason is what Dara sees; clientReason is what friends see.
   Friends never learn which programs he has open. */
function policy(): PolicyResult {
	const h = s(),
		w = world;
	const inUse = "He's using his computer";
	if (!host.sharing) return { state: 'off', reason: '', clientReason: '' };
	if (h.pluggedOnly && w.battery)
		return { state: 'paused', reason: 'Your laptop is running on battery', clientReason: 'His laptop is on battery' };
	if (h.hoursMode === 'schedule' && !inHours())
		return {
			state: 'paused',
			reason: 'Outside your sharing hours, ' + fmtHour(h.hoursFrom) + ' to ' + fmtHour(h.hoursTo),
			clientReason: 'Outside the hours he shares'
		};
	const listed = w.apps.filter((a) => h.pauseApps.includes(a));
	if (listed.length) return { state: 'paused', reason: listed[0] + ' is open', clientReason: inUse };
	if (h.pauseOnGpuUse) {
		const heavy = w.apps.filter((a) => GPU_HEAVY.includes(a));
		if (heavy.length)
			return { state: 'paused', reason: heavy[0] + ' is using your graphics card', clientReason: inUse };
	}
	if (h.pauseWhenInUse && w.atDesk)
		return { state: 'paused', reason: "You're using your computer", clientReason: inUse };
	if (h.pauseWhenInUse && awayLeft() > 0) {
		const n = Math.ceil(awayLeft() / SIM_MINUTE);
		return {
			state: 'paused',
			reason: 'Starts again in ' + n + ' min',
			clientReason: 'He just stepped away. Back in about ' + n + ' min',
			countdown: true
		};
	}
	if (tempBlocked)
		return {
			state: 'paused',
			reason: 'Your graphics card is at ' + Math.round(host.temp ?? 0) + '°C',
			clientReason: 'His graphics card is cooling down'
		};
	if (job) return { state: 'busy', reason: '', clientReason: '' };
	return { state: 'available', reason: '', clientReason: '' };
}
const blockedNow = () => {
	const st = policy().state;
	return st === 'paused' || st === 'off';
};

function toast(title: string, body: string) {
	const id = 't' + seq++;
	host.toasts.push({ id, title, body });
	setTimeout(() => {
		host.toasts = host.toasts.filter((t) => t.id !== id);
	}, 5200);
}

// ---------- mirroring internal state into the views ----------
function syncViews() {
	host.job = job && job.phase !== 'queued' ? { who: job.who, phase: job.phase } : null;
	host.queue = queue.map((j) => j.who);
	host.vramUsedGB = host.loaded ? vramFor(model(host.loaded)!, s().ctx) : host.loading ? vramFor(model(host.loading)!, s().ctx) * (host.loadPct / 100) : 0;
	host.runningApps = [...world.apps];

	const mine = job && job.who === ME.id ? job : null;
	const qi = queue.findIndex((j) => j.who === ME.id);
	client.turn = mine
		? mine.phase === 'loading'
			? { phase: 'loading', model: mine.model, pct: host.loadPct }
			: { phase: 'streaming' }
		: qi >= 0
			? { phase: 'queued', position: qi + (job ? 1 : 0) + 1 }
			: null;

	const pairedMe = host.paired.some((p) => p.id === ME.id);
	client.hosts = host.sharing
		? [
				{
					id: HOST_ID,
					name: host.name,
					os: 'Windows',
					gpu: CARD.name,
					status: host.state,
					reason: host.clientReason,
					busyWithOther: !!job && job.who !== ME.id,
					paired: pairedMe,
					models: pairedMe ? offered().map((id) => ({ id, name: model(id)!.name })) : [],
					address: '192.168.1.23'
				}
			]
		: [];
	if (!pairedMe && client.chatHost) client.chatHost = null;
}

// ---------- policy loop ----------
function tick() {
	// temperature
	let target: number;
	if ((job && job.phase === 'streaming') || host.loading) target = world.heat ? 89 : 74;
	else if (host.loaded) target = world.heat ? 64 : 51;
	else target = world.heat ? 52 : 39;
	if (world.apps.some((a) => GPU_HEAVY.includes(a))) target = Math.max(target, world.heat ? 86 : 69);
	host.temp = (host.temp ?? 38) + (target - (host.temp ?? 38)) * 0.12;
	if (host.temp >= s().tempPause) tempBlocked = true;
	if (host.temp <= s().tempResume) tempBlocked = false;

	const p = policy();
	const wasPaused = host.state === 'paused';
	host.state = p.state;
	host.reason = p.reason;
	host.clientReason = p.clientReason;
	host.countdown = !!p.countdown;

	// pausing or switching off frees the card and turns away everyone waiting
	if ((p.state === 'paused' || p.state === 'off') && (host.loaded || host.loading || job || queue.length)) {
		abortAll(p.state === 'off' ? 'Dara switched sharing off' : p.clientReason);
		host.loaded = null;
		if (p.state === 'paused' && !wasPaused) toast('Sharing paused', p.reason + '. Your graphics card is free again.');
	}
	if (host.loaded && vramFor(model(host.loaded)!, s().ctx) > s().budget) {
		abortAll('The model needed more memory than Dara allows');
		host.loaded = null;
		toast('Model unloaded', 'It needs more memory than your limit allows.');
	}
	// free the card after a quiet spell
	if (host.loaded && !job && !queue.length && Date.now() - lastReq > IDLE_FREE_MS) {
		host.loaded = null;
		toast('Graphics card freed', 'No one has used it for a while, so the model was unloaded.');
	}
	pump();
	syncViews();
}

// ---------- the queue: one answer at a time, first come first served ----------
const hasJob = (id: string) => (job && job.who === id) || queue.some((j) => j.who === id);

function enqueue(who: string, question: string, modelId: string) {
	queue.push({ id: newId(), who, question, model: modelId, phase: 'queued', text: '' });
	lastReq = Date.now();
	pump();
	syncViews();
}

function pump() {
	if (job || !queue.length || blockedNow()) return;
	const next = queue.shift()!;
	job = next;
	if (host.loaded === next.model) startStream(next);
	else startLoad(next);
}

function startLoad(j: Job) {
	const m = model(j.model)!;
	j.phase = 'loading';
	host.loaded = null; // switching models stops the old engine process first
	host.loading = j.model;
	host.loadPct = 0;
	const total = m.fileGB * 1900; // roughly 2.5 GB/s off an NVMe drive
	const t0 = Date.now();
	loadTimer = setInterval(() => {
		if (job !== j) {
			if (loadTimer) clearInterval(loadTimer);
			loadTimer = null;
			return;
		}
		host.loadPct = Math.min(100, Math.round(((Date.now() - t0) / total) * 100));
		if (host.loadPct >= 100) {
			if (loadTimer) clearInterval(loadTimer);
			loadTimer = null;
			host.loading = null;
			host.loaded = j.model;
			startStream(j);
		}
		syncViews();
	}, 120);
}

function startStream(j: Job) {
	const m = model(j.model)!;
	const mine = j.who === ME.id;
	j.phase = 'streaming';
	j.started = Date.now();
	// other friends tend to ask for longer answers, which makes the queue easy to see
	const full = mine ? replyFor(j.question) : replyFor(j.question) + '\n\n' + FALLBACK[1];
	const words = full.split(/(\s+)/);
	let i = 0;
	if (mine) j.msgIndex = client.msgs.push({ role: 'assistant', text: '', streaming: true }) - 1;
	syncViews();
	const perChunk = 1000 / (m.tps * WORDS_PER_TOKEN) / 2; // words and gaps alternate
	streamTimer = setInterval(() => {
		if (job !== j) {
			if (streamTimer) clearInterval(streamTimer);
			streamTimer = null;
			return;
		}
		j.text += words[i];
		i++;
		// Re-read from the reactive array: a reference held from before the push is not the
		// proxied object, so mutating it would not re-render.
		if (j.msgIndex !== undefined) client.msgs[j.msgIndex].text = j.text;
		if (i >= words.length) finishJob(j);
	}, perChunk);
}

function finishJob(j: Job) {
	if (streamTimer) clearInterval(streamTimer);
	streamTimer = null;
	if (j.msgIndex !== undefined) client.msgs[j.msgIndex].streaming = false;
	countUsage(j);
	job = null;
	lastReq = Date.now();
	pump();
	syncViews();
}

function countUsage(j: Job) {
	if (!j.started) return; // never reached the card
	const u = host.usage[j.who] ?? (host.usage[j.who] = { reqs: 0, tokIn: 0, tokOut: 0, gpuSec: 0 });
	const words = (t: string) => t.split(/\s+/).filter(Boolean).length;
	u.reqs++;
	u.tokIn += toTokens(words(j.question));
	u.tokOut += toTokens(words(j.text));
	u.gpuSec += (Date.now() - j.started) / 1000;
}

function clearTimers() {
	if (loadTimer) clearInterval(loadTimer);
	if (streamTimer) clearInterval(streamTimer);
	loadTimer = streamTimer = null;
}

/* cut off whatever is running and turn away everyone waiting */
function abortAll(why: string) {
	clearTimers();
	const turnedAway: string[] = [];
	const notAnswered = "Your question wasn't answered. " + why + ". Send it again when he's back.";
	if (job) {
		const j = job;
		countUsage(j);
		if (j.who === ME.id) {
			if (j.msgIndex !== undefined) {
				client.msgs[j.msgIndex].streaming = false;
				client.msgs.push({ role: 'note', text: "Dara's computer stopped part way through. " + why + '.' });
			} else client.msgs.push({ role: 'note', text: notAnswered });
		} else turnedAway.push(whoShort(j.who));
	}
	for (const j of queue) {
		if (j.who === ME.id) client.msgs.push({ role: 'note', text: notAnswered });
		else turnedAway.push(whoShort(j.who));
	}
	if (turnedAway.length)
		bench.note =
			turnedAway.join(' and ') + (turnedAway.length === 1 ? ' was' : ' were') + ' told the same thing you were.';
	job = null;
	queue = [];
	host.loading = null;
	syncViews();
}

/* you pressed Stop or Cancel */
function stopMine() {
	if (job && job.who === ME.id) {
		const j = job;
		clearTimers();
		countUsage(j);
		if (j.msgIndex !== undefined) client.msgs[j.msgIndex].streaming = false;
		if (j.phase === 'loading') host.loading = null;
		job = null;
		lastReq = Date.now();
		client.msgs.push({
			role: 'note',
			text: j.msgIndex !== undefined ? 'You stopped the answer.' : 'You cancelled your question.'
		});
		pump();
	} else if (queue.some((q) => q.who === ME.id)) {
		queue = queue.filter((q) => q.who !== ME.id);
		client.msgs.push({ role: 'note', text: 'You cancelled your question before it started.' });
	}
	syncViews();
}

// ---------- pairing ----------
function requestPairing(who: { id: string; name: string }): boolean {
	if (host.pending) return false;
	const code = String(Math.floor(100000 + Math.random() * 900000));
	host.pending = { id: who.id, name: who.name, code };
	if (who.id === ME.id) client.pairing = { hostId: HOST_ID, code };
	return true;
}

function unpair(id: string) {
	host.paired = host.paired.filter((p) => p.id !== id);
	queue = queue.filter((j) => j.who !== id);
	if (job && job.who === id) {
		clearTimers();
		countUsage(job);
		if (job.phase === 'loading') host.loading = null;
		job = null;
	}
	if (id === ME.id) {
		client.msgs = [];
		client.chatHost = null;
	}
	pump();
	syncViews();
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
				syncViews();
			}, 1400);
		}
	}, 150);
}

function setSharing(on: boolean) {
	host.sharing = on;
	if (!on) {
		abortAll('Dara switched sharing off');
		host.loaded = null;
		host.pending = null;
		client.pairing = null;
	} else world.awayAt = Math.min(world.awayAt, Date.now());
	tick();
}

// ============================================================
// The two interfaces the screens use
// ============================================================

export const hostBackend: HostBackend = {
	get view() {
		return host;
	},
	offeredModels: offered,
	welcomeDone() {
		host.settings.welcomed = true;
	},
	setSharing,
	updateSettings(patch) {
		Object.assign(host.settings, patch);
		syncViews();
	},
	modelAdd: startDownload,
	modelToggle(id) {
		const d = s().disabled;
		s().disabled = d.includes(id) ? d.filter((x) => x !== id) : d.concat(id);
		syncViews();
	},
	modelRemove(id) {
		if (host.loaded === id && !job) host.loaded = null;
		host.installed = host.installed.filter((x) => x !== id);
		syncViews();
	},
	modelSetDefault(id) {
		s().defaultModel = id;
	},
	async modelAddLink() {
		return 'In the finished app this adds a model from a Hugging Face link, checked like the others.';
	},
	unpair,
	pairAllow() {
		const p = host.pending;
		if (!p) return;
		host.paired.push({ id: p.id, name: p.name });
		host.usage[p.id] = host.usage[p.id] ?? { reqs: 0, tokIn: 0, tokOut: 0, gpuSec: 0 };
		host.pending = null;
		if (p.id === ME.id) {
			client.pairing = null;
			client.chatHost = HOST_ID;
			openChatHook?.();
		}
		toast('Connected', p.name + ' can now ask questions.');
		syncViews();
	},
	pairDeny() {
		const p = host.pending;
		host.pending = null;
		if (p && p.id === ME.id) client.pairing = null;
		else if (p) bench.note = p.name + ' was turned away.';
	},
	quit() {
		setSharing(false);
		toast('GPU Share closed', 'Nothing is running until you open it again.');
	}
};

/** Lets the preview switch the client to its Chat tab when pairing succeeds. */
let openChatHook: (() => void) | null = null;
export function onPaired(fn: () => void) {
	openChatHook = fn;
}

export const clientBackend: ClientBackend = {
	get view() {
		return client;
	},
	askConnect() {
		requestPairing(ME);
	},
	async addByIp() {
		return 'In the finished app this connects to an address like 192.168.1.42, for networks where computers cannot find each other automatically.';
	},
	rescan() {},
	openChat(hostId) {
		client.chatHost = hostId;
	},
	sendMessage(text) {
		const t = text.trim();
		if (!t || hasJob(ME.id)) return;
		const offeredNow = offered();
		if (!offeredNow.includes(client.model)) client.model = offeredNow[0];
		client.msgs.push({ role: 'user', text: t });
		enqueue(ME.id, t, client.model);
	},
	stop: stopMine,
	retry() {
		const last = [...client.msgs].reverse().find((m) => m.role === 'user');
		if (last && !hasJob(ME.id)) enqueue(ME.id, last.text, client.model);
	},
	setModel(id) {
		client.model = id;
	},
	forget(hostId) {
		if (hostId === HOST_ID) unpair(ME.id);
	}
};

// ============================================================
// Test bench: pretend real life around Dara's laptop (preview only)
// ============================================================

export const sim = {
	get world() {
		return world;
	},
	get bench() {
		return bench;
	},
	setAtDesk(v: boolean) {
		world.atDesk = v;
		if (!v) world.awayAt = Date.now();
	},
	toggleApp(app: string) {
		world.apps = world.apps.includes(app) ? world.apps.filter((x) => x !== app) : world.apps.concat(app);
		tick();
	},
	setBattery(v: boolean) {
		world.battery = v;
	},
	setHeat(v: boolean) {
		world.heat = v;
	},
	setHour(h: number) {
		world.hour = h;
	},
	/** What each bench friend can do right now. */
	friendActions() {
		return FRIENDS.map((f) => {
			const paired = host.paired.some((p) => p.id === f.id);
			if (!paired) {
				const waiting = host.pending?.id === f.id;
				return {
					f,
					act: 'connect' as const,
					label: waiting ? f.short + ' is waiting for Dara' : f.short + ' asks to connect',
					off: waiting || !!host.pending || !host.sharing
				};
			}
			const busy = hasJob(f.id);
			return {
				f,
				act: 'ask' as const,
				label: busy ? f.short + "'s question is in" : f.short + ' asks a question',
				off: busy
			};
		});
	},
	friendDo(id: string, act: 'connect' | 'ask') {
		const f = FRIENDS.find((x) => x.id === id)!;
		bench.note = '';
		if (act === 'connect') {
			if (!requestPairing(f)) bench.note = 'Dara is already looking at another request.';
			return;
		}
		const p = policy();
		const models = offered();
		if (p.state === 'paused' || p.state === 'off')
			bench.note = f.short + ' was told: ' + (p.clientReason || 'sharing is off') + '.';
		else if (!models.length) bench.note = "Dara isn't offering any models, so " + f.short + " can't ask.";
		else
			enqueue(
				f.id,
				FRIEND_QS[Math.floor(Math.random() * FRIEND_QS.length)],
				models[Math.floor(Math.random() * models.length)]
			);
	},
	/** Who is being answered and who is waiting, for the bench readout. */
	lineup() {
		const parts: string[] = [];
		if (host.job) parts.push('Answering ' + whoShort(host.job.who));
		if (host.queue.length) parts.push('waiting: ' + host.queue.map(whoShort).join(', '));
		return parts.length ? parts.join(', ') + '.' : 'No one is asking right now.';
	},
	reset() {
		clearTimers();
		job = null;
		queue = [];
		tempBlocked = false;
		lastReq = 0;
		Object.assign(host, freshHost());
		Object.assign(world, freshWorld());
		Object.assign(client, freshClient());
		bench.note = '';
		start();
	}
};

/** Starts the simulation. Called by the preview shell, never inside the real app. */
export function start() {
	if (tickTimer) clearInterval(tickTimer);
	tickTimer = setInterval(tick, 500);
	tick();
}
