// The real backend: the Rust side of the Tauri app pushes `host-view` / `client-view` events and
// exposes the commands in src-tauri/src/commands.rs. Same interfaces as the mock.

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { ClientBackend, ClientView, HostBackend, HostView } from '../types';
import { ui } from '../ui.svelte';

function emptyHost(): HostView {
	return {
		name: '',
		card: null,
		sharing: false,
		settings: {
			welcomed: true,
			resumeAfterRestart: false,
			budget: 6,
			ctx: 4096,
			pauseWhenInUse: true,
			awayMin: 5,
			pauseApps: [],
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
		},
		state: 'off',
		reason: '',
		clientReason: '',
		countdown: false,
		loaded: null,
		loading: null,
		loadPct: 0,
		vramUsedGB: 0,
		temp: null,
		job: null,
		queue: [],
		paired: [],
		usage: {},
		pending: null,
		catalog: [],
		installed: [],
		downloads: {},
		toasts: [],
		runningApps: [],
		networkWarning: null,
		engineSetup: null,
		idleKnown: true,
		memoryGB: null
	};
}

const host = $state<HostView>(emptyHost());
const client = $state<ClientView>({
	scanning: true,
	hosts: [],
	pairing: null,
	chatHost: null,
	model: '',
	msgs: [],
	turn: null,
	networkHint: '',
	notice: ''
});
/** Whether this computer can share (an engine was found). */
export const capability = $state({ canShare: false, ready: false });

function call(cmd: string, args?: Record<string, unknown>) {
	invoke(cmd, args).catch((e) => console.error(cmd, e));
}

let connected = false;
/** Starts listening for views. Called once by the app shell. */
export async function connect() {
	if (connected) return;
	connected = true;
	await listen<string>('host-view', (e) => {
		const v = JSON.parse(e.payload) as HostView | null;
		capability.canShare = !!v;
		if (v) Object.assign(host, v);
	});
	await listen<string>('client-view', (e) => Object.assign(client, JSON.parse(e.payload) as ClientView));
	const [h, c] = await Promise.all([invoke<HostView | null>('host_view'), invoke<ClientView>('client_view')]);
	capability.canShare = !!h;
	if (h) Object.assign(host, h);
	Object.assign(client, c);
	// Open on Share mode where sharing is the point (a graphics card, or set up already); on a
	// Linux computer without a card that hasn't shared yet, open on "Chat".
	ui.mode = h && (h.card || h.sharing || h.settings.welcomed) ? 'share' : 'use';
	capability.ready = true;
}

export const hostBackend: HostBackend = {
	get view() {
		return host;
	},
	offeredModels() {
		const s = host.settings;
		return host.installed.filter((id) => {
			const m = host.catalog.find((c) => c.id === id);
			return m && !s.disabled.includes(id) && m.fileGB + m.kvGB * (s.ctx / 4096) + 0.5 <= s.budget;
		});
	},
	welcomeDone: () => call('host_welcome_done'),
	setSharing: (on) => call('host_set_sharing', { on }),
	updateSettings(patch) {
		Object.assign(host.settings, patch); // feels instant; the next view confirms it
		call('host_update_settings', { patch });
	},
	modelAdd: (id) => call('host_model_add', { id }),
	modelToggle: (id) => call('host_model_toggle', { id }),
	modelRemove: (id) => call('host_model_remove', { id }),
	modelSetDefault: (id) => call('host_model_set_default', { id }),
	modelAddLink: (url) => invoke<string | null>('host_model_add_link', { url }),
	unpair: (id) => call('host_unpair', { id }),
	pairAllow: () => call('host_pair_allow'),
	pairDeny: () => call('host_pair_deny'),
	quit: () => call('app_quit')
};

export const clientBackend: ClientBackend = {
	get view() {
		return client;
	},
	askConnect: (hostId) => call('client_ask_connect', { hostId }),
	addByIp: (address) => invoke<string | null>('client_add_by_ip', { address }),
	rescan: () => call('client_rescan'),
	openChat: (hostId) => call('client_open_chat', { hostId }),
	sendMessage: (text) => call('client_send', { text }),
	stop: () => call('client_stop'),
	retry: () => call('client_retry'),
	setModel(id) {
		client.model = id;
		call('client_set_model', { id });
	},
	forget: (hostId) => call('client_forget', { hostId })
};
