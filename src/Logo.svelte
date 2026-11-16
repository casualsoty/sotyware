<script lang="ts">
  // sotyware's mark: the cube on the blue-to-teal tile of its app icon
  // (icons/source/*.svg). Decoration only: the name is always next to it.
  interface Props {
    size?: number;
  }

  let { size = 22 }: Props = $props();
  // Each instance needs its own gradient id: two logos on one page must not
  // share (or fight over) one.
  const uid = $props.id();
  const gradient = `brand-tile-${uid}`;
</script>

<svg width={size} height={size} viewBox="0 0 32 32" aria-hidden="true">
  <defs>
    <linearGradient id={gradient} x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#2b6cf0" />
      <stop offset="1" stop-color="#0f8b8d" />
    </linearGradient>
  </defs>
  <rect x="1" y="1" width="30" height="30" rx="7" fill="url(#{gradient})" />
  <path d="M16 5 L26 10 L16 15 L6 10 Z" fill="#fff" />
  <path d="M6 10 L16 15 V27 L6 22 Z" fill="#fff" fill-opacity=".78" />
  <path d="M26 10 L16 15 V27 L26 22 Z" fill="#fff" fill-opacity=".54" />
</svg>
