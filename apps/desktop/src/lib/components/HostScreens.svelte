<script lang="ts">
	// Share mode: the welcome screen until it has been seen, then the open tab.
	import { hostBackend } from '../backend';
	import { ui } from '../ui.svelte';
	import WelcomeScreen from './WelcomeScreen.svelte';
	import ShareScreen from './ShareScreen.svelte';
	import LimitsScreen from './LimitsScreen.svelte';
	import ModelsScreen from './ModelsScreen.svelte';
	import AccessScreen from './AccessScreen.svelte';

	const h = hostBackend.view;
</script>

{#if !h.settings.welcomed}
	<WelcomeScreen />
{:else if ui.hostTab === 'share'}
	<ShareScreen />
{:else if ui.hostTab === 'limits'}
	<LimitsScreen />
{:else if ui.hostTab === 'models'}
	<ModelsScreen />
{:else if ui.hostTab === 'access'}
	<AccessScreen />
{/if}
