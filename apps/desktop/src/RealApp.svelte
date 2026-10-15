<script lang="ts">
	// The installed app: one window, Share mode (if this computer can share) and Use mode.
	import { capability, connect } from './lib/real/backend.svelte';
	import { ui, type ClientTab, type HostTab } from './lib/ui.svelte';
	import { clientBackend, hostBackend } from './lib/backend';
	import HostScreens from './lib/components/HostScreens.svelte';
	import DevicesScreen from './lib/components/DevicesScreen.svelte';
	import ChatScreen from './lib/components/ChatScreen.svelte';
	import Toasts from './lib/components/Toasts.svelte';
	import PairModal from './lib/components/PairModal.svelte';
	import { hardware } from './lib/hardware';

	connect();

	// when pairing succeeds the runtime opens the chat; follow it to the Chat tab
	let wasPairing = false;
	$effect(() => {
		const pairing = !!clientBackend.view.pairing;
		if (wasPairing && !pairing && clientBackend.view.chatHost) ui.clientTab = 'chat';
		wasPairing = pairing;
	});

	let theme = $state<'light' | 'dark'>(window.matchMedia?.('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');
	$effect(() => {
		document.documentElement.dataset.theme = theme;
	});

	const hostTabs: { id: HostTab; label: string }[] = [
		{ id: 'share', label: 'Share' },
		{ id: 'limits', label: 'Limits' },
		{ id: 'models', label: 'Models' },
		{ id: 'access', label: 'Access' }
	];
	const clientTabs: { id: ClientTab; label: string }[] = [
		{ id: 'devices', label: 'Computers' },
		{ id: 'chat', label: 'Chat' }
	];

	const mode = $derived(capability.canShare ? ui.mode : 'use');
	const dot = $derived(
		{ off: 'var(--ink-faint)', available: 'var(--live)', busy: 'var(--win)', paused: 'var(--wait)' }[hostBackend.view.state]
	);
</script>

<header class="topbar">
	<div class="brand">GPU<span>Share</span></div>
	{#if capability.canShare}
		<div class="modes" role="tablist">
			<button class="tbtn" role="tab" aria-selected={mode === 'share'} onclick={() => (ui.mode = 'share')}>
				<span class="traydot" style="background:{dot}"></span> {hardware(hostBackend.view).shareLabel}
			</button>
			<button class="tbtn" role="tab" aria-selected={mode === 'use'} onclick={() => (ui.mode = 'use')}>
				Chat
			</button>
		</div>
	{/if}
	<div class="spacer"></div>
	<button class="tbtn" onclick={() => (theme = theme === 'dark' ? 'light' : 'dark')}>{theme === 'dark' ? 'Light' : 'Dark'}</button>
</header>

<main class="appwin">
	{#if !capability.ready}
		<div class="empty"><div class="spinner"></div></div>
	{:else}
		<section class="machine app">
			<div class="tabs" role="tablist">
				{#if mode === 'share'}
					{#each hostTabs as t (t.id)}
						<button
							class="tab"
							role="tab"
							aria-selected={ui.hostTab === t.id}
							onclick={() => {
								ui.hostTab = t.id;
								hostBackend.welcomeDone();
							}}>{t.label}</button
						>
					{/each}
				{:else}
					{#each clientTabs as t (t.id)}
						<button class="tab" role="tab" aria-selected={ui.clientTab === t.id} onclick={() => (ui.clientTab = t.id)}
							>{t.label}</button
						>
					{/each}
				{/if}
			</div>
			<div class="screen">
				{#if mode === 'share'}
					<HostScreens />
				{:else if ui.clientTab === 'devices'}
					<DevicesScreen />
				{:else}
					<ChatScreen />
				{/if}
			</div>
			{#if capability.canShare}
				<Toasts />
				<PairModal />
			{/if}
		</section>
	{/if}
</main>
