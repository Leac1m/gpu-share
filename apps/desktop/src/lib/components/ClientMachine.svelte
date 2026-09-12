<script lang="ts">
	import { clientBackend, hostBackend } from '../mock/backend.svelte';
	import type { ClientTab } from '../types';
	import Machine from './Machine.svelte';
	import DevicesScreen from './DevicesScreen.svelte';
	import ChatScreen from './ChatScreen.svelte';

	const c = clientBackend.client;

	const tabs = [
		{ id: 'devices', label: 'Computers' },
		{ id: 'chat', label: 'Chat' }
	];

	// The client's taskbar clock mirrors the shared simulated world clock, same as the
	// prototype (both windows are one simulation, so there's a single "time of day").
	const clockText = $derived(String(hostBackend.world.hour).padStart(2, '0') + ':00');
</script>

<Machine
	os="ubuntu"
	title="Your PC"
	osName="Ubuntu 24.04 · no graphics card"
	{tabs}
	activeTab={c.tab}
	onTab={(id) => clientBackend.setTab(id as ClientTab)}
	taskbarLabel="Ubuntu"
>
	{#snippet screen()}
		{#if c.tab === 'devices'}
			<DevicesScreen />
		{:else}
			<ChatScreen />
		{/if}
	{/snippet}
	{#snippet taskbarRight()}
		<span style="margin-left:auto" class="mono">{clockText}</span>
	{/snippet}
</Machine>
