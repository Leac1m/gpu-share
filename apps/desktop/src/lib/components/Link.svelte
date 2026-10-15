<script lang="ts">
	// The wire between the two machines in the preview (the prototype's syncLive()).
	import { clientBackend, hostBackend } from '../backend';

	const h = hostBackend.view;
	const c = clientBackend.view;

	const remote = $derived(c.hosts[0]);
	const paired = $derived(!!remote?.paired);

	const state = $derived(
		!paired
			? 'none'
			: h.state === 'paused'
				? 'paused'
				: c.turn && c.turn.phase !== 'queued'
					? 'streaming'
					: h.state === 'off'
						? 'none'
						: 'idle'
	);

	const label = $derived(
		!h.sharing
			? 'not connected'
			: !paired
				? 'waiting to be allowed'
				: h.state === 'paused'
					? 'paused'
					: c.turn?.phase === 'loading'
						? 'loading model'
						: c.turn?.phase === 'streaming'
							? 'answering you'
							: c.turn?.phase === 'queued'
								? "you're in line"
								: remote?.busyWithOther
									? 'answering another friend'
									: 'connected'
	);
</script>

<div class="link" data-state={state}>
	<div class="wifi">home Wi-Fi</div>
	<div class="wire"><span class="pulse"></span></div>
	<div class="linklabel">{label}</div>
</div>
