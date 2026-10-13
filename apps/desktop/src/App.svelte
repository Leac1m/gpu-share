<script lang="ts">
	import { isPreview } from './lib/backend';
	import Preview from './Preview.svelte';
	import RealApp from './RealApp.svelte';
</script>

{#if isPreview}
	<Preview />
{:else}
	<RealApp />
{/if}
