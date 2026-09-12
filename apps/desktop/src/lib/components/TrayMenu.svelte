<script lang="ts">
	import { hostBackend } from '../mock/backend.svelte';
</script>

{#if hostBackend.host.trayOpen}
	<div class="traymenu">
		<button onclick={() => hostBackend.trayShare()}>
			{hostBackend.host.sharing ? 'Stop sharing' : 'Start sharing'}
		</button>
		<button onclick={() => hostBackend.traySettings()}>Open settings</button>
		<div class="sep"></div>
		<button onclick={() => hostBackend.trayQuit()}>Quit GPU Share</button>
	</div>
{/if}
