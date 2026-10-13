<script lang="ts">
	import { clientBackend } from '../backend';
	import { ui } from '../ui.svelte';
	import StatusChip from './StatusChip.svelte';

	const c = clientBackend.view;

	let ipOpen = $state(false);
	let ip = $state('');
	let ipError = $state('');
	let ipBusy = $state(false);

	async function addByIp() {
		ipBusy = true;
		ipError = (await clientBackend.addByIp(ip.trim())) ?? '';
		ipBusy = false;
		if (!ipError) {
			ipOpen = false;
			ip = '';
		}
	}

	function openChat(id: string) {
		clientBackend.openChat(id);
		ui.clientTab = 'chat';
	}

	const pairingHost = $derived(c.hosts.find((h) => h.id === c.pairing?.hostId));
</script>

{#snippet addIp()}
	{#if ipOpen}
		<div class="composer" style="border:0;padding:12px 0 0">
			<input bind:value={ip} placeholder="An address like 192.168.1.42" onkeydown={(e) => e.key === 'Enter' && addByIp()} />
			<button class="btn pri sm" disabled={ipBusy || !ip.trim()} onclick={addByIp}>{ipBusy ? 'Looking…' : 'Add'}</button>
		</div>
		{#if ipError}<div class="banner warn" style="margin-top:10px"><div>{ipError}</div></div>{/if}
	{:else}
		<button class="btn sm" style="margin-top:12px" onclick={() => (ipOpen = true)}>Add by IP address</button>
	{/if}
{/snippet}

{#if !c.hosts.length}
	<div class="empty">
		<div class="spinner"></div>
		<div class="big">Looking for computers on your network</div>
		<div>Nothing yet. A computer shows up here once its owner turns sharing on.</div>
		<div class="rule"></div>
		<div style="font-size:13px">
			{c.networkHint ||
				"Can't find a computer you expect? It may be on a different Wi-Fi, or the router may be blocking devices from seeing each other."}
		</div>
		{@render addIp()}
	</div>
{:else}
	<h4 class="hd">Computers on your network</h4>
	<p class="sub">Found by looking around your Wi-Fi, the same way LocalSend does.</p>
	{#each c.hosts as host (host.id)}
		<div class="device">
			<div class="ic">{host.name.slice(0, 1).toUpperCase()}</div>
			<div class="body">
				<div class="nm">{host.name}</div>
				<div class="ds">
					{[host.os, host.gpu].filter(Boolean).join(' · ')}{host.paired
						? ' · ' + host.models.length + ' model' + (host.models.length === 1 ? '' : 's') + ' offered'
						: ''}
				</div>
				<div style="margin-top:7px">
					<StatusChip state={host.status} />
					{#if host.status === 'paused'}<span class="ds" style="margin-left:8px">{host.reason}</span>{/if}
					{#if host.status === 'busy' && host.busyWithOther}
						<span class="ds" style="margin-left:8px">Answering another friend</span>
					{/if}
				</div>
			</div>
			<div>
				{#if host.paired}
					<button class="btn pri sm" onclick={() => openChat(host.id)}>Chat</button>
				{:else if c.pairing?.hostId === host.id}
					<button class="btn sm" disabled>Waiting…</button>
				{:else}
					<button class="btn sm" disabled={!!c.pairing} onclick={() => clientBackend.askConnect(host.id)}>
						Ask to connect
					</button>
				{/if}
			</div>
		</div>
	{/each}
	{#if c.pairing && pairingHost}
		<div class="banner info">
			<div>
				<b>Check the code with the owner of {pairingHost.name}</b>
				Ask them to confirm they see <span class="mono">{c.pairing.code}</span> on their screen, then
				they can say yes.
			</div>
		</div>
	{/if}
	<div class="rule"></div>
	<p class="sub" style="margin:0">
		{c.hosts.some((h) => h.paired)
			? "You take turns with the owner's other friends, one question at a time."
			: "Until the owner allows you, you can only see a computer's name and whether it's ready."}
		Everything stays on your own network.
	</p>
	{@render addIp()}
{/if}
