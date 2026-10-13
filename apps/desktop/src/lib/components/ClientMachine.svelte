<script lang="ts">
	// Your PC in the preview: the Use-mode screens inside an Ubuntu window frame.
	import { sim } from '../mock/backend.svelte';
	import { ui, type ClientTab } from '../ui.svelte';
	import Machine from './Machine.svelte';
	import DevicesScreen from './DevicesScreen.svelte';
	import ChatScreen from './ChatScreen.svelte';

	const tabs = [
		{ id: 'devices', label: 'Computers' },
		{ id: 'chat', label: 'Chat' }
	];

	// Both windows are one simulation with a single time of day, as in the prototype.
	const clockText = $derived(String(sim.world.hour).padStart(2, '0') + ':00');
</script>

<Machine
	os="ubuntu"
	title="Your PC"
	osName="Ubuntu 24.04 · no graphics card"
	{tabs}
	activeTab={ui.clientTab}
	onTab={(id) => (ui.clientTab = id as ClientTab)}
	taskbarLabel="Ubuntu"
>
	{#snippet screen()}
		{#if ui.clientTab === 'devices'}
			<DevicesScreen />
		{:else}
			<ChatScreen />
		{/if}
	{/snippet}
	{#snippet taskbarRight()}
		<span style="margin-left:auto" class="mono">{clockText}</span>
	{/snippet}
</Machine>
