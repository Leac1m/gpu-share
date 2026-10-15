<script lang="ts">
	import { hostBackend, isPreview } from '../backend';
	import { approxWords, fmtHour, fmtNum, SUGGESTED_PAUSE_APPS, vramFor } from '../format';
	import Switch from './Switch.svelte';
	import { hardware } from '../hardware';

	const h = hostBackend.view;
	const st = $derived(h.settings);
	const update = hostBackend.updateSettings;

	const current = $derived(
		h.catalog.find((m) => m.id === hostBackend.offeredModels()[0]) ??
			h.catalog.find((m) => h.installed.includes(m.id)) ??
			h.catalog[0]
	);
	const need = $derived(current ? vramFor(current, st.ctx) : 0);
	const kv = $derived(current ? current.kvGB * (st.ctx / 4096) : 0);
	const hw = $derived(hardware(h));
	const vram = $derived(hw.totalGB);
	const remainingApps = $derived(
		[...new Set([...h.runningApps, ...SUGGESTED_PAUSE_APPS])].filter((a) => !st.pauseApps.includes(a))
	);

	const num = (e: Event) => parseFloat((e.target as HTMLInputElement).value);

	function onAppAdd(e: Event) {
		const el = e.target as HTMLSelectElement;
		if (el.value) {
			update({ pauseApps: [...st.pauseApps, el.value] });
			el.value = '';
		}
	}
</script>

<h4 class="hd">Your limits</h4>
<p class="sub">
	These rules decide when the app is allowed to run and how much of your machine it may use. You
	can change them any time.
</p>

<div class="row">
	<div class="txt">
		<div class="t">{hw.card ? 'Graphics card memory' : 'Memory'} it may use</div>
		<div class="d">
			{hw.card ? 'Your card has' : 'This computer has'} {vram.toFixed(0)} GB. Your current model needs about {need.toFixed(1)}
			GB. Leave room for whatever you're working on.
		</div>
	</div>
	<div class="ctl">
		<input
			type="range"
			min="1"
			max={Math.max(1, vram - (hw.card ? 0.5 : 2))}
			step="0.5"
			value={st.budget}
			oninput={(e) => update({ budget: num(e) })}
		/>
		<span class="val mono">{st.budget.toFixed(1)} GB</span>
	</div>
</div>

