<script lang="ts">
	import { hostBackend } from '../backend';
	import { vramFor } from '../format';
	import Switch from './Switch.svelte';
	import type { CatalogEntry } from '../types';

	const h = hostBackend.view;

	const installed = $derived(h.catalog.filter((m) => h.installed.includes(m.id)));
	const available = $derived(h.catalog.filter((m) => !h.installed.includes(m.id)));
	const vram = $derived(h.card?.vramGB ?? 8);

	const need = (m: CatalogEntry) => vramFor(m, h.settings.ctx);
	const isDefault = (m: CatalogEntry) => (h.settings.defaultModel ?? hostBackend.offeredModels()[0]) === m.id;

	let advanced = $state(false);
	let link = $state('');
	let linkError = $state('');
	let linkBusy = $state(false);
	async function addLink() {
		linkBusy = true;
		linkError = (await hostBackend.modelAddLink(link.trim())) ?? '';
		linkBusy = false;
		if (!linkError) link = '';
	}
	const fitsCard = (m: CatalogEntry) => need(m) <= vram - 0.4;
	const fitsBudget = (m: CatalogEntry) => need(m) <= h.settings.budget;
</script>

{#snippet card(m: CatalogEntry)}
	{@const isInstalled = h.installed.includes(m.id)}
	{@const on = !h.settings.disabled.includes(m.id)}
	{@const dl = h.downloads[m.id]}
	<div class="model">
		<div class="body">
			<div class="nm">
				{m.name}
				{#if h.loaded === m.id}<span class="badge loaded">On the card now</span>{/if}
				{#if h.loading === m.id}<span class="badge loaded">Loading…</span>{/if}
				{#if isInstalled && isDefault(m)}<span class="badge fits">Default</span>{/if}
				{#if !fitsCard(m)}
					<span class="badge nofit">Too big for this card</span>
				{:else if !fitsBudget(m)}
					<span class="badge nofit">Over your memory limit</span>
				{:else}
					<span class="badge fits">Fits your card</span>
				{/if}
			</div>
			<div class="ds">{m.desc}</div>
			{#if isInstalled && fitsCard(m) && !fitsBudget(m)}
				<div class="ds" style="color:var(--halt)">Not offered to friends until you raise your memory limit.</div>
			{/if}
			<div class="meta mono">
				{m.fileGB.toFixed(1)} GB file · needs {need(m).toFixed(1)} GB on the card · {m.tps
					? 'about ' + m.tps + ' tokens/sec (estimate)'
					: "won't run here"} · {m.license}
			</div>
			{#if dl}
				<div class="progress"><i style="width:{dl.pct}%"></i></div>
				<div class="meta mono">{dl.stage}</div>
			{/if}
		</div>
		<div class="ctl" style="display:flex;flex-direction:column;gap:7px;align-items:flex-end">
			{#if isInstalled}
				<Switch checked={on} size="sm" label="Offer this model to friends" onToggle={() => hostBackend.modelToggle(m.id)} />
				{#if !isDefault(m) && on}
					<button class="btn sm" onclick={() => hostBackend.modelSetDefault(m.id)}>Make default</button>
				{/if}
				<button class="btn sm danger" onclick={() => hostBackend.modelRemove(m.id)}>Remove</button>
			{:else if dl}
				<button class="btn sm" disabled>Adding…</button>
			{:else}
				<button class="btn sm {fitsCard(m) ? 'pri' : ''}" disabled={!fitsCard(m)} onclick={() => hostBackend.modelAdd(m.id)}>
					Add
				</button>
			{/if}
		</div>
	</div>
{/snippet}

<h4 class="hd">Models on your machine</h4>
<p class="sub">
	These are the AI models your friends can choose from. Switch one off to keep it for yourself. When
	two friends want different models, the card swaps between them, which adds a few seconds.
</p>
{#each installed as m (m.id)}
	{@render card(m)}
{:else}
	<div class="banner calm">You have no models yet. Add one below.</div>
{/each}
<div class="rule"></div>
<h4 class="hd">Models you can add</h4>
<p class="sub">Downloaded once, then kept on your disk. Each file is checked after downloading.</p>
{#each available as m (m.id)}
	{@render card(m)}
{/each}

<div class="rule"></div>
<button class="btn sm" onclick={() => (advanced = !advanced)}>{advanced ? 'Hide advanced' : 'Advanced'}</button>
{#if advanced}
	<h4 class="hd" style="margin-top:14px">Add from a Hugging Face link</h4>
	<p class="sub">
		For a model that isn't in the list: paste the link to a .gguf file on huggingface.co. It is checked
		against the checksum Hugging Face publishes. Make sure you're happy with its licence.
	</p>
	<div class="composer" style="border:0;padding:0">
		<input bind:value={link} placeholder="https://huggingface.co/…/resolve/main/model.gguf" />
		<button class="btn pri sm" disabled={linkBusy || !link.trim()} onclick={addLink}>{linkBusy ? 'Checking…' : 'Add'}</button>
	</div>
	{#if linkError}<div class="banner warn" style="margin-top:10px"><div>{linkError}</div></div>{/if}
{/if}
