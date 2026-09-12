// Shared types for the GPU Share UI and its mock backend.
// These mirror the state shape used by the Phase 1 prototype (docs/gpu-share-preview.html)
// so that a future real backend (Tauri commands / gatekeeper HTTP API) can implement the
// same shapes without the Svelte components changing.

export type HostTab = 'share' | 'limits' | 'models' | 'access';
export type ClientTab = 'devices' | 'chat';

export type RuntimeStateName = 'off' | 'available' | 'busy' | 'paused';

export interface CatalogEntry {
	id: string;
	name: string;
	quant: string;
	fileGB: number;
	kvGB: number;
	tps: number; // tokens (words) per second on the reference card, 0 = does not fit
	desc: string;
	license: string;
}

export interface DownloadProgress {
	pct: number;
	stage: string;
}

export interface PairedDevice {
	id: string;
	name: string;
}

export interface PendingPairing {
	id: string;
	name: string;
	code: string;
}

export interface UsageStats {
	reqs: number;
	tokIn: number;
	tokOut: number;
	gpuSec: number;
}

export interface HostState {
	welcomed: boolean;
	tab: HostTab;
	sharing: boolean;
	resumeAfterRestart: boolean;
	budget: number; // GB of VRAM the app may use
	ctx: number; // context length in "words"
	pauseWhenInUse: boolean;
	awayMin: number;
	pauseApps: string[];
	pauseOnGpuUse: boolean;
	tempPause: number;
	tempResume: number;
	pluggedOnly: boolean;
	hoursMode: 'any' | 'schedule';
	hoursFrom: number;
	hoursTo: number;
	maxReply: number;
	installed: string[];
	disabled: string[];
	downloads: Record<string, DownloadProgress>;
	paired: PairedDevice[];
	pending: PendingPairing | null;
	usage: Record<string, UsageStats>;
	trayOpen: boolean;
	toasts: Toast[];
}

export interface Toast {
	id: string;
	title: string;
	body: string;
}

export interface WorldState {
	atDesk: boolean;
	awayAt: number;
	apps: string[];
	battery: boolean;
	heat: boolean;
	hour: number;
}

export interface RuntimeState {
	state: RuntimeStateName;
	reason: string;
	countdown?: boolean;
	loaded: string | null;
	loading: string | null;
	loadPct?: number;
	temp: number;
	tempBlocked: boolean;
	lastReq: number;
	streaming: boolean;
	aborted: boolean;
	streamStart?: number;
	streamQ?: string;
}

export type ChatRole = 'user' | 'assistant' | 'note';

export interface ChatMessage {
	role: ChatRole;
	text: string;
	streaming?: boolean;
}

export interface ClientState {
	tab: ClientTab;
	scanning: boolean;
	paired: boolean;
	model: string;
	msgs: ChatMessage[];
	waitingCode: string | null;
	wantChat: boolean;
}

export interface PolicyResult {
	state: RuntimeStateName;
	reason: string;
	countdown?: boolean;
}
