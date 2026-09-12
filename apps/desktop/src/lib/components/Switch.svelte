<script lang="ts">
	let {
		checked,
		onToggle,
		size = 'md',
		label
	}: {
		checked: boolean;
		onToggle: () => void;
		size?: 'md' | 'sm';
		label?: string;
	} = $props();
</script>

<button
	class="sw {size === 'sm' ? 'sm' : ''}"
	role="switch"
	aria-checked={checked}
	aria-label={label}
	onclick={onToggle}
>
	<i></i>
</button>
