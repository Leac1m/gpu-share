<script lang="ts">
	// The visual wire between the host and client machines. Mirrors the prototype's
	// #link element and syncLive()'s link state derivation.
	import { hostBackend, clientBackend } from '../mock/backend.svelte';

	const h = hostBackend.host;
	const c = clientBackend.client;
	const rt = hostBackend.rt;

	const state = $derived(
		!c.paired
			? 'none'
			: rt.state === 'paused'
				? 'paused'
				: rt.streaming || rt.loading
					? 'streaming'
					: rt.state === 'off'
						? 'none'
						: 'idle'
	);

	const label = $derived(
		!h.sharing
			? 'not connected'
			: !c.paired
				? 'waiting to be allowed'
				: rt.state === 'paused'
					? 'paused'
					: rt.loading
						? 'loading model'
						: rt.streaming
							? 'answering'
							: 'connected'
	);
</script>

<div class="link" data-state={state}>
	<div class="wifi">home Wi-Fi</div>
	<div class="wire"><span class="pulse"></span></div>
	<div class="linklabel">{label}</div>
</div>
