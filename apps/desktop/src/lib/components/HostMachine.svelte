<script lang="ts">
	import { hostBackend } from '../mock/backend.svelte';
	import type { HostTab } from '../types';
	import Machine from './Machine.svelte';
	import WelcomeScreen from './WelcomeScreen.svelte';
	import ShareScreen from './ShareScreen.svelte';
	import LimitsScreen from './LimitsScreen.svelte';
	import ModelsScreen from './ModelsScreen.svelte';
	import AccessScreen from './AccessScreen.svelte';
	import Toasts from './Toasts.svelte';
	import TrayMenu from './TrayMenu.svelte';
	import PairModal from './PairModal.svelte';

	const h = hostBackend.host;
	const rt = hostBackend.rt;

	const tabs = [
		{ id: 'share', label: 'Share' },
		{ id: 'limits', label: 'Limits' },
		{ id: 'models', label: 'Models' },
		{ id: 'access', label: 'Access' }
	];

	const trayDotColor = $derived(
		{ off: 'var(--ink-faint)', available: 'var(--live)', busy: 'var(--win)', paused: 'var(--wait)' }[rt.state]
	);
</script>

<Machine
	os="windows"
	title="Dara's laptop"
	osName="Windows 11 · RTX A2000 8 GB"
	{tabs}
	activeTab={h.tab}
	onTab={(id) => hostBackend.setTab(id as HostTab)}
	taskbarLabel="Windows"
>
	{#snippet screen()}
		{#if !h.welcomed}
			<WelcomeScreen />
		{:else if h.tab === 'share'}
			<ShareScreen />
		{:else if h.tab === 'limits'}
			<LimitsScreen />
		{:else if h.tab === 'models'}
			<ModelsScreen />
		{:else if h.tab === 'access'}
			<AccessScreen />
		{/if}
	{/snippet}
	{#snippet taskbarRight()}
		<button class="trayicon" onclick={() => hostBackend.trayToggle()}>
			<span class="traydot" style="background:{trayDotColor}"></span> GPU Share
		</button>
	{/snippet}
	{#snippet overlays()}
		<Toasts />
		<TrayMenu />
		<PairModal />
	{/snippet}
</Machine>
