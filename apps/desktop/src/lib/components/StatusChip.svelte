<script lang="ts">
	import type { RuntimeStateName } from '../types';

	let { state }: { state: RuntimeStateName } = $props();

	const labels: Record<RuntimeStateName, string> = {
		off: 'Off',
		available: 'Ready',
		busy: 'In use',
		paused: 'Paused'
	};
</script>

<span class="chip {state}"><i></i>{labels[state]}</span>
