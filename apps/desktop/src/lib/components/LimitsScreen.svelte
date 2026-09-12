<script lang="ts">
	import { hostBackend, APPS_ALL } from '../mock/backend.svelte';
	import { model, vramFor } from '../mock/catalog';
	import Switch from './Switch.svelte';

	const h = hostBackend.host;
	const w = hostBackend.world;

	const offeredFirst = $derived(model(hostBackend.offeredModels()[0] || 'qwen25-7b')!);
	const need = $derived(vramFor(offeredFirst, h.ctx));
	const remainingApps = $derived(APPS_ALL.filter((a) => !h.pauseApps.includes(a)));

	function onBudgetInput(e: Event) {
		hostBackend.setBudget(parseFloat((e.target as HTMLInputElement).value));
	}
	function onAwayInput(e: Event) {
		hostBackend.setAwayMin(parseFloat((e.target as HTMLInputElement).value));
	}
	function onTempInput(e: Event) {
		hostBackend.setTempPause(parseFloat((e.target as HTMLInputElement).value));
	}
	function onCtxChange(e: Event) {
		hostBackend.setCtx(parseInt((e.target as HTMLSelectElement).value, 10));
	}
	function onMaxReplyChange(e: Event) {
		hostBackend.setMaxReply(parseInt((e.target as HTMLSelectElement).value, 10));
	}
	function onHoursModeChange(e: Event) {
		hostBackend.setHoursMode((e.target as HTMLSelectElement).value as 'any' | 'schedule');
	}
	function onAppAddChange(e: Event) {
		const el = e.target as HTMLSelectElement;
		if (el.value) {
			hostBackend.addPauseApp(el.value);
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
		<div class="t">Graphics card memory it may use</div>
		<div class="d">
			Your card has 8 GB. Your current model needs about {need.toFixed(1)} GB. Leave room for whatever
			you're working on.
		</div>
	</div>
	<div class="ctl">
		<input type="range" min="2" max="7.5" step="0.5" value={h.budget} oninput={onBudgetInput} />
		<span class="val mono">{h.budget.toFixed(1)} GB</span>
	</div>
</div>

<div class="row">
	<div class="txt">
		<div class="t">How much of a conversation the model remembers</div>
		<div class="d">
			Longer conversations use more memory on your card. {h.ctx} words' worth uses about {(
				offeredFirst.kvGB *
				(h.ctx / 4096)
			).toFixed(1)} GB.
		</div>
	</div>
	<div class="ctl">
		<select onchange={onCtxChange}>
			{#each [2048, 4096, 8192] as c (c)}
				<option value={c} selected={c === h.ctx}>{c}</option>
			{/each}
		</select>
	</div>
</div>

<div class="row">
	<div class="txt">
		<div class="t">Longest answer it will give</div>
		<div class="d">Stops very long answers from tying up your card.</div>
	</div>
	<div class="ctl">
		<select onchange={onMaxReplyChange}>
			{#each [512, 1024, 2048] as c (c)}
				<option value={c} selected={c === h.maxReply}>{c} words</option>
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
	</div>
	<div class="ctl">
		<Switch checked={h.pauseWhenInUse} size="sm" onToggle={() => hostBackend.toggleBoolField('pauseWhenInUse')} />
	</div>
</div>

{#if h.pauseWhenInUse}
	<div class="row">
		<div class="txt">
			<div class="t">Wait this long after I stop typing</div>
			<div class="d">
				In this preview a minute passes in about a second, so you can watch it count down.
			</div>
		</div>
		<div class="ctl">
			<input type="range" min="1" max="15" step="1" value={h.awayMin} oninput={onAwayInput} />
			<span class="val mono">{h.awayMin} min</span>
		</div>
	</div>
{/if}

<div class="row">
	<div class="txt">
		<div class="t">Pause when these programs are open</div>
		<div class="d">Add anything you don't want sharing to interfere with.</div>
		<div class="apps">
			{#each h.pauseApps as a (a)}
				<span class="apptag {w.apps.includes(a) ? 'live' : ''}"
					>{a}<button aria-label={'Remove ' + a} onclick={() => hostBackend.removePauseApp(a)}>×</button
					></span
				>
			{/each}
			{#if remainingApps.length}
				<select style="font-size:12.5px" onchange={onAppAddChange}>
					<option value="">Add a program…</option>
					{#each remainingApps as a (a)}
						<option value={a}>{a}</option>
					{/each}
				</select>
			{/if}
		</div>
	</div>
</div>

<div class="row">
	<div class="txt">
		<div class="t">Pause when anything else uses the graphics card</div>
		<div class="d">A safety net for programs you haven't listed above.</div>
	</div>
	<div class="ctl">
		<Switch checked={h.pauseOnGpuUse} size="sm" onToggle={() => hostBackend.toggleBoolField('pauseOnGpuUse')} />
	</div>
</div>

<div class="row">
	<div class="txt">
		<div class="t">Pause if the card gets hot</div>
		<div class="d">Stops at {h.tempPause}°C and waits until it cools to {h.tempResume}°C.</div>
	</div>
	<div class="ctl">
		<input type="range" min="70" max="88" step="1" value={h.tempPause} oninput={onTempInput} />
		<span class="val mono">{h.tempPause}°C</span>
	</div>
</div>

<div class="row">
	<div class="txt">
		<div class="t">Only when the charger is plugged in</div>
		<div class="d">Keeps sharing off your battery.</div>
	</div>
	<div class="ctl">
		<Switch checked={h.pluggedOnly} size="sm" onToggle={() => hostBackend.toggleBoolField('pluggedOnly')} />
	</div>
</div>

<div class="row">
	<div class="txt">
		<div class="t">Hours when sharing is allowed</div>
		<div class="d">
			{h.hoursMode === 'any'
				? 'Any time of day.'
				: 'Only between ' +
					hostBackend.fmtHour(h.hoursFrom) +
					' and ' +
					hostBackend.fmtHour(h.hoursTo) +
					'.'}
		</div>
	</div>
	<div class="ctl">
		<select onchange={onHoursModeChange}>
			<option value="any" selected={h.hoursMode === 'any'}>Any time</option>
			<option value="schedule" selected={h.hoursMode === 'schedule'}>Evenings and nights</option>
		</select>
	</div>
</div>

<div class="row">
	<div class="txt">
		<div class="t">Keep sharing on after I restart</div>
		<div class="d">Off means sharing always starts switched off when you turn the laptop on.</div>
	</div>
	<div class="ctl">
		<Switch
			checked={h.resumeAfterRestart}
			size="sm"
			onToggle={() => hostBackend.toggleBoolField('resumeAfterRestart')}
		/>
	</div>
</div>
