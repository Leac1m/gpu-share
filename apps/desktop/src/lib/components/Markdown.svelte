<script lang="ts">
	import { renderMarkdown } from '../markdown';

	let { text }: { text: string } = $props();
</script>

<!-- renderMarkdown escapes all input; see markdown.test.ts -->
<!-- eslint-disable-next-line svelte/no-at-html-tags -->
{@html renderMarkdown(text)}
