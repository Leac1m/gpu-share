<script lang="ts">
	import { hostBackend } from '../mock/backend.svelte';
</script>

<div class="welcome">
	<h3>Lend your graphics card to a friend</h3>
	<p class="sub">
		Your RTX A2000 sits idle most of the day. This app lets one friend on your Wi-Fi borrow it to
		run AI chat models, and only when you say so.
	</p>
	<div class="points">
		<div class="wpoint">
			<div class="n">1</div>
			<div>
				<div class="t">One switch, and it's yours again</div>
				<div class="d">
					Turn sharing off and the app stops completely. Your graphics card is freed and your
					computer disappears from your friend's screen.
				</div>
			</div>
		</div>
		<div class="wpoint">
			<div class="n">2</div>
			<div>
				<div class="t">It gets out of your way on its own</div>
				<div class="d">
					Open Blender, unplug the charger, or just sit down at your desk, and sharing pauses
					without you doing anything.
				</div>
			</div>
		</div>
		<div class="wpoint">
			<div class="n">3</div>
			<div>
				<div class="t">It cannot reach your files</div>
				<div class="d">
					The app has no way to open, read, copy or send your files, and no one can see your screen
					or control your computer. It answers questions and nothing else.
				</div>
			</div>
		</div>
	</div>
	<button class="btn pri" onclick={() => hostBackend.welcomeDone()}>Set up sharing</button>
	<p class="sub" style="margin-top:14px;margin-bottom:0">
		You can change every setting later, and you'll be asked before anyone is allowed to connect.
	</p>
</div>
