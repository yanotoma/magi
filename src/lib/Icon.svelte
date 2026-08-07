<!--
  Icons, inlined.

  The paths come from Lucide (https://lucide.dev), taken verbatim from
  `lucide-static@1.30.0` rather than typed from memory — an icon reproduced by hand
  looks approximately right and is subtly wrong at 14 pixels.

  Inlined rather than added as a dependency. Three icons do not justify a package,
  its build step, and its version to keep current; the geometry below is the whole
  of what would have been imported. If this grows past a handful, `lucide-svelte`
  becomes the better trade and this file should be replaced by it.

  Lucide is ISC licensed, which requires its notice to travel with copies of the
  work:

      ISC License

      Copyright (c) 2026 Lucide Icons and Contributors

      Permission to use, copy, modify, and/or distribute this software for any
      purpose with or without fee is hereby granted, provided that the above
      copyright notice and this permission notice appear in all copies.

      THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
      WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
      MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
      ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
      WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
      ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
      OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
-->
<script lang="ts">
  export type IconName = "pencil" | "trash" | "plus" | "chevron-down" | "chevron-right";

  let {
    name,
    size = 14,
  }: {
    name: IconName;
    /** Edge length in pixels. Lucide's grid is 24, scaled by the viewBox. */
    size?: number;
  } = $props();

  /** The inner geometry of each icon, verbatim from lucide-static. */
  const PATHS: Record<IconName, string[]> = {
    pencil: [
      "M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z",
      "m15 5 4 4",
    ],
    trash: [
      "M10 11v6",
      "M14 11v6",
      "M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6",
      "M3 6h18",
      "M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2",
    ],
    plus: ["M5 12h14", "M12 5v14"],
    "chevron-down": ["m6 9 6 6 6-6"],
    "chevron-right": ["m9 18 6-6-6-6"],
  };
</script>

<!--
  `aria-hidden` throughout, with no title. Every icon here sits inside a button that
  already has a text label or a `title`, so announcing the glyph as well would make
  a screen reader read the same action twice.
-->
<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  focusable="false"
>
  {#each PATHS[name] as d (d)}
    <path {d} />
  {/each}
</svg>

<style>
  svg {
    /* Sits on the text baseline rather than above it, so an icon beside a word does
       not push the line taller. */
    display: block;
    flex: none;
  }
</style>
