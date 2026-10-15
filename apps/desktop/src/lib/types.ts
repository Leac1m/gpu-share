// The boundary between the Svelte screens and whatever drives them.
//
// Two backends implement it: the mock in `mock/backend.svelte.ts` (the browser preview,
// prototype build 0.2) and the real one in `real/` (Tauri commands and events). Screens only
// read `HostView` / `ClientView` and call the action interfaces, so either can be swapped in
// without touching a component.

export type RuntimeStateName = 'off' | 'available' | 'busy' | 'paused';

/** A model the host can run. Mirrors `protocol::models::CatalogEntry` plus display fields. */
export interface CatalogEntry {
	id: string;
	name: string;
	quant: string;
	fileGB: number;
	/** KV cache size at 4,096 tokens of context, in GB. Scales linearly with context. */
	kvGB: number;
	/** Estimated tokens per second on the reference card; 0 = does not fit. */
	tps: number;
	desc: string;
	license: string;
}

export interface DownloadProgress {
	pct: number;
	stage: string;
}

export interface UsageStats {
	reqs: number;
	tokIn: number;
	tokOut: number;
	gpuSec: number;
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

export interface Toast {
	id: string;
	title: string;
	body: string;
}

/** The owner's limits (spec §6.4). Persisted on the host. */
export interface HostSettings {
	welcomed: boolean;
	resumeAfterRestart: boolean;
	/** GB of VRAM the app may use. */
	budget: number;
	/** Context size in tokens, passed to the engine as `-c`. */
	ctx: number;
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
	/** Max reply length in tokens; the gatekeeper caps `max_tokens` to this. */
	maxReply: number;
	/** Minutes without requests before the engine is stopped. */
	idleFreeMin: number;
	/** Models switched off by the owner (hidden from friends). */
	disabled: string[];
	/** The model friends' pickers start on. */
	defaultModel: string | null;
}

export type JobPhase = 'queued' | 'loading' | 'streaming';

/** What the host's Share screen shows. Never contains prompt or reply text. */
export interface HostView {
	/** Name shown to friends, e.g. "Dara's laptop". */
	name: string;
	/** null when no supported GPU was found. */
	card: { name: string; vramGB: number } | null;
	sharing: boolean;
	settings: HostSettings;
	state: RuntimeStateName;
	/** Exact reason, for the owner only ("Blender is open"). */
	reason: string;
	/** General reason friends see ("He's using his computer"). */
	clientReason: string;
	countdown: boolean;
	loaded: string | null;
	loading: string | null;
	loadPct: number;
	vramUsedGB: number;
	/** null when temperature can't be read. */
	temp: number | null;
	/** The request being answered: which device and how far along. */
	job: { who: string; phase: 'loading' | 'streaming' } | null;
	/** Device ids waiting in line, first in line first. */
	queue: string[];
	paired: PairedDevice[];
	usage: Record<string, UsageStats>;
	pending: PendingPairing | null;
	catalog: CatalogEntry[];
	installed: string[];
	downloads: Record<string, DownloadProgress>;
	toasts: Toast[];
	/** Apps currently running, offered when adding to the pause list. */
	runningApps: string[];
	/** Set when the network is configured so friends can't connect (Windows "Public" network). */
	networkWarning: string | null;
	/** First-run engine download, while it's in progress. */
	engineSetup: DownloadProgress | null;
	/** Whether this computer can tell when the owner is using it (not on every Linux desktop). */
	idleKnown: boolean;
	/** Total RAM in GB: what a host without a graphics card answers from. */
	memoryGB: number | null;
}

export interface HostBackend {
	readonly view: HostView;
	/** Models friends can pick: installed, enabled and within the memory budget. */
	offeredModels(): string[];
	welcomeDone(): void;
	setSharing(on: boolean): void;
	updateSettings(patch: Partial<HostSettings>): void;
	modelAdd(id: string): void;
	modelToggle(id: string): void;
	modelRemove(id: string): void;
	/** The model friends' pickers start on. */
	modelSetDefault(id: string): void;
	/** Add from a Hugging Face link. Resolves to an error message, or null once downloading. */
	modelAddLink(url: string): Promise<string | null>;
	unpair(id: string): void;
	pairAllow(): void;
	pairDeny(): void;
	quit(): void;
}

// ---------------------------------------------------------------- client side

export interface RemoteModel {
	id: string;
	name: string;
}

/** A host seen on the network. Friends get general reasons only. */
export interface RemoteHost {
	id: string;
	name: string;
	os: string;
	/** e.g. "RTX A2000 8 GB"; empty when unknown. */
	gpu: string;
	status: RuntimeStateName;
	reason: string;
	/** True when someone else's question is being answered. */
	busyWithOther: boolean;
	paired: boolean;
	/** Only filled in for paired hosts. */
	models: RemoteModel[];
	address: string;
	/** The owner's own computer ("This computer"): always paired, answers whatever the pause rules say. */
	own: boolean;
}

export type ChatRole = 'user' | 'assistant' | 'note';

export interface ChatMessage {
	role: ChatRole;
	text: string;
	streaming?: boolean;
}

/** Where the client's own question is. */
export type Turn =
	| { phase: 'queued'; position: number }
	| { phase: 'loading'; model: string; pct: number }
	| { phase: 'streaming' };

export interface ClientView {
	scanning: boolean;
	hosts: RemoteHost[];
	/** A pairing request in flight: the code to read out to the host's owner. */
	pairing: { hostId: string; code: string } | null;
	/** The host the chat screen talks to. */
	chatHost: string | null;
	model: string;
	msgs: ChatMessage[];
	turn: Turn | null;
	/** Plain-language explanation when discovery looks blocked (client isolation etc.). */
	networkHint: string;
	/** A one-off message for the Computers screen, e.g. the owner turned a pairing request down. */
	notice: string;
}

export interface ClientBackend {
	readonly view: ClientView;
	askConnect(hostId: string): void;
	/** Resolves to an error message, or null when the host was found. */
	addByIp(address: string): Promise<string | null>;
	/** Look around the network again (subnet scan). */
	rescan(): void;
	openChat(hostId: string): void;
	sendMessage(text: string): void;
	/** Stop the answer, or cancel a question still waiting in line. */
	stop(): void;
	/** Ask the last question again (after a pause cut it short or turned it away). */
	retry(): void;
	setModel(id: string): void;
	forget(hostId: string): void;
}
