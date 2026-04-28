<script lang="ts">
	export type IdKind = 'yours' | 'imported' | 'forked' | 'diverged' | 'local' | 'remote' | 'draft';

	type Props = { kind?: IdKind };
	let { kind = 'remote' }: Props = $props();
</script>

<div class="id-bar" style:background={`var(--id-${kind})`}></div>