<div class="row">
	<div class="txt">
		<div class="t">How much of a conversation the model remembers</div>
		<div class="d">
			Measured in tokens, the small pieces of words a model reads. 1,000 tokens is about 750 words.
			Longer conversations use more memory{hw.card ? ' on your card' : ''}: {fmtNum(st.ctx)} tokens uses about {kv.toFixed(1)} GB.
		</div>
	</div>
	<div class="ctl">
		<select onchange={(e) => update({ ctx: num(e) })}>
			{#each [2048, 4096, 8192, 16384] as c (c)}
				<option value={c} selected={c === st.ctx}>{fmtNum(c)} tokens · about {approxWords(c)} words</option>
			{/each}
		</select>
	</div>
</div>

<div class="row">
	<div class="txt">
		<div class="t">Longest answer it will give</div>
		<div class="d">Stops very long answers from tying up your {hw.short} while other friends wait.</div>
	</div>
	<div class="ctl">
		<select onchange={(e) => update({ maxReply: num(e) })}>
			{#each [512, 1024, 2048] as c (c)}
				<option value={c} selected={c === st.maxReply}>{fmtNum(c)} tokens · about {approxWords(c)} words</option>
			{/each}
		</select>
	</div>
</div>

<div class="rule"></div>

<div class="row">
	<div class="txt">
		<div class="t">Pause while I'm using the computer</div>
		<div class="d">
			Sharing stops the moment you touch the keyboard, and starts again once you've been away for a
			while.
		</div>
		{#if !h.idleKnown && !isPreview}
			<div class="d" style="color:var(--halt)">
				This computer can't tell when you're using it (only Ubuntu's standard desktop, GNOME, says), so this
				rule has no effect here.
			</div>
		{/if}
	</div>
	<div class="ctl">
		<Switch checked={st.pauseWhenInUse} size="sm" onToggle={() => update({ pauseWhenInUse: !st.pauseWhenInUse })} />
	</div>
</div>

{#if st.pauseWhenInUse}
	<div class="row">
		<div class="txt">
			<div class="t">Wait this long after I stop typing</div>
			<div class="d">
				{#if isPreview}In this preview a minute passes in about a second, so you can watch it count
					down.{:else}Sharing starts again once you've been away this long.{/if}
			</div>
		</div>
		<div class="ctl">
			<input type="range" min="1" max="15" step="1" value={st.awayMin} oninput={(e) => update({ awayMin: num(e) })} />
			<span class="val mono">{st.awayMin} min</span>
		</div>
	</div>
{/if}

<div class="row">
	<div class="txt">
		<div class="t">Pause when these programs are open</div>
		<div class="d">
			Add anything you don't want sharing to interfere with. Friends only see that you're busy, never
			which program.
		</div>
		<div class="apps">
			{#each st.pauseApps as a (a)}
				<span class="apptag {h.runningApps.includes(a) ? 'live' : ''}"
					>{a}<button
						aria-label={'Remove ' + a}
						onclick={() => update({ pauseApps: st.pauseApps.filter((x) => x !== a) })}>×</button
					></span
				>
			{/each}
			{#if remainingApps.length}
				<select style="font-size:12.5px" onchange={onAppAdd}>
					<option value="">Add a program…</option>
					{#each remainingApps as a (a)}
						<option value={a}>{a}</option>
					{/each}
				</select>
			{/if}
		</div>
	</div>
</div>

{#if hw.card}
<div class="row">
	<div class="txt">
		<div class="t">Pause when anything else uses the graphics card</div>
		<div class="d">A safety net for programs you haven't listed above.</div>
	</div>
	<div class="ctl">
		<Switch checked={st.pauseOnGpuUse} size="sm" onToggle={() => update({ pauseOnGpuUse: !st.pauseOnGpuUse })} />
	</div>
</div>

<div class="row">
	<div class="txt">
		<div class="t">Pause if the card gets hot</div>
		<div class="d">Stops at {st.tempPause}°C and waits until it cools to {st.tempResume}°C.</div>
	</div>
	<div class="ctl">
		<input
			type="range"
			min="70"
			max="88"
			step="1"
			value={st.tempPause}
			oninput={(e) => update({ tempPause: num(e), tempResume: num(e) - 8 })}
		/>
		<span class="val mono">{st.tempPause}°C</span>
	</div>
</div>
{/if}

<div class="row">
	<div class="txt">
		<div class="t">Only when the charger is plugged in</div>
		<div class="d">Keeps sharing off your battery.</div>
	</div>
	<div class="ctl">
		<Switch checked={st.pluggedOnly} size="sm" onToggle={() => update({ pluggedOnly: !st.pluggedOnly })} />
	</div>
</div>

<div class="row">
	<div class="txt">
		<div class="t">Hours when sharing is allowed</div>
		<div class="d">
			{st.hoursMode === 'any'
				? 'Any time of day.'
				: 'Only between ' + fmtHour(st.hoursFrom) + ' and ' + fmtHour(st.hoursTo) + '.'}
		</div>
	</div>
	<div class="ctl">
		<select onchange={(e) => update({ hoursMode: (e.target as HTMLSelectElement).value as 'any' | 'schedule' })}>
			<option value="any" selected={st.hoursMode === 'any'}>Any time</option>
			<option value="schedule" selected={st.hoursMode === 'schedule'}>Evenings and nights</option>
		</select>
	</div>
</div>

<div class="row">
	<div class="txt">
		<div class="t">Keep sharing on after I restart</div>
		<div class="d">Off means sharing always starts switched off when you turn the computer on.</div>
	</div>
	<div class="ctl">
		<Switch
			checked={st.resumeAfterRestart}
			size="sm"
			onToggle={() => update({ resumeAfterRestart: !st.resumeAfterRestart })}
		/>
	</div>
</div>
