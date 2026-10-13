<script lang="ts">
	// The preview's stand-in for the Windows tray menu. The real app uses the OS tray.
	import { hostBackend } from '../backend';
	import { ui } from '../ui.svelte';

	function close() {
		ui.trayOpen = false;
	}
</script>

{#if ui.trayOpen}
	<div class="traymenu">
		<button
			onclick={() => {
				close();
				hostBackend.setSharing(!hostBackend.view.sharing);
			}}
		>
			{hostBackend.view.sharing ? 'Stop sharing' : 'Start sharing'}
		</button>
		<button
			onclick={() => {
				close();
				ui.hostTab = 'limits';
				hostBackend.welcomeDone();
			}}>Open settings</button
		>
		<div class="sep"></div>
		<button
			onclick={() => {
				close();
				hostBackend.quit();
			}}>Quit GPU Share</button
		>
	</div>
{/if}
