<script lang="ts">
	// The browser preview (prototype build 0.2): both machines side by side over the mock
	// backend, with the test bench underneath. Every screen here is labelled as a preview.
	import HostMachine from './lib/components/HostMachine.svelte';
	import ClientMachine from './lib/components/ClientMachine.svelte';
	import Link from './lib/components/Link.svelte';
	import TestBench from './lib/components/TestBench.svelte';
	import { onPaired, sim, start } from './lib/mock/backend.svelte';
	import { ui } from './lib/ui.svelte';

	onPaired(() => (ui.clientTab = 'chat'));
	start();

	let theme = $state<'light' | 'dark'>(
		window.matchMedia?.('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
	);

	$effect(() => {
		document.documentElement.dataset.theme = theme;
	});

	// Like the prototype, an OS-level scheme change always wins, even after a manual pick.
	$effect(() => {
		if (!window.matchMedia) return;
		const mq = window.matchMedia('(prefers-color-scheme: dark)');
		const handler = (e: MediaQueryListEvent) => (theme = e.matches ? 'dark' : 'light');
		mq.addEventListener('change', handler);
		return () => mq.removeEventListener('change', handler);
	});

	function showTour() {
		alert(
			'Try it in this order:\n\n' +
				"1. On Dara's laptop, press Set up sharing, then turn on Share my graphics card.\n" +
				'2. On your PC, his laptop appears. Press Ask to connect.\n' +
				'3. Back on his laptop, check the code and press Allow.\n' +
				'4. Ask a question. The first one waits while the model loads onto his card.\n' +
				"5. In the test bench, let Tunde connect and ask a question, then ask one yourself straight after. You'll see where you are in line.\n" +
				"6. While someone is being answered, open Blender or press He's using the laptop.\n\n" +
				'Watch what happens to his graphics card memory, and what each side sees.'
		);
	}

	function startOver() {
		ui.hostTab = 'share';
		ui.clientTab = 'devices';
		ui.trayOpen = false;
		sim.reset();
	}
</script>

<header class="topbar">
	<div class="brand">GPU<span>Share</span></div>
	<div class="topnote">
		Interactive preview. Nothing here is real yet, but everything behaves the way the finished app
		will.
	</div>
	<div class="spacer"></div>
	<button class="tbtn" onclick={() => (theme = theme === 'dark' ? 'light' : 'dark')}>
		{theme === 'dark' ? 'Light' : 'Dark'}
	</button>
	<button class="tbtn" onclick={showTour}>How to use this preview</button>
	<button class="tbtn" onclick={startOver}>Start over</button>
</header>

<main class="scene">
	<div class="lede">
		<h1>Several friends, one graphics card.</h1>
		<p>
			On the left is Dara's Windows laptop with the RTX A2000. On the right is your Ubuntu PC. Turn on
			sharing, connect the two, and chat. Use the test bench underneath to bring in other friends who
			share the same card, and to see what happens when Dara needs his computer back.
		</p>
	</div>

	<div class="bench">
		<HostMachine />
		<Link />
		<ClientMachine />
	</div>

	<TestBench />
</main>

<div class="foot">
	Preview build 0.2 · Reply speeds and model load times are estimates for an NVIDIA RTX A2000 8 GB.
	They will be measured on Dara's laptop in Phase 5 and updated here.
</div>
