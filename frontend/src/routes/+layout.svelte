<script lang="ts">
  import '../app.css';

  let { children } = $props();
</script>

<div class="min-h-screen bg-[#f8fafc] text-[#334155]">
  {@render children()}
</div>
