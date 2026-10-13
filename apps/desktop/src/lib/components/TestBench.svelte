<script lang="ts">
	// Pretend real life around Dara's laptop. Preview only: not part of the installed app.
	import { hostBackend } from '../backend';
	import { awayLeft, BENCH_APPS, sim, SIM_MINUTE } from '../mock/backend.svelte';
	import { fmtHour } from '../format';

	const w = sim.world;
	const h = hostBackend.view;

	// awayLeft() reads Date.now(), so re-evaluate it on the host view's tick
	const awayLeftMin = $derived((void h.state, void h.countdown, void h.reason, Math.ceil(awayLeft() / SIM_MINUTE)));
	const friends = $derived((void h.paired.length, void h.pending, void h.queue, void h.job, void h.sharing, sim.friendActions()));
	const lineup = $derived((void h.queue, void h.job, sim.lineup()));

	const hourButtons = [
		{ hr: 10, label: '10am' },
		{ hr: 15, label: '3pm' },
		{ hr: 21, label: '9pm' },
		{ hr: 2, label: '2am' }
	];
</script>

<section class="testbench">
	<h2>Test bench</h2>
	<p class="tbnote">
		These switches pretend to be real life around Dara's laptop. They are not part of the app he would
		install. Reply speed and model loading run at the estimated speed for an RTX A2000. Rules counted
		in minutes run faster here so you don't have to wait five real minutes to see them work.
	</p>
	<div class="controls">
		<div class="cgroup">
			<h3>Is he at his desk?</h3>
			<div class="cbtns">
				<button class="cbtn" aria-pressed={w.atDesk} onclick={() => sim.setAtDesk(!w.atDesk)}>
					{w.atDesk ? 'He walked away' : "He's using the laptop"}
				</button>
			</div>
			<div class="creadout">
				{#if w.atDesk}
					He is at the keyboard.
				{:else if awayLeftMin > 0 && h.settings.pauseWhenInUse && h.sharing}
					Away. Sharing resumes in <b>{awayLeftMin} min</b>.
				{:else}
					Away from the laptop.
				{/if}
			</div>
		</div>
		<div class="cgroup">
			<h3>What has he opened?</h3>
			<div class="cbtns">
				{#each BENCH_APPS as a (a)}
					<button class="cbtn" aria-pressed={w.apps.includes(a)} onclick={() => sim.toggleApp(a)}>{a}</button>
				{/each}
			</div>
			<div class="creadout">
				{#if w.apps.length}Open: <b>{w.apps.join(', ')}</b>{:else}Nothing heavy is open.{/if}
			</div>
		</div>
		<div class="cgroup">
			<h3>Power and heat</h3>
			<div class="cbtns">
				<button class="cbtn" aria-pressed={w.battery} onclick={() => sim.setBattery(!w.battery)}>
					{w.battery ? 'Plug the charger back in' : 'Unplug the charger'}
				</button>
				<button class="cbtn" aria-pressed={w.heat} onclick={() => sim.setHeat(!w.heat)}>
					{w.heat ? 'Clear the fan vent' : 'Block the fan vent'}
				</button>
			</div>
			<div class="creadout">
				{#if w.battery}On <b>battery</b>{:else}Plugged in{/if}, card at <b>{Math.round(h.temp ?? 0)}°C</b>
				{#if w.heat}<br />Heat only builds while the card is answering something.{/if}
			</div>
		</div>
		<div class="cgroup">
			<h3>Time of day</h3>
			<div class="cbtns">
				{#each hourButtons as b (b.hr)}
					<button class="cbtn" aria-pressed={w.hour === b.hr} onclick={() => sim.setHour(b.hr)}>{b.label}</button>
				{/each}
			</div>
			<div class="creadout">It is <b>{fmtHour(w.hour)}</b> on his laptop.</div>
		</div>
		<div class="cgroup">
			<h3>Other friends on the Wi-Fi</h3>
			<div class="cbtns">
				{#each friends as b (b.f.id)}
					<button
						class="cbtn"
						disabled={b.off}
						style={b.off ? 'opacity:.5;cursor:not-allowed' : ''}
						onclick={() => sim.friendDo(b.f.id, b.act)}>{b.label}</button
					>
				{/each}
			</div>
			<div class="creadout">
				{lineup}
				{#if !h.sharing}<br />Turn sharing on first, so they can find his laptop.{/if}
				{#if sim.bench.note}<br />{sim.bench.note}{/if}
			</div>
		</div>
	</div>
</section>
