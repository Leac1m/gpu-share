<script lang="ts">
	import { hostBackend, BENCH_APPS, SIM_MINUTE } from '../mock/backend.svelte';

	const w = hostBackend.world;
	const h = hostBackend.host;
	const rt = hostBackend.rt;

	const awayLeftMin = $derived(Math.ceil(hostBackend.awayLeft() / SIM_MINUTE));

	const hourButtons: { hr: number; label: string }[] = [
		{ hr: 10, label: '10am' },
		{ hr: 15, label: '3pm' },
		{ hr: 21, label: '9pm' },
		{ hr: 2, label: '2am' }
	];
</script>

<section class="testbench">
	<h2>Test bench</h2>
	<p class="tbnote">
		These switches pretend to be real life on Dara's laptop. They are not part of the app he would
		install. Typing speed and model loading run at true speed for an RTX A2000. Rules counted in
		minutes run faster here so you don't have to wait five real minutes to see them work.
	</p>
	<div class="controls">
		<div class="cgroup">
			<h3>Is he at his desk?</h3>
			<div class="cbtns">
				<button class="cbtn" aria-pressed={w.atDesk} onclick={() => hostBackend.simSetAtDesk(!w.atDesk)}>
					{w.atDesk ? 'He walked away' : "He's using the laptop"}
				</button>
			</div>
			<div class="creadout">
				{#if w.atDesk}
					He is at the keyboard.
				{:else if hostBackend.awayLeft() > 0 && h.pauseWhenInUse && h.sharing}
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
					<button class="cbtn" aria-pressed={w.apps.includes(a)} onclick={() => hostBackend.simToggleApp(a)}>
						{a}
					</button>
				{/each}
			</div>
			<div class="creadout">
				{#if w.apps.length}
					Open: <b>{w.apps.join(', ')}</b>
				{:else}
					Nothing heavy is open.
				{/if}
			</div>
		</div>
		<div class="cgroup">
			<h3>Power and heat</h3>
			<div class="cbtns">
				<button class="cbtn" aria-pressed={w.battery} onclick={() => hostBackend.simSetBattery(!w.battery)}>
					{w.battery ? 'Plug the charger back in' : 'Unplug the charger'}
				</button>
				<button class="cbtn" aria-pressed={w.heat} onclick={() => hostBackend.simSetHeat(!w.heat)}>
					{w.heat ? 'Clear the fan vent' : 'Block the fan vent'}
				</button>
			</div>
			<div class="creadout">
				{#if w.battery}On <b>battery</b>{:else}Plugged in{/if}, card at <b>{Math.round(rt.temp)}°C</b>
				{#if w.heat}<br />Heat only builds while the card is answering something.{/if}
			</div>
		</div>
		<div class="cgroup">
			<h3>Time of day</h3>
			<div class="cbtns">
				{#each hourButtons as b (b.hr)}
					<button class="cbtn" aria-pressed={w.hour === b.hr} onclick={() => hostBackend.simSetHour(b.hr)}>
						{b.label}
					</button>
				{/each}
			</div>
			<div class="creadout">It is <b>{hostBackend.fmtHour(w.hour)}</b> on his laptop.</div>
		</div>
	</div>
</section>
