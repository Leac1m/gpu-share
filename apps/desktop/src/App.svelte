<script lang="ts">
	import HostMachine from './lib/components/HostMachine.svelte';
	import ClientMachine from './lib/components/ClientMachine.svelte';
	import Link from './lib/components/Link.svelte';
	import TestBench from './lib/components/TestBench.svelte';
	import { hostBackend } from './lib/mock/backend.svelte';

	let theme = $state<'light' | 'dark'>(
		typeof window !== 'undefined' &&
			window.matchMedia &&
			window.matchMedia('(prefers-color-scheme: dark)').matches
			? 'dark'
			: 'light'
	);

	$effect(() => {
		document.documentElement.dataset.theme = theme;
	});

	// Like the prototype, an OS-level scheme change always wins, even after a manual pick.
	$effect(() => {
		if (!window.matchMedia) return;
		const mq = window.matchMedia('(prefers-color-scheme: dark)');
		const handler = (e: MediaQueryListEvent) => {
			theme = e.matches ? 'dark' : 'light';
		};
		mq.addEventListener('change', handler);
		return () => mq.removeEventListener('change', handler);
	});

	function toggleTheme() {
		theme = theme === 'dark' ? 'light' : 'dark';
	}

	function showTour() {
		alert(
			"Try it in this order:\n\n" +
				"1. On Dara's laptop, press Set up sharing, then turn on Share my graphics card.\n" +
				'2. On your PC, his laptop appears. Press Ask to connect.\n' +
				'3. Back on his laptop, check the code and press Allow.\n' +
				'4. Ask a question. The first one waits while the model loads onto his card.\n' +
				"5. While it's answering, use the test bench to open Blender, or press He's using the laptop.\n\n" +
				'Watch what happens to his graphics card memory, and what you see on your side.'
		);
	}

	function startOver() {
		hostBackend.reset();
	}
</script>

<header class="topbar">
	<div class="brand">GPU<span>Share</span></div>
	<div class="topnote">
		Interactive preview. Nothing here is real yet, but everything behaves the way the finished app
		will.
	</div>
	<div class="spacer"></div>
	<button class="tbtn" onclick={toggleTheme}>{theme === 'dark' ? 'Light' : 'Dark'}</button>
	<button class="tbtn" onclick={showTour}>How to use this preview</button>
	<button class="tbtn" onclick={startOver}>Start over</button>
</header>

<main class="scene">
	<div class="lede">
		<h1>Two computers, one graphics card.</h1>
		<p>
			On the left is your friend's Windows machine with the RTX A2000. On the right is your Ubuntu
			PC. Turn on sharing, connect the two, and chat. Use the test bench underneath to see what
			happens when he needs his computer back.
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
	Preview build 0.1 · The performance figures shown are estimates for an NVIDIA RTX A2000 8 GB and
	still need to be measured on the real machine.
</div>
