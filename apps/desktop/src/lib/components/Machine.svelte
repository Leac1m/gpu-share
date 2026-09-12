<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		os,
		title,
		osName,
		tabs,
		activeTab,
		onTab,
		taskbarLabel,
		screen,
		taskbarRight,
		overlays
	}: {
		os: 'windows' | 'ubuntu';
		title: string;
		osName: string;
		tabs: { id: string; label: string }[];
		activeTab: string;
		onTab: (id: string) => void;
		taskbarLabel: string;
		screen: Snippet;
		taskbarRight?: Snippet;
		overlays?: Snippet;
	} = $props();
</script>

<section class="machine" data-os={os}>
	<div class="titlebar">
		<h2>{title}</h2>
		<span class="osname">{osName}</span>
		<div class="wctl"><i></i><i></i><i></i></div>
	</div>
	<div class="tabs" role="tablist">
		{#each tabs as t (t.id)}
			<button class="tab" role="tab" aria-selected={t.id === activeTab} onclick={() => onTab(t.id)}>
				{t.label}
			</button>
		{/each}
	</div>
	<div class="screen">
		{@render screen()}
	</div>
	<div class="taskbar">
		<span>{taskbarLabel}</span>
		{#if taskbarRight}{@render taskbarRight()}{/if}
	</div>
	{#if overlays}{@render overlays()}{/if}
</section>
