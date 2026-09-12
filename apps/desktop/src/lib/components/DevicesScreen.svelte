<script lang="ts">
	import { clientBackend } from '../mock/backend.svelte';
	import StatusChip from './StatusChip.svelte';

	const c = clientBackend.client;
	const rt = clientBackend.rt;
	const h = clientBackend.host;

	const visible = $derived(h.sharing);
	const enabled = $derived(clientBackend.offeredModels());

	function addByIp() {
		alert(
			'In the finished app this asks for an address like 192.168.1.42, for networks where computers cannot find each other automatically.'
		);
	}
</script>

{#if !visible}
	<div class="empty">
		<div class="spinner"></div>
		<div class="big">Looking for computers on your network</div>
		<div>Nothing yet. A computer shows up here once its owner turns sharing on.</div>
		<div class="rule"></div>
		<div style="font-size:13px">
			Can't find a computer you expect? It may be on a different Wi-Fi, or the router may be
			blocking devices from seeing each other.
		</div>
		<button class="btn sm" style="margin-top:12px" onclick={addByIp}>Add by IP address</button>
	</div>
{:else}
	<h4 class="hd">Computers on your network</h4>
	<p class="sub">Found by looking around your Wi-Fi, the same way LocalSend does.</p>
	<div class="device">
		<div class="ic">D</div>
		<div class="body">
			<div class="nm">Dara's laptop</div>
			<div class="ds">Windows · RTX A2000 8 GB · {enabled.length} model{enabled.length === 1 ? '' : 's'} offered</div>
			<div style="margin-top:7px">
				<StatusChip state={rt.state} />
				{#if rt.state === 'paused'}<span class="ds" style="margin-left:8px">{rt.reason}</span>{/if}
			</div>
		</div>
		<div>
			{#if c.paired}
				<button class="btn pri sm" onclick={() => clientBackend.openChat()}>Chat</button>
			{:else if c.waitingCode}
				<button class="btn sm" disabled>Waiting…</button>
			{:else}
				<button class="btn sm" onclick={() => clientBackend.askConnect()}>Ask to connect</button>
			{/if}
		</div>
	</div>
	{#if c.waitingCode}
		<div class="banner info">
			<div>
				<b>Check the code with Dara</b>
				Ask him to confirm he sees <span class="mono">{c.waitingCode}</span> on his screen, then say
				yes.
			</div>
		</div>
	{/if}
	<div class="rule"></div>
	<p class="sub" style="margin:0">
		Only computers that have allowed you will let you chat. Everything stays on your own network.
	</p>
{/if}
