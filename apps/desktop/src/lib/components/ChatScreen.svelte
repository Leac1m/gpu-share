<script lang="ts">
	import { clientBackend } from '../mock/backend.svelte';
	import { model } from '../mock/catalog';
	import StatusChip from './StatusChip.svelte';

	const c = clientBackend.client;
	const rt = clientBackend.rt;

	const enabled = $derived(clientBackend.offeredModels());

	$effect(() => {
		if (enabled.length && !enabled.includes(c.model)) {
			clientBackend.setModel(enabled[0]);
		}
	});

	const blocked = $derived(rt.state === 'paused' || rt.state === 'off');

	let text = $state('');
	let msgsEl: HTMLDivElement | undefined = $state();

	function handleSend() {
		if (!text.trim()) return;
		clientBackend.sendMessage(text);
		text = '';
	}
	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Enter') handleSend();
	}

	$effect(() => {
		// touch c.msgs so this effect reruns whenever the message list changes
		c.msgs.length;
		c.msgs.at(-1)?.text;
		if (msgsEl) msgsEl.scrollTop = msgsEl.scrollHeight;
	});
</script>

{#if !c.paired}
	<div class="empty">
		<div class="big">Not connected yet</div>
		<div>Go to Computers and ask Dara's laptop to let you in.</div>
		<button class="btn pri sm" style="margin-top:14px" onclick={() => clientBackend.setTab('devices')}>
			Open Computers
		</button>
	</div>
{:else if !enabled.length}
	<div class="empty">
		<div class="big">No models available</div>
		<div>Dara has switched them all off, or they need more graphics card memory than he has allowed.</div>
	</div>
{:else}
	<div class="chatwrap">
		<div class="chathead">
			<span class="who">Dara's laptop</span>
			<select disabled={rt.streaming} onchange={(e) => clientBackend.setModel((e.target as HTMLSelectElement).value)}>
				{#each enabled as id (id)}
					<option value={id} selected={id === c.model}>{model(id)!.name}</option>
				{/each}
			</select>
			<span style="margin-left:auto"><StatusChip state={rt.state} /></span>
		</div>
		<div class="msgs" bind:this={msgsEl}>
			{#if rt.loading}
				<div class="loadbar mono">Loading {model(rt.loading)!.name} onto his graphics card… {rt.loadPct || 0}%</div>
			{/if}
			{#if blocked}
				<div class="banner warn" style="margin:0">
					<div>
						<b>{rt.state === 'off' ? "Dara has switched sharing off" : "Dara's computer is busy"}</b>
						{rt.state === 'off'
							? 'Nothing you send will get through until he turns it back on.'
							: rt.reason + '. It will come back on its own.'}
					</div>
				</div>
			{/if}
			{#if c.msgs.length === 0 && !blocked}
				<div class="empty" style="padding:26px 10px">
					<div class="big">Ask anything</div>
					<div>The first question takes a few seconds longer while the model loads onto his card.</div>
				</div>
			{/if}
			{#each c.msgs as m, i (i)}
				{#if m.role === 'note'}
					<div class="banner calm" style="margin:0;font-size:12.5px">{m.text}</div>
				{:else}
					<div class="msg {m.role === 'user' ? 'me' : 'ai'}">{m.text}{#if m.streaming}<span class="caret"></span>{/if}</div>
				{/if}
			{/each}
		</div>
		<div class="composer">
			<input
				bind:value={text}
				placeholder={blocked ? "Waiting for Dara's computer…" : 'Type a question'}
				disabled={blocked || rt.streaming}
				onkeydown={handleKeydown}
			/>
			{#if rt.streaming}
				<button class="btn danger" onclick={() => clientBackend.stopStream()}>Stop</button>
			{:else}
				<button class="btn pri" disabled={blocked} onclick={handleSend}>Send</button>
			{/if}
		</div>
	</div>
{/if}
