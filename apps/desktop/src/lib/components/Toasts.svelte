<script lang="ts">
	import { hostBackend } from '../backend';
</script>

<div class="toasts">
	{#each hostBackend.view.toasts as t (t.id)}
		<div class="toast"><b>{t.title}</b>{t.body}</div>
	{/each}
</div>
