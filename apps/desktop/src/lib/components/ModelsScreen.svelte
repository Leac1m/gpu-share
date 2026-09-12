<script lang="ts">
	import { hostBackend } from '../mock/backend.svelte';
	import { CATALOG, CARD, vramFor } from '../mock/catalog';
	import Switch from './Switch.svelte';
	import type { CatalogEntry } from '../types';

	const h = hostBackend.host;
	const rt = hostBackend.rt;

	const installed = $derived(CATALOG.filter((m) => h.installed.includes(m.id)));
	const available = $derived(CATALOG.filter((m) => !h.installed.includes(m.id)));

	function need(m: CatalogEntry) {
		return vramFor(m, h.ctx);
	}
	function fitsCard(m: CatalogEntry) {
		return need(m) <= CARD.vram - 0.4;
	}
	function fitsBudget(m: CatalogEntry) {
		return need(m) <= h.budget;
	}
</script>

<h4 class="hd">Models on your machine</h4>
<p class="sub">
	These are the AI models your friend can choose from. Switch one off to keep it for yourself.
</p>
{#if installed.length === 0}
	<div class="banner calm">You have no models yet. Add one below.</div>
{:else}
	{#each installed as m (m.id)}
		{@const on = !h.disabled.includes(m.id)}
		{@const isLoaded = rt.loaded === m.id}
		{@const isLoading = rt.loading === m.id}
		{@const dl = h.downloads[m.id]}
		<div class="model">
			<div class="body">
				<div class="nm">
					{m.name}
					{#if isLoaded}<span class="badge loaded">On the card now</span>{/if}
					{#if isLoading}<span class="badge loaded">Loading…</span>{/if}
					{#if !fitsCard(m)}
						<span class="badge nofit">Too big for this card</span>
					{:else if !fitsBudget(m)}
						<span class="badge nofit">Over your memory limit</span>
					{:else}
						<span class="badge fits">Fits your card</span>
					{/if}
				</div>
				<div class="ds">{m.desc}</div>
				{#if fitsCard(m) && !fitsBudget(m)}
					<div class="ds" style="color:var(--halt)">
						Not offered to friends until you raise your memory limit.
					</div>
				{/if}
				<div class="meta mono">
					{m.fileGB.toFixed(1)} GB file · needs {need(m).toFixed(1)} GB on the card · about {m.tps ||
						'—'} words/sec · {m.license}
				</div>
				{#if dl}
					<div class="progress"><i style="width:{dl.pct}%"></i></div>
					<div class="meta mono">{dl.stage}</div>
				{/if}
			</div>
			<div class="ctl" style="display:flex;flex-direction:column;gap:7px;align-items:flex-end">
				<Switch checked={on} size="sm" label="Offer this model to friends" onToggle={() => hostBackend.modelToggle(m.id)} />
				<button class="btn sm danger" onclick={() => hostBackend.modelRemove(m.id)}>Remove</button>
			</div>
		</div>
	{/each}
{/if}
<div class="rule"></div>
<h4 class="hd">Models you can add</h4>
<p class="sub">Downloaded once, then kept on your disk. Each file is checked after downloading.</p>
{#each available as m (m.id)}
	{@const dl = h.downloads[m.id]}
	<div class="model">
		<div class="body">
			<div class="nm">
				{m.name}
				{#if !fitsCard(m)}
					<span class="badge nofit">Too big for this card</span>
				{:else if !fitsBudget(m)}
					<span class="badge nofit">Over your memory limit</span>
				{:else}
					<span class="badge fits">Fits your card</span>
				{/if}
			</div>
			<div class="ds">{m.desc}</div>
			<div class="meta mono">
				{m.fileGB.toFixed(1)} GB file · needs {need(m).toFixed(1)} GB on the card · about {m.tps ||
					'—'} words/sec · {m.license}
			</div>
			{#if dl}
				<div class="progress"><i style="width:{dl.pct}%"></i></div>
				<div class="meta mono">{dl.stage}</div>
			{/if}
		</div>
		<div class="ctl" style="display:flex;flex-direction:column;gap:7px;align-items:flex-end">
			{#if dl}
				<button class="btn sm" disabled>Adding…</button>
			{:else}
				<button class="btn sm {fitsCard(m) ? 'pri' : ''}" disabled={!fitsCard(m)} onclick={() => hostBackend.modelAdd(m.id)}>
					Add
				</button>
			{/if}
		</div>
	</div>
{/each}
