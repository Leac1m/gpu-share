<script lang="ts">
	import { hostBackend } from '../backend';
	import { fmtNum, minutes } from '../format';

	const h = hostBackend.view;
</script>

<h4 class="hd">Who can connect</h4>
<p class="sub">
	Only computers you've allowed. Everyone else on your Wi-Fi sees your computer's name and whether
	sharing is ready, and nothing else. Friends you've allowed take turns: one question is answered at a
	time, in the order they arrive.
</p>
{#each h.paired as p (p.id)}
	{@const u = h.usage[p.id] ?? { reqs: 0, tokIn: 0, tokOut: 0, gpuSec: 0 }}
	<div class="row" style="border-top:1px solid var(--rule)">
		<div class="txt">
			<div class="t">{p.name}</div>
			<div class="d mono">
				{u.reqs} question{u.reqs === 1 ? '' : 's'} · about {fmtNum(u.tokOut)} tokens written · {minutes(u.gpuSec)}
				min of {h.card ? 'card' : 'computer'} time
			</div>
		</div>
		<div class="ctl">
			<button class="btn sm danger" onclick={() => hostBackend.unpair(p.id)}>Remove</button>
		</div>
	</div>
{:else}
	<div class="banner calm">No one yet.</div>
{/each}

<div class="rule"></div>
<h4 class="hd">What this app can and cannot reach</h4>
<p class="sub">This is built into how the app works, not a setting you have to trust us on.</p>
<div class="safety">
	<div class="yes">
		<h4>It can</h4>
		<ul>
			<li>Run models from its own folder</li>
			<li>Answer questions from people you allowed</li>
			<li>Download models you picked</li>
			<li>Count how much each friend used</li>
		</ul>
	</div>
	<div class="no">
		<h4>It cannot</h4>
		<ul>
			<li>Open, read, copy or change any of your files</li>
			<li>Show your screen or control your computer</li>
			<li>Run programs sent from the network</li>
			<li>Keep a copy of anyone's questions or answers</li>
		</ul>
	</div>
</div>
<p class="sub" style="margin-top:14px">
	Your friends' questions are answered on your machine. The app doesn't store them, you never see
	them, and friends never see each other's.
</p>
