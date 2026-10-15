<script lang="ts">
	// The host's window in the preview: Dara's laptop with its tabs, tray and popups.
	import { hostBackend } from '../backend';
	import { ui, type HostTab } from '../ui.svelte';
	import Machine from './Machine.svelte';
	import HostScreens from './HostScreens.svelte';
	import Toasts from './Toasts.svelte';
	import TrayMenu from './TrayMenu.svelte';
	import PairModal from './PairModal.svelte';

	const h = hostBackend.view;

	const tabs = [
		{ id: 'share', label: 'Share' },
		{ id: 'limits', label: 'Limits' },
		{ id: 'models', label: 'Models' },
		{ id: 'access', label: 'Access' }
	];

	const trayDotColor = $derived(
		{ off: 'var(--ink-faint)', available: 'var(--live)', busy: 'var(--win)', paused: 'var(--wait)' }[h.state]
	);

	function onTab(id: string) {
		ui.hostTab = id as HostTab;
		hostBackend.welcomeDone();
	}
</script>

<Machine
	os="windows"
	title={h.name}
	osName="Windows 11 · RTX A2000 8 GB"
	{tabs}
	activeTab={ui.hostTab}
	{onTab}
	taskbarLabel="Windows"
>
	{#snippet screen()}
		<HostScreens />
	{/snippet}
	{#snippet taskbarRight()}
		<button class="trayicon" onclick={() => (ui.trayOpen = !ui.trayOpen)}>
			<span class="traydot" style="background:{trayDotColor}"></span> GPU Share
		</button>
	{/snippet}
	{#snippet overlays()}
		<Toasts />
		<TrayMenu />
		<PairModal />
	{/snippet}
</Machine>
