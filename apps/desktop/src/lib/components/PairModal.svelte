<script lang="ts">
	import { hostBackend } from '../mock/backend.svelte';
</script>

{#if hostBackend.host.pending}
	{@const p = hostBackend.host.pending}
	<div class="modal">
		<div class="card">
			<h3>{p.name} wants to use your graphics card</h3>
			<p>Check that your friend sees this same code before you allow it.</p>
			<div class="code mono">{p.code}</div>
			<div class="acts">
				<button class="btn" onclick={() => hostBackend.pairDeny()}>Not now</button>
				<button class="btn pri" onclick={() => hostBackend.pairAllow()}>Allow</button>
			</div>
			<p style="margin:13px 0 0;font-size:12.5px">
				Allowing this lets them ask questions. It does not give them access to anything else on your
				computer.
			</p>
		</div>
	</div>
{/if}
