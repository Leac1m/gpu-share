<script lang="ts">
	import { hostBackend } from '../mock/backend.svelte';
</script>

<div class="toasts">
	{#each hostBackend.host.toasts as t (t.id)}
		<div class="toast"><b>{t.title}</b>{t.body}</div>
	{/each}
</div>
