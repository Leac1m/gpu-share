<script lang="ts">
	import { hostBackend } from '../backend';
	import { ui } from '../ui.svelte';
	import { minutes, ordinal } from '../format';
	import StatusChip from './StatusChip.svelte';
	import Switch from './Switch.svelte';
	import { hardware } from '../hardware';

	const h = hostBackend.view;

	const hw = $derived(hardware(h));
	const vram = $derived(hw.totalGB);
	const modelName = (id: string | null) => h.catalog.find((m) => m.id === id)?.name ?? id ?? '';
	const whoName = (id: string) => h.paired.find((p) => p.id === id)?.name ?? 'A friend';
	const totalMins = $derived(minutes(Object.values(h.usage).reduce((a, u) => a + u.gpuSec, 0)));
	const temp = $derived(h.temp);
	const tempBarClass = $derived(temp === null ? '' : temp >= h.settings.tempPause ? 'hot' : temp >= 70 ? 'warm' : '');
	const switchHint = $derived(
		h.sharing
			? 'Your computer is visible to friends you have allowed.'
			: hw.card
				? 'Your computer is invisible and your graphics card is untouched.'
				: "Your computer is invisible and the AI engine isn't running."
	);
	const rightNowText = $derived.by(() => {
		const waiting = h.queue.length
			? ' ' + h.queue.length + (h.queue.length === 1 ? ' more question is' : ' more questions are') + ' waiting.'
			: '';
		if (h.state === 'off') return 'Nothing is running. The app is idle and your ' + hw.short + ' is free.';
		if (h.state === 'paused') return 'Nothing is running. The model was unloaded when sharing paused.';
		if (h.job?.phase === 'loading')
			return 'Loading ' + modelName(h.loading) + ' ' + hw.onto + ' for ' + whoName(h.job.who) + '.' + waiting;
		if (h.job) return whoName(h.job.who) + ' is chatting with ' + modelName(h.loaded) + '.' + waiting;
		if (h.loaded) return modelName(h.loaded) + ' is loaded, waiting for the next question.';
		return 'Ready. Nothing is loaded until someone asks a question.';
	});

	function nowChip(id: string): { cls: string; text: string } | null {
		if (h.job?.who === id) return { cls: 'busy', text: h.job.phase === 'loading' ? 'Loading model' : 'Being answered' };
		const pos = h.queue.indexOf(id);
		if (pos >= 0) return { cls: 'paused', text: 'Waiting, ' + ordinal(pos + (h.job ? 2 : 1)) + ' in line' };
		return null;
	}
</script>

<div class="bigswitch">
	<Switch checked={h.sharing} onToggle={() => hostBackend.setSharing(!h.sharing)} label={hw.shareLabel} />
	<div>
		<div class="lbl">{hw.shareLabel}</div>
		<div class="hint">{switchHint}</div>
	</div>
	<div style="margin-left:auto"><StatusChip state={h.state} /></div>
</div>

{#if h.engineSetup}
	<div class="banner info">
		<div>
			<b>Getting the AI engine ready</b>A one-time download{hw.card ? ' for your graphics card' : ''}. Sharing
			starts once it's done.
			<div class="progress" style="margin-top:8px"><i style="width:{h.engineSetup.pct}%"></i></div>
			<div class="meta mono">{h.engineSetup.stage}</div>
		</div>
	</div>
{/if}

{#if !hw.card}
	<div class="banner calm">
		<div>
			<b>No graphics card found</b>Answers come from this computer's processor, so expect a few words a
			second. Add a small model such as Qwen3 0.6B or 1.7B on the Models tab.
		</div>
	</div>
{/if}

{#if h.networkWarning && h.sharing}
	<div class="banner warn"><div><b>Friends can't reach this computer</b>{h.networkWarning}</div></div>
{/if}

{#if h.state === 'paused'}
	<div class="banner warn">
		<div>
			<b>Sharing is paused</b>{h.reason}. It starts again on its own. Friends just see that you're busy.
		</div>
		<button class="act" onclick={() => (ui.hostTab = 'limits')}>Change this rule</button>
	</div>
{:else if h.state === 'off' && !h.sharing && h.paired.length}
	<div class="banner calm">Sharing is switched off. None of your friends can reach your computer.</div>
{/if}

<div class="readouts">
	<div class="ro">
		<div class="k">{hw.memoryLabel}</div>
		<div class="v mono">{h.vramUsedGB.toFixed(1)}<small> / {vram.toFixed(1)} GB</small></div>
		<div class="meter"><i style="width:{(h.vramUsedGB / vram) * 100}%"></i></div>
	</div>
	{#if hw.card}
	<div class="ro">
		<div class="k">Temperature</div>
		<div class="v mono">{temp === null ? '—' : Math.round(temp) + '°C'}</div>
		<div class="meter">
			<i class={tempBarClass} style="width:{temp === null ? 0 : Math.min(100, ((temp - 30) / 60) * 100)}%"></i>
		</div>
	</div>
	{/if}
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

{#each h.paired as p (p.id)}
	{@const u = h.usage[p.id] ?? { reqs: 0, tokIn: 0, tokOut: 0, gpuSec: 0 }}
	{@const now = nowChip(p.id)}
	<div class="row" style="border-top:1px solid var(--rule)">
		<div class="txt">
			<div class="t">{p.name}</div>
			<div class="d">
				{u.reqs} question{u.reqs === 1 ? '' : 's'} answered · {minutes(u.gpuSec)} min of {hw.thing} time
			</div>
		</div>
		<div class="ctl">
			{#if now}<span class="chip {now.cls}"><i></i>{now.text}</span>{/if}
			<button class="btn sm danger" onclick={() => hostBackend.unpair(p.id)}>Remove</button>
		</div>
	</div>
{:else}
	<div class="banner calm">
		No one is allowed to connect yet. When someone asks, you'll get a message here with a code to
		check.
	</div>
{/each}
