<script lang="ts">
	import { clientBackend } from '../backend';
	import { ui } from '../ui.svelte';
	import { ordinal } from '../format';
	import StatusChip from './StatusChip.svelte';
	import Markdown from './Markdown.svelte';

	const c = clientBackend.view;

	const host = $derived(c.hosts.find((h) => h.id === c.chatHost && h.paired));
	const models = $derived(host?.models ?? []);
	const modelName = (id: string) => models.find((m) => m.id === id)?.name ?? id;

	$effect(() => {
		if (models.length && !models.some((m) => m.id === c.model)) clientBackend.setModel(models[0].id);
	});

	const blocked = $derived(!host || host.status === 'paused' || host.status === 'off');
	// offer "Ask again" when the last thing in the chat is a note about a cut or refused question
	const canRetry = $derived(
		!c.turn && !blocked && c.msgs.at(-1)?.role === 'note' && /wasn't answered|stopped part way/.test(c.msgs.at(-1)?.text ?? '')
	);
	const waitingForMe = $derived(!!c.turn);

	let text = $state('');
	let msgsEl: HTMLDivElement | undefined = $state();

	function send() {
		if (!text.trim() || blocked || waitingForMe) return;
		clientBackend.sendMessage(text);
		text = '';
	}

	$effect(() => {
		// rerun whenever the transcript grows or the streaming reply changes
		void c.msgs.length;
		void c.msgs.at(-1)?.text;
		void c.turn;
		if (msgsEl) msgsEl.scrollTop = msgsEl.scrollHeight;
	});
</script>

{#if !host}
	<div class="empty">
		<div class="big">Not connected yet</div>
		<div>Go to Computers and ask a friend's computer to let you in.</div>
		<button class="btn pri sm" style="margin-top:14px" onclick={() => (ui.clientTab = 'devices')}>
			Open Computers
		</button>
	</div>
{:else if !models.length}
	<div class="empty">
		<div class="big">No models available</div>
		<div>
			The owner has switched them all off, or they need more memory than the owner
			allows.
		</div>
	</div>
{:else}
	<div class="chatwrap">
		<div class="chathead">
			<span class="who">{host.name}</span>
			<select disabled={waitingForMe} onchange={(e) => clientBackend.setModel((e.target as HTMLSelectElement).value)}>
				{#each models as m (m.id)}
					<option value={m.id} selected={m.id === c.model}>{m.name}</option>
				{/each}
			</select>
			<span style="margin-left:auto"><StatusChip state={host.status} /></span>
		</div>
		<div class="msgs" bind:this={msgsEl}>
			{#if blocked}
				<div class="banner warn" style="margin:0">
					<div>
						<b>{host.status === 'off' ? 'Sharing is switched off' : 'Sharing is paused'}</b>
						{host.status === 'off'
							? 'Nothing you send will get through until the owner turns it back on.'
							: host.reason + '. It will come back on its own.'}
					</div>
				</div>
			{/if}
			{#if c.msgs.length === 0 && !blocked}
				<div class="empty" style="padding:26px 10px">
					<div class="big">Ask anything</div>
					<div>
						The first question takes a few seconds longer while the model loads. If
						another friend is already asking, you'll wait your turn.
					</div>
				</div>
			{/if}
			{#each c.msgs as m, i (i)}
				{#if m.role === 'note'}
					<div class="banner calm" style="margin:0;font-size:12.5px">{m.text}</div>
				{:else if m.role === 'user'}
					<div class="msg me">{m.text}</div>
				{:else}
					<div class="msg ai"><Markdown text={m.text} />{#if m.streaming}<span class="caret"></span>{/if}</div>
				{/if}
			{/each}
			{#if canRetry}
				<div><button class="btn sm" onclick={() => clientBackend.retry()}>Ask again</button></div>
			{/if}
			{#if c.turn?.phase === 'queued'}
				{@const ahead = c.turn.position - 1}
				<div class="loadbar">
					You're {ordinal(c.turn.position)} in line. {ahead === 1
						? 'One question from another friend is'
						: ahead + ' questions from other friends are'} ahead of yours. Their computer answers one at a time,
					so yours starts as soon as {ahead === 1 ? 'it finishes' : 'they finish'}.
				</div>
			{:else if c.turn?.phase === 'loading'}
				<div class="loadbar mono">Loading {modelName(c.turn.model)}… {c.turn.pct}%</div>
			{/if}
		</div>
		<div class="composer">
			<input
				bind:value={text}
				placeholder={blocked ? "Waiting for the owner's computer…" : 'Type a question'}
				disabled={blocked || waitingForMe}
				onkeydown={(e) => e.key === 'Enter' && send()}
			/>
			{#if waitingForMe}
				<button class="btn danger" onclick={() => clientBackend.stop()}>
					{c.turn?.phase === 'streaming' ? 'Stop' : 'Cancel'}
				</button>
			{:else}
				<button class="btn pri" disabled={blocked} onclick={send}>Send</button>
			{/if}
		</div>
	</div>
{/if}
