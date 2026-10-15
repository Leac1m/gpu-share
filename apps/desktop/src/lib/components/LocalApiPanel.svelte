<script lang="ts">
	// The local OpenAI-compatible address (P9-T2): other programs on this computer can use the
	// models of the computers it's paired with (and its own, on a host).
	import { clientBackend } from '../backend';
	import Switch from './Switch.svelte';

	const c = clientBackend.view;
	const api = $derived(c.localApi);
	const modelIds = $derived([...new Set(c.hosts.filter((h) => h.paired).flatMap((h) => h.models.map((m) => m.id)))]);
	let copied = $state('');

	async function copy(what: string, text: string) {
		try {
			await navigator.clipboard.writeText(text);
			copied = what;
			setTimeout(() => (copied = ''), 1500);
		} catch {
			copied = '';
		}
	}
</script>

<div class="rule"></div>
<div class="row" style="border:0">
	<div class="txt">
		<div class="t">Use from other apps</div>
		<div class="d">
			Editors, scripts and AI agents on this computer can use the models below through an OpenAI-compatible
			address. Their questions wait in line like yours, within each owner's limits. Only programs on this
			computer that have the key can use it.
		</div>
	</div>
	<div class="ctl">
		<Switch checked={api.enabled} size="sm" label="Use from other apps" onToggle={() => clientBackend.setLocalApi(!api.enabled)} />
	</div>
</div>

{#if api.enabled}
	{#if api.error}
		<div class="banner warn"><div>{api.error}</div></div>
	{:else}
		<div class="apibox">
			<div class="k">Base URL</div>
			<div class="v mono">{api.url}</div>
			<button class="btn sm" onclick={() => copy('url', api.url)}>{copied === 'url' ? 'Copied' : 'Copy'}</button>
			<div class="k">API key</div>
			<div class="v mono">{api.key}</div>
			<button class="btn sm" onclick={() => copy('key', api.key)}>{copied === 'key' ? 'Copied' : 'Copy'}</button>
			<div class="k">Models</div>
			<div class="v mono">{modelIds.length ? modelIds.join(', ') : 'none yet: pair with a computer first'}</div>
			<span></span>
		</div>
		<p class="sub" style="margin:8px 0 0">
			Tool calling works. Small models are weaker at it than big ones. If a program says the conversation is too
			long, the computer's owner can raise "How much of a conversation the model remembers".
			<button class="btn sm" style="margin-left:6px" onclick={() => clientBackend.newLocalApiKey()}>New key</button>
		</p>
	{/if}
{/if}

<style>
	.apibox {
		display: grid;
		grid-template-columns: auto 1fr auto;
		gap: 8px 12px;
		align-items: center;
		padding: 12px 14px;
		border: 1px solid var(--rule);
		border-radius: 10px;
	}
	.apibox .k {
		font-size: 12.5px;
		color: var(--ink-faint);
	}
	.apibox .v {
		font-size: 12.5px;
		overflow-wrap: anywhere;
	}
</style>
