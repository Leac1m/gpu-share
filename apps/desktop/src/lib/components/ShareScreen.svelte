<script lang="ts">
	import { hostBackend } from '../mock/backend.svelte';
	import { model, vramFor, CARD } from '../mock/catalog';
	import StatusChip from './StatusChip.svelte';
	import Switch from './Switch.svelte';

	const h = hostBackend.host;
	const rt = hostBackend.rt;

	const loadedModel = $derived(rt.loaded ? model(rt.loaded) : null);
	const used = $derived(loadedModel ? vramFor(loadedModel, h.ctx) : 0);
	const friend = $derived(h.paired[0] as (typeof h.paired)[number] | undefined);
	const totalMins = $derived(
		Math.round((Object.values(h.usage).reduce((a, u) => a + u.gpuSec, 0) / 60) * 10) / 10
	);
	const tempBarClass = $derived(rt.temp >= h.tempPause ? 'hot' : rt.temp >= 70 ? 'warm' : '');
	const switchHint = $derived(
		h.sharing
			? 'Your computer is visible to friends you have allowed.'
			: 'Your computer is invisible and your graphics card is untouched.'
	);
	const rightNowText = $derived.by(() => {
		if (rt.state === 'off') return 'Nothing is running. The app is idle and your card is free.';
		if (rt.state === 'paused') return 'Nothing is running. Your card was freed when sharing paused.';
		if (rt.loading) return 'Loading ' + model(rt.loading)!.name + ' onto your card.';
		if (rt.streaming)
			return (
				(h.paired[0] ? h.paired[0].name : 'Your friend') +
				' is chatting with ' +
				model(rt.loaded!)!.name +
				'.'
			);
		if (rt.loaded) return model(rt.loaded)!.name + ' is on your card, waiting for the next question.';
		return 'Ready. Nothing is loaded until someone asks a question.';
	});
</script>

<div class="bigswitch">
	<Switch checked={h.sharing} onToggle={() => hostBackend.setSharing(!h.sharing)} label="Share my graphics card" />
	<div>
		<div class="lbl">Share my graphics card</div>
		<div class="hint">{switchHint}</div>
	</div>
	<div style="margin-left:auto"><StatusChip state={rt.state} /></div>
</div>

{#if rt.state === 'paused'}
	<div class="banner warn">
		<div><b>Sharing is paused</b>{rt.reason}. It starts again on its own.</div>
		<button class="act" onclick={() => hostBackend.setTab('limits')}>Change this rule</button>
	</div>
{:else if rt.state === 'off' && !h.sharing && h.paired.length}
	<div class="banner calm">Sharing is switched off. {h.paired[0].name} can't reach your computer.</div>
{/if}

<div class="readouts">
	<div class="ro">
		<div class="k">Graphics card memory</div>
		<div class="v mono">{used.toFixed(1)}<small> / {CARD.vram.toFixed(1)} GB</small></div>
		<div class="meter"><i style="width:{(used / CARD.vram) * 100}%"></i></div>
	</div>
	<div class="ro">
		<div class="k">Temperature</div>
		<div class="v mono">{Math.round(rt.temp)}°C</div>
		<div class="meter">
			<i class={tempBarClass} style="width:{Math.min(100, ((rt.temp - 30) / 60) * 100)}%"></i>
		</div>
	</div>
	<div class="ro">
		<div class="k">Used this week</div>
		<div class="v mono">{totalMins}<small> min</small></div>
		<div class="hint" style="font-size:12px;color:var(--ink-faint);margin-top:7px">
			across {h.paired.length} friend{h.paired.length === 1 ? '' : 's'}
		</div>
	</div>
</div>

<div class="rule"></div>
<h4 class="hd">Right now</h4>
<div class="sub" style="margin-bottom:14px">{rightNowText}</div>

{#if friend}
	<div class="row" style="border-top:1px solid var(--rule)">
		<div class="txt">
			<div class="t">{friend.name}</div>
			<div class="d">
				{h.usage[friend.id]?.reqs ?? 0} question{(h.usage[friend.id]?.reqs ?? 0) === 1 ? '' : 's'} answered
				· {Math.round(((h.usage[friend.id]?.gpuSec ?? 0) / 60) * 10) / 10} min of graphics card time
			</div>
		</div>
		<div class="ctl">
			<button class="btn sm danger" onclick={() => hostBackend.unpair(friend.id)}>Remove</button>
		</div>
	</div>
{:else}
	<div class="banner calm">
		No one is allowed to connect yet. When someone asks, you'll get a message here with a code to
		check.
	</div>
{/if}
