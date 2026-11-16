<script lang="ts" module>
  // sotyware's icon set: simple 24×24 line drawings in the current text
  // colour, drawn for this app (no icon font, no external asset). Icons are
  // decoration: the control next to them always has its own text or
  // aria-label, so every icon is aria-hidden.
  export const ICONS = {
    overview: "M4 11.5 12 5l8 6.5V19a1 1 0 0 1-1 1h-4.5v-5h-5v5H5a1 1 0 0 1-1-1z",
    cloud: "M7.5 18.5h9.8a3.7 3.7 0 0 0 .5-7.37A5.5 5.5 0 0 0 7.2 9.6 4.45 4.45 0 0 0 7.5 18.5z",
    backup: "M4 7.5h16M5.5 7.5V18a1.5 1.5 0 0 0 1.5 1.5h10a1.5 1.5 0 0 0 1.5-1.5V7.5M4.5 4.5h15v3h-15zM10 11.5h4",
    account: "M12 12a3.75 3.75 0 1 0 0-7.5 3.75 3.75 0 0 0 0 7.5zM4.75 19.5c.9-3.3 3.75-5 7.25-5s6.35 1.7 7.25 5",
    settings:
      "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM19.4 13.5l1.4 1.1-1.8 3.1-1.7-.6a7 7 0 0 1-1.9 1.1l-.3 1.8h-3.6l-.3-1.8a7 7 0 0 1-1.9-1.1l-1.7.6-1.8-3.1 1.4-1.1a7 7 0 0 1 0-3L3.8 9.4l1.8-3.1 1.7.6a7 7 0 0 1 1.9-1.1l.3-1.8h3.6l.3 1.8a7 7 0 0 1 1.9 1.1l1.7-.6 1.8 3.1-1.4 1.1a7 7 0 0 1 0 3z",
    plus: "M12 5v14M5 12h14",
    folder: "M3.5 7A1.5 1.5 0 0 1 5 5.5h4.2l2 2.2H19A1.5 1.5 0 0 1 20.5 9.2V17A1.5 1.5 0 0 1 19 18.5H5A1.5 1.5 0 0 1 3.5 17z",
    file: "M7 3.5h6.5l4.5 4.5V19a1.5 1.5 0 0 1-1.5 1.5H7A1.5 1.5 0 0 1 5.5 19V5A1.5 1.5 0 0 1 7 3.5zM13.5 3.5V8H18",
    upload: "M12 15.5V4.5M7.5 9 12 4.5 16.5 9M4.5 15v3a1.5 1.5 0 0 0 1.5 1.5h12a1.5 1.5 0 0 0 1.5-1.5v-3",
    "folder-upload":
      "M3.5 7A1.5 1.5 0 0 1 5 5.5h4.2l2 2.2H19A1.5 1.5 0 0 1 20.5 9.2V17A1.5 1.5 0 0 1 19 18.5H5A1.5 1.5 0 0 1 3.5 17zM12 16v-5.5M9.5 13 12 10.5l2.5 2.5",
    "folder-plus":
      "M3.5 7A1.5 1.5 0 0 1 5 5.5h4.2l2 2.2H19A1.5 1.5 0 0 1 20.5 9.2V17A1.5 1.5 0 0 1 19 18.5H5A1.5 1.5 0 0 1 3.5 17zM12 10.5v5.5M9.25 13.25h5.5",
    download: "M12 4.5v11M7.5 11 12 15.5 16.5 11M4.5 15v3a1.5 1.5 0 0 0 1.5 1.5h12a1.5 1.5 0 0 0 1.5-1.5v-3",
    refresh: "M19.5 12a7.5 7.5 0 1 1-2.2-5.3M19.5 4.5v4h-4",
    "arrow-up": "M12 19V5M6 11l6-6 6 6",
    "arrow-left": "M19 12H5M11 6l-6 6 6 6",
    search: "M10.5 17a6.5 6.5 0 1 0 0-13 6.5 6.5 0 0 0 0 13zM15.5 15.5 20 20",
    filter: "M4 6h16M7 12h10M10 18h4",
    sort: "M8 5v14M4.5 15.5 8 19l3.5-3.5M16 19V5M12.5 8.5 16 5l3.5 3.5",
    trash: "M4.5 7h15M9.5 7V4.5h5V7M6.5 7l.9 11.6a1.5 1.5 0 0 0 1.5 1.4h6.2a1.5 1.5 0 0 0 1.5-1.4L17.5 7",
    copy: "M9 9h9.5v11H9zM15 9V4.5H5.5V15H9",
    move: "M5 12h13M13 7l5 5-5 5",
    edit: "M14.5 5.5l4 4L9 19H5v-4zM12.5 7.5l4 4",
    star: "M12 4.5l2.3 4.7 5.2.75-3.75 3.65.9 5.15L12 16.3l-4.65 2.45.9-5.15L4.5 9.95l5.2-.75z",
    close: "M6.5 6.5l11 11M17.5 6.5l-11 11",
    more: "M6 12h.01M12 12h.01M18 12h.01",
    check: "M5 12.5 10 17.5 19 7",
    alert: "M12 4.5 21 19.5H3zM12 10v4M12 17h.01",
    info: "M12 20.5a8.5 8.5 0 1 0 0-17 8.5 8.5 0 0 0 0 17zM12 11v5.5M12 7.75h.01",
    sidebar: "M4.5 5.5h15v13h-15zM9.5 5.5v13",
    usage: "M4.5 19.5h15M7 19.5v-7M12 19.5V5.5M17 19.5v-10",
    snapshot: "M12 7.5V12l3 2M4.6 12a7.4 7.4 0 1 0 2.2-5.25M4.5 4.5v3.25h3.25",
    "chevron-right": "M9.5 6l6 6-6 6",
    "chevron-down": "M6 9.5l6 6 6-6",
    link: "M10 14a4 4 0 0 0 5.66 0l3-3a4 4 0 0 0-5.66-5.66l-1 1M14 10a4 4 0 0 0-5.66 0l-3 3a4 4 0 0 0 5.66 5.66l1-1",
    restore: "M4.5 9.5h10a5 5 0 0 1 0 10H9M8.5 5.5l-4 4 4 4",
    sun: "M12 16a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM12 2.5v2M12 19.5v2M2.5 12h2M19.5 12h2M5.3 5.3l1.4 1.4M17.3 17.3l1.4 1.4M5.3 18.7l1.4-1.4M17.3 6.7l1.4-1.4",
    moon: "M19.5 14.5A7.5 7.5 0 0 1 9.5 4.5a7.5 7.5 0 1 0 10 10z",
    monitor: "M3.5 5h17v11h-17zM8.5 20h7M12 16v4",
    key: "M14.5 9.5a4 4 0 1 1-1.2-2.8M13.3 6.7 20 13.4V16h-2.6v-2.1h-2.1v-2.1",
    lock: "M6.5 10.5h11v9h-11zM8.5 10.5V8a3.5 3.5 0 0 1 7 0v2.5",
    clock: "M12 20.5a8.5 8.5 0 1 0 0-17 8.5 8.5 0 0 0 0 17zM12 7.5V12l3 2",
    eye: "M2.5 12s3.5-6.5 9.5-6.5S21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12zM12 14.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5z",
    sync: "M4.5 12a7.5 7.5 0 0 1 12.8-5.3l2.2 2.3M19.5 12a7.5 7.5 0 0 1-12.8 5.3L4.5 15M19.5 4.5V9H15M4.5 19.5V15H9",
    feature: "M5 5h6v6H5zM13 5h6v6h-6zM5 13h6v6H5zM13 16h6M16 13v6",
    bucket: "M5 7c0-1.4 3.1-2.5 7-2.5s7 1.1 7 2.5-3.1 2.5-7 2.5S5 8.4 5 7zM5 7l1.6 11.2c.2 1.2 2.6 2.3 5.4 2.3s5.2-1.1 5.4-2.3L19 7",
    history: "M4.5 12a7.5 7.5 0 1 0 2.2-5.3L4.5 9M4.5 4.5V9H9M12 8v4.5l3 1.5",
    external: "M13.5 4.5h6v6M19.5 4.5l-8 8M17.5 13.5v5a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1v-11a1 1 0 0 1 1-1h5",
    power: "M12 3.5v8M7 6.6a7 7 0 1 0 10 0",
  } as const;

  export type IconName = keyof typeof ICONS;
</script>

<script lang="ts">
  interface Props {
    name: IconName;
    /** Pixel size; the token sizes are 14, 16 and 20. */
    size?: number;
    /** Filled shape (the star of a favourite). */
    filled?: boolean;
  }

  let { name, size = 16, filled = false }: Props = $props();
</script>

<svg
  class="icon"
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill={filled ? "currentColor" : "none"}
  stroke="currentColor"
  stroke-width="1.75"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  focusable="false"><path d={ICONS[name]} /></svg
>

<style>
  .icon {
    flex: none;
    display: inline-block;
    vertical-align: middle;
  }
</style>
