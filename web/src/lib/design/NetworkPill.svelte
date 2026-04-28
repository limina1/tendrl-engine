<script lang="ts">
	type Props = { online?: boolean };
	let { online = true }: Props = $props();
</script>

<span class="pill {online ? 'pill--online' : 'pill--offline'}">
	<span class="dot {online ? 'dot--online' : 'dot--offline'}"></span>
	{online ? 'online' : 'offline'}
</span>
